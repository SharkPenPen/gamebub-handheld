use std::{
    fmt::{Debug, Display},
    fs::File,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use esp_idf_svc::hal::units::Hertz;
use rtc::RtcState;
use thiserror::Error;

use crate::{
    device::{drivers::fpga, Device},
    kvs,
    util::BackgroundReader,
};
use crate::{ui, util::ReaderResult};

use super::Bitstream;
use save_type_detector::SaveTypeDetector;

mod game_db;
mod rtc;
mod save_type_detector;

const SYSTEM_CLOCK_RATE: Hertz = Hertz(16 * 1024 * 1024);
const ROM_HEADER_LENGTH: usize = 192;
const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(250);

const REG_EMU_CART_CONFIG: u32 = 0xC000_0000;
const REG_EMU_CART_ROM_SIZE: u32 = 0xC000_0004;
const REG_GB_PLAYER: u32 = 0xC000_0008;
const REG_IMU_GYRO_Z: u32 = 0xC000_0100;
const REG_IMU_ACCEL_X: u32 = 0xC000_0104;
const REG_IMU_ACCEL_Y: u32 = 0xC000_0108;
const REG_RTC_LO: u32 = 0xC000_0200;
const REG_RTC_HI: u32 = 0xC000_0204;
const REG_STAT_STALLS: u32 = 0xC000_1000;
const REG_STAT_CYCLES: u32 = 0xC000_1004;

#[derive(Debug, Error)]
pub enum GbaError {
    #[error("I/O error")]
    IoError(#[from] std::io::Error),
    #[error("FPGA error")]
    FpgaError(#[from] crate::device::drivers::fpga::Error),
}

#[allow(unused)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
enum SaveType {
    /// No backup
    #[default]
    None,
    /// EEPROM - Autodetect Size
    EepromAuto,
    /// EEPROM, 512B
    Eeprom512,
    /// EEPROM, 8KiB
    Eeprom8K,
    /// SRAM or FRAM, 32 KiB
    Sram,
    /// Flash 64KiB
    Flash64K,
    /// Flash 128KiB
    Flash128K,
}

impl SaveType {
    fn get_size(self) -> usize {
        match self {
            Self::None => 0,
            Self::EepromAuto | Self::Eeprom8K => 8 * 1024,
            Self::Eeprom512 => 512,
            Self::Sram => 32 * 1024,
            Self::Flash64K => 64 * 1024,
            Self::Flash128K => 128 * 1024,
        }
    }
}

#[derive(Copy, Clone)]
struct EmulatedCartridgeConfig {
    pub save_type: SaveType,
    pub has_rumble: bool,
    pub has_rtc: bool,
    pub has_accel: bool,
    pub has_gyro: bool,
}

impl EmulatedCartridgeConfig {
    pub const DISABLED: u32 = 0;

    const fn from_save_type(save_type: SaveType) -> Self {
        EmulatedCartridgeConfig {
            save_type,
            has_rumble: false,
            has_rtc: false,
            has_accel: false,
            has_gyro: false,
        }
    }

    fn as_config_u32(self) -> u32 {
        let backup: u32 = match self.save_type {
            SaveType::None => 0b0000,
            SaveType::Sram => 0b0001,
            SaveType::Flash64K => 0b0010,
            SaveType::Flash128K => 0b0110,
            SaveType::EepromAuto => 0b1011,
            SaveType::Eeprom512 => 0b0011,
            SaveType::Eeprom8K => 0b0111,
        };
        let has_gpio = self.has_rumble || self.has_rtc || self.has_gyro;
        1 | (backup << 1)
            | ((has_gpio as u32) << 5)
            | ((self.has_rumble as u32) << 6)
            | ((self.has_rtc as u32) << 7)
            | ((self.has_accel as u32) << 8)
            | ((self.has_gyro as u32) << 9)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RomHeader {
    game_title: [u8; 12],
    game_code: [u8; 4],
}

impl RomHeader {
    fn parse(header: [u8; ROM_HEADER_LENGTH]) -> RomHeader {
        RomHeader {
            game_title: header[0xA0..0xAC].try_into().unwrap(),
            game_code: header[0xAC..0xB0].try_into().unwrap(),
        }
    }
}

impl Display for RomHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let title = String::from_utf8_lossy(&self.game_title);
        let code = String::from_utf8_lossy(&self.game_code);
        write!(f, "\'{}\' ({})", title, code)
    }
}

/// Driver for GBA FPGA module
pub struct Gba {
    /// Path to the save file, if this is an emulated cartridge.
    save_path: Option<PathBuf>,
    /// Size of the save, if this is an emulated cartridge.
    save_size: u32,
    /// Emulated cartridge config.
    emu_cart_config: Option<EmulatedCartridgeConfig>,
    /// Loaded bios path
    bios_path: Option<&'static str>,
}

impl Gba {
    pub fn new() -> Self {
        Gba {
            save_path: None,
            save_size: 0,
            emu_cart_config: None,
            bios_path: None,
        }
    }

    fn get_bios_path() -> &'static str {
        if kvs::keys::GBA_SKIP_BOOT_ANIM.get().unwrap() {
            "gba.bios-fast.bin"
        } else {
            "gba.bios.bin"
        }
    }

    fn load_bios(&mut self, device: &mut Device) -> Result<(), GbaError> {
        let bios_path = Self::get_bios_path();
        if self.bios_path == Some(bios_path) {
            return Ok(());
        }

        let mut bios_file = crate::util::open_system_file(bios_path)?;
        let mut buf = vec![0u8; 16 * 1024].into_boxed_slice();
        bios_file.read(&mut buf)?;

        let address = 0xC010_0000;
        let command = fpga::SpiCommand {
            word_size: fpga::FpgaSpiWordSize::Bits32,
            byte_swap: true,
            increment_address: true,
        };
        // 32 bits per transfer, 2 clocks each.
        let max_clock = SYSTEM_CLOCK_RATE * 32 / (4 * 2);
        device
            .fpga
            .spi_write(Some(max_clock), command, address, &buf)?;

        self.bios_path = Some(bios_path);
        Ok(())
    }

    pub fn set_physical_cartridge(&mut self) -> Result<(), GbaError> {
        let mut device = Device::lock();

        // Hold in reset
        device.fpga.write_u32(fpga::REG_CONTROL, 0b0000)?;

        // Load bios if needed
        self.load_bios(&mut device)?;

        device.fpga.write_u32(
            REG_GB_PLAYER,
            kvs::keys::GBA_ENABLE_GBP.get().unwrap() as u32,
        )?;

        // Switch to physical cartridge.
        device
            .fpga
            .write_u32(REG_EMU_CART_CONFIG, EmulatedCartridgeConfig::DISABLED)?;

        // Disable IRQs (including vblank)
        device.fpga.write_u32(fpga::REG_IRQ_ENABLE, 0)?;

        // Resume
        device.fpga.write_u32(fpga::REG_CONTROL, 0b1011)?;
        device.imu.disable_gyro().unwrap();
        device.imu.disable_accel().unwrap();

        self.save_path = None;
        self.emu_cart_config = None;
        Ok(())
    }

    pub fn set_emulated_cartridge(&mut self, rom_path: &Path) -> Result<(), GbaError> {
        // Hold in reset
        {
            let mut device = Device::lock();
            device.fpga.write_u32(fpga::REG_CONTROL, 0b0000)?;
            device.imu.disable_gyro().unwrap();
            device.imu.disable_accel().unwrap();

            // Load bios if needed
            self.load_bios(&mut device)?;

            device.fpga.write_u32(
                REG_GB_PLAYER,
                kvs::keys::GBA_ENABLE_GBP.get().unwrap() as u32,
            )?;
        }

        // Load ROM
        let mut rom_file = File::open(rom_path)?;
        let rom_file_size = rom_file.metadata()?.len() as u32;
        let mut rom_header = [0u8; ROM_HEADER_LENGTH];
        rom_file.read(&mut rom_header)?;
        rom_file.seek(std::io::SeekFrom::Start(0))?;
        let rom_header = RomHeader::parse(rom_header);
        log::info!("Loading rom: {}", rom_header);

        let mut save_type_detector = SaveTypeDetector::new();
        let emu_cart_config = game_db::lookup(&rom_header.game_code);

        const CHUNK_SIZE: usize = 32 * 1024;
        let mut reader = BackgroundReader::new(rom_file, CHUNK_SIZE);

        let mut total = 0u32;
        let mut last_progress_update = Instant::now();
        let start_time = Instant::now();
        let mut transfer_duration = Duration::ZERO;
        let mut detect_duration = Duration::ZERO;
        loop {
            let chunk = match reader.get() {
                ReaderResult::Ok(buf) => buf,
                ReaderResult::Eof => break,
                ReaderResult::Err(err) => Err(err)?,
            };

            let transfer_start = Instant::now();
            Device::lock().fpga.sdram_write(total, &chunk)?;
            total += chunk.len() as u32;
            transfer_duration += transfer_start.elapsed();

            // Update UI progress bar.
            if last_progress_update.elapsed() > PROGRESS_UPDATE_INTERVAL {
                let progress = (total as f32) / (rom_file_size as f32);
                ui::send(ui::Message::RomLoadingProgress(progress));
                last_progress_update = Instant::now();
            }

            let detect_start = Instant::now();
            if emu_cart_config.is_none() {
                save_type_detector.process(&chunk);
            }
            detect_duration += detect_start.elapsed();
        }
        ui::send(ui::Message::RomLoadingProgress(1.0));
        let duration = start_time.elapsed();
        log::info!(
            "Loaded ROM: {} bytes in {} ms ({}/{} ms transfer/detect)",
            total,
            duration.as_millis(),
            transfer_duration.as_millis(),
            detect_duration.as_millis(),
        );
        let rom_size = total;
        // TODO clear up to the next power of two

        match emu_cart_config {
            Some(config) => log::info!("Using save config: {:?}", config.save_type),
            None => log::info!("Detected save type: {:?}", save_type_detector.get()),
        }
        let emu_cart_config = emu_cart_config
            .unwrap_or_else(|| EmulatedCartridgeConfig::from_save_type(save_type_detector.get()));

        // Load save
        let save_type = emu_cart_config.save_type;
        let save_size = save_type.get_size() as u32;
        let save_path = rom_path.with_extension("sav");
        let mut rtc_state: Option<RtcState> = None;
        let mut buf = vec![0; CHUNK_SIZE];
        if let Ok(mut save_file) = File::open(save_path.as_path()) {
            log::info!("Loading save file");

            let mut pos = 0u32;
            while pos < save_size {
                let to_read = ((save_size - pos) as usize).min(CHUNK_SIZE);
                let n = save_file.read(&mut buf[..to_read])?;
                if n == 0 {
                    break;
                }
                Device::lock().fpga.sram_write(pos, &buf[..n])?;
                pos += n as u32;
            }

            if emu_cart_config.has_rtc {
                // Read next 16 bytes for RTC data.
                let n = save_file.read(&mut buf[..16])?;
                if n == 16 {
                    let prev_state = RtcState::from_disk(buf[0..8].try_into().unwrap());
                    let rtc_timestamp = u64::from_le_bytes(buf[8..16].try_into().unwrap());
                    let elapsed = Device::lock()
                        .get_datetime()
                        .unix_timestamp()
                        .saturating_sub_unsigned(rtc_timestamp);

                    match prev_state.to_offset_date_time() {
                        Ok(time) => {
                            let datetime = time.saturating_add(time::Duration::seconds(elapsed));
                            let new_state = RtcState::from_offset_date_time(datetime);
                            log::info!(
                                "Loaded saved RTC state: {:?}, elapsed={}",
                                new_state,
                                elapsed
                            );
                            rtc_state = Some(new_state);
                        }
                        Err(_) => {
                            log::warn!("Saved RTC state invalid");
                        }
                    }
                }
            }
        } else {
            // No save file to load, clear the save region.
            log::info!("No save file to load");
            buf.fill(0xFF);
            let mut pos = 0u32;
            while pos < save_size {
                let n = ((save_size - pos) as usize).min(CHUNK_SIZE);
                Device::lock().fpga.sram_write(pos, &buf[..n])?;
                pos += n as u32;
            }
        }

        // Time-intensive ROM loading is complete, so re-acquire Device lock.
        let mut device = Device::lock();

        // Configure emulated cartridge control registers
        device
            .fpga
            .write_u32(REG_EMU_CART_CONFIG, emu_cart_config.as_config_u32())?;
        device.fpga.write_u32(REG_EMU_CART_ROM_SIZE, rom_size - 1)?;

        // Update RTC state
        let rtc_state = match rtc_state {
            Some(state) => state,
            None => {
                let datetime = device.get_datetime();
                RtcState::from_offset_date_time(datetime)
            }
        };
        let (rtc_lo, rtc_hi) = rtc_state.to_fpga();
        device.fpga.write_u32(REG_RTC_LO, rtc_lo)?;
        device.fpga.write_u32(REG_RTC_HI, rtc_hi)?;

        // If IMU is needed, enable vsync IRQ
        let mut irq_mask = 0b0;
        if emu_cart_config.has_gyro {
            device.imu.enable_gyro().unwrap();
            irq_mask |= 0b1;
        }
        if emu_cart_config.has_accel {
            device.imu.enable_accel().unwrap();
            irq_mask |= 0b1;
        }
        device.fpga.write_u32(fpga::REG_IRQ_ENABLE, irq_mask)?;

        // Resume
        device.fpga.write_u32(fpga::REG_CONTROL, 0b1011)?;

        self.save_path = Some(save_path);
        self.save_size = save_size;
        self.emu_cart_config = Some(emu_cart_config);
        Ok(())
    }

    pub fn persist_save(&mut self) -> Result<(), GbaError> {
        let save_path = match self.save_path.as_ref() {
            Some(save_path) => save_path,
            None => return Ok(()),
        };

        log::info!("Saving to: {}", save_path.display());

        let mut file = File::create(save_path)?;
        const CHUNK_SIZE: usize = 8 * 1024;
        let mut buf = vec![0; CHUNK_SIZE].into_boxed_slice();
        let mut address: u32 = 0;
        let mut bytes_left = self.save_size as usize;

        let mut device = Device::lock();
        while bytes_left > 0 {
            let to_read = CHUNK_SIZE.min(bytes_left);
            let data = &mut buf[0..to_read];
            device.fpga.sram_read(address, data)?;
            file.write(data)?;
            address += to_read as u32;
            bytes_left -= to_read;
        }

        if self.emu_cart_config.as_ref().map_or(false, |e| e.has_rtc) {
            let rtc_lo = device.fpga.read_u32(REG_RTC_LO)?;
            let rtc_hi = device.fpga.read_u32(REG_RTC_HI)?;
            let rtc_state = RtcState::from_fpga(rtc_lo, rtc_hi);
            file.write(&rtc_state.to_disk())?;
            file.write(&(device.get_datetime().unix_timestamp() as u64).to_le_bytes())?;
            log::info!("Wrote RTC state: {:?}", rtc_state);
        }

        Ok(())
    }

    /// Return whether the current save game would need to be persisted to disk.
    pub fn needs_save_persist(&self) -> bool {
        self.save_path.is_some()
    }
}

impl Bitstream for Gba {
    fn get_bitstream_path(&self) -> &'static str {
        return "gba.bit.gz";
    }

    fn on_after_program(&mut self) -> Result<(), String> {
        Device::lock().fpga.set_system_clock_rate(SYSTEM_CLOCK_RATE);
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), fpga::Error> {
        let mut device = Device::lock();

        // Enable/disable IMU as needed
        if !paused && self.emu_cart_config.as_ref().map_or(false, |h| h.has_gyro) {
            device.imu.enable_gyro().unwrap();
        } else {
            device.imu.disable_gyro().unwrap();
        }
        if !paused && self.emu_cart_config.as_ref().map_or(false, |h| h.has_accel) {
            device.imu.enable_accel().unwrap();
        } else {
            device.imu.disable_accel().unwrap();
        }

        device
            .fpga
            .write_u32(fpga::REG_CONTROL, 0b1010u32 | ((!paused) as u32))?;

        if paused {
            // Debug output stall stats
            let num_cycles = device.fpga.read_u32(REG_STAT_CYCLES)?;
            let num_stalls = device.fpga.read_u32(REG_STAT_STALLS)?;
            device.fpga.write_u32(REG_STAT_CYCLES, 0)?;
            device.fpga.write_u32(REG_STAT_STALLS, 0)?;
            let rate = (num_cycles as f32) / ((num_cycles as f32) + (num_stalls as f32));
            log::info!("Run rate: {}%", rate * 100.0);
        }

        Ok(())
    }

    fn reset(&mut self) -> Result<(), fpga::Error> {
        let mut device = Device::lock();
        device.fpga.write_u32(fpga::REG_CONTROL, 0b0000)?;
        device.fpga.write_u32(fpga::REG_CONTROL, 0b1010)?;
        Ok(())
    }

    fn on_vblank_irq(&mut self) {
        let mut device = Device::lock();

        // Read IMU
        let has_gyro = self.emu_cart_config.as_ref().map_or(false, |h| h.has_gyro);
        if has_gyro {
            let gyro_sample = device.imu.read_gyro().unwrap();
            let gyro_z = ((0x700 as f32) - gyro_sample.z) as u16;
            device
                .fpga
                .write_u32(REG_IMU_GYRO_Z, gyro_z as u32)
                .unwrap();
        }
        let has_accel = self.emu_cart_config.as_ref().map_or(false, |h| h.has_accel);
        if has_accel {
            let accel_sample = device.imu.read_accel().unwrap();
            // TODO: determine X and Y inversion
            let accel_x = ((0x3A0 as f32) + ((0x1D0 as f32) * -accel_sample.x)) as u16;
            let accel_y = ((0x3A0 as f32) + ((0x1D0 as f32) * -accel_sample.y)) as u16;
            device
                .fpga
                .write_u32(REG_IMU_ACCEL_X, accel_x as u32)
                .unwrap();
            device
                .fpga
                .write_u32(REG_IMU_ACCEL_Y, accel_y as u32)
                .unwrap();
        }
    }
}
