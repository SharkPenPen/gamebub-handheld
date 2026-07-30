use esp_idf_svc::hal::units::Hertz;
use std::{
    fs::File,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use thiserror::Error;

use crate::{
    device::{drivers::fpga, Device},
    kvs,
    util::BackgroundReader,
};
use crate::{ui, util::ReaderResult};

use super::Bitstream;

const SYSTEM_CLOCK_RATE: Hertz = Hertz(8 * 1024 * 1024);
const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(250);

const REG_EMU_CONFIG: u32 = 0xC000_0000;
const REG_EMU_CART_CONFIG: u32 = 0xC000_0004;
const REG_EMU_CART_ROM_ADDR: u32 = 0xC000_0008;
const REG_EMU_CART_ROM_MASK: u32 = 0xC000_000C;
const REG_EMU_CART_RAM_ADDR: u32 = 0xC000_0010;
const REG_EMU_CART_RAM_MASK: u32 = 0xC000_0014;
const REG_RTC_STATE: u32 = 0xC000_0018;
const REG_RTC_LATCHED: u32 = 0xC000_001C;
const REG_IMU_ACCEL_X: u32 = 0xC000_0020;
const REG_IMU_ACCEL_Y: u32 = 0xC000_0024;

#[derive(Debug, Error)]
pub enum GameboyError {
    #[error("unsupported cartridge type {0}")]
    UnsupportedCartridgeType(u8),
    #[error("I/O error")]
    IoError(#[from] std::io::Error),
    #[error("FPGA error")]
    FpgaError(#[from] crate::device::drivers::fpga::Error),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum MbcType {
    None = 0,
    Mbc1 = 1,
    Mbc2 = 2,
    Mbc3 = 3,
    Mbc5 = 4,
    Mbc7 = 5,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub struct RomHeader {
    mbc: MbcType,
    rom_size: u32,
    ram_size: u32,

    has_ram: bool,
    has_battery: bool,
    has_rtc: bool,
    has_rumble: bool,
    has_sensor: bool,
}

impl RomHeader {
    fn parse(header: [u8; 0x150]) -> Result<RomHeader, GameboyError> {
        macro_rules! cart_type {
            ($mbc:expr, $($field:ident),*) => {
                {
                    $(
                        $field = true;
                    )*
                    $mbc
                }
            }
        }

        let cartridge_type = header[0x147];
        let mut has_ram = false;
        let mut has_battery = false;
        let mut has_rtc = false;
        let mut has_rumble = false;
        let mut has_sensor = false;
        let mbc = match cartridge_type {
            0x00 => cart_type!(MbcType::None,),
            0x01 => cart_type!(MbcType::Mbc1,),
            0x02 => cart_type!(MbcType::Mbc1, has_ram),
            0x03 => cart_type!(MbcType::Mbc1, has_ram, has_battery),
            0x05 => cart_type!(MbcType::Mbc2, has_ram),
            0x06 => cart_type!(MbcType::Mbc2, has_ram, has_battery),
            0x08 => cart_type!(MbcType::None, has_ram),
            0x09 => cart_type!(MbcType::None, has_ram, has_battery),
            0x0F => cart_type!(MbcType::Mbc3, has_rtc, has_battery),
            0x10 => cart_type!(MbcType::Mbc3, has_rtc, has_ram, has_battery),
            0x11 => cart_type!(MbcType::Mbc3,),
            0x12 => cart_type!(MbcType::Mbc3, has_ram),
            0x13 => cart_type!(MbcType::Mbc3, has_ram, has_battery),
            0x19 => cart_type!(MbcType::Mbc5,),
            0x1A => cart_type!(MbcType::Mbc5, has_ram),
            0x1B => cart_type!(MbcType::Mbc5, has_ram, has_battery),
            0x1C => cart_type!(MbcType::Mbc5, has_rumble),
            0x1D => cart_type!(MbcType::Mbc5, has_rumble, has_ram),
            0x1E => cart_type!(MbcType::Mbc5, has_rumble, has_ram, has_battery),
            0x22 => cart_type!(MbcType::Mbc7, has_sensor, has_ram), // EEPROM, accelerometer
            _ => return Err(GameboyError::UnsupportedCartridgeType(cartridge_type)),
        };

        let rom_size = 32 * 1024 * (1 << header[0x148]);
        let ram_size = match header[0x149] {
            _ if mbc == MbcType::Mbc2 => 512,
            _ if mbc == MbcType::Mbc7 => 256,
            2 => 8 * 1024,
            3 => 32 * 1024,
            4 => 128 * 1024,
            5 => 64 * 1024,
            _ => 0,
        };

        Ok(RomHeader {
            mbc,
            rom_size,
            ram_size,
            has_ram,
            has_battery,
            has_rtc,
            has_rumble,
            has_sensor,
        })
    }

    fn as_emu_cart_config(&self) -> u32 {
        1 | ((self.mbc as u32) << 1)
            | ((self.has_ram as u32) << 4)
            | ((self.has_rtc as u32) << 5)
            | ((self.has_rumble as u32) << 6)
    }
}

/// Driver for Gameboy FPGA module
pub struct Gameboy {
    /// Rom header, if this is an emulated cartridge
    rom_header: Option<RomHeader>,
    /// Path to the RAM file, if this is an emulated cartridge.
    ram_path: Option<PathBuf>,
    /// Loaded bootrom path.
    bootrom_path: Option<&'static str>,
}

impl Gameboy {
    pub fn new() -> Self {
        Gameboy {
            rom_header: None,
            ram_path: None,
            bootrom_path: None,
        }
    }

    fn get_bootrom_path() -> &'static str {
        let is_dmg = kvs::keys::GB_IS_DMG.get().unwrap();
        let skip = kvs::keys::GB_SKIP_BOOT_ANIM.get().unwrap();

        if is_dmg {
            if skip {
                "gameboy.bios-dmg-fast.bin"
            } else {
                "gameboy.bios-dmg.bin"
            }
        } else {
            if skip {
                "gameboy.bios-cgb-fast.bin"
            } else {
                "gameboy.bios-cgb.bin"
            }
        }
    }

    fn set_config(&mut self, device: &mut Device) -> Result<(), GameboyError> {
        let config = 0 | (((!kvs::keys::GB_IS_DMG.get().unwrap()) as u32) << 0);
        device.fpga.write_u32(REG_EMU_CONFIG, config)?;
        Ok(())
    }

    fn load_bootrom(&mut self, device: &mut Device) -> Result<(), GameboyError> {
        let bios_path = Self::get_bootrom_path();
        if self.bootrom_path == Some(bios_path) {
            return Ok(());
        }

        log::info!("Loading CGB bootrom");
        let mut bios_file = crate::util::open_system_file(bios_path)?;
        let mut buf = vec![0u8; 2048].into_boxed_slice();
        bios_file.read(&mut buf)?;

        let address = 0xC010_0000;
        let command = fpga::SpiCommand {
            word_size: fpga::FpgaSpiWordSize::Bits8,
            byte_swap: true,
            increment_address: true,
        };
        // 8 bits per transfer, 2 clocks each.
        // This would be ~8 MHz. However, since it's such a short transfer, we can do a slightly
        // higher rate and let the SPI FIFO buffer it.
        let max_clock = Hertz(10_000_000);
        device
            .fpga
            .spi_write(Some(max_clock), command, address, &buf)?;

        self.bootrom_path = Some(bios_path);
        Ok(())
    }

    pub fn set_physical_cartridge(&mut self) -> Result<(), GameboyError> {
        self.ram_path = None;

        let mut device = Device::lock();

        // Hold in reset
        device.fpga.write_u32(fpga::REG_CONTROL, 0b0000)?;

        // Configure device.
        self.set_config(&mut device)?;
        self.load_bootrom(&mut device)?;

        // Switch to physical cartridge.
        device.fpga.write_u32(REG_EMU_CART_CONFIG, 0)?;

        // Disable IRQs (including vblank)
        device.fpga.write_u32(fpga::REG_IRQ_ENABLE, 0)?;

        // Resume
        device.fpga.write_u32(fpga::REG_CONTROL, 0b1011)?;
        device.imu.disable_accel().unwrap();

        Ok(())
    }

    pub fn set_emulated_cartridge(&mut self, rom_path: &Path) -> Result<(), GameboyError> {
        // Hold in reset
        {
            let mut device = Device::lock();
            device.fpga.write_u32(fpga::REG_CONTROL, 0b0000)?;
            device.imu.disable_accel().unwrap();

            // Configure device
            self.set_config(&mut device)?;
            self.load_bootrom(&mut device)?;
        }

        // Load ROM
        let mut rom_file = File::open(rom_path)?;
        let rom_file_size = rom_file.metadata()?.len() as u32;
        let mut rom_header = [0u8; 0x150];
        rom_file.read(&mut rom_header)?;
        let rom_header = RomHeader::parse(rom_header)?;
        rom_file.seek(std::io::SeekFrom::Start(0))?;
        log::info!("Loading rom: {:?}", rom_header);

        const CHUNK_SIZE: usize = 32 * 1024;
        let mut last_progress_update = Instant::now();
        let mut reader = BackgroundReader::new(rom_file, CHUNK_SIZE);
        let mut total = 0u32;
        loop {
            let chunk = match reader.get() {
                ReaderResult::Ok(buf) => buf,
                ReaderResult::Eof => break,
                ReaderResult::Err(err) => Err(err)?,
            };

            Device::lock().fpga.sdram_write(total, &chunk)?;
            total += chunk.len() as u32;

            // Update UI progress bar.
            if last_progress_update.elapsed() > PROGRESS_UPDATE_INTERVAL {
                let progress = (total as f32) / (rom_file_size as f32);
                ui::send(ui::Message::RomLoadingProgress(progress));
                last_progress_update = Instant::now();
            }
        }
        ui::send(ui::Message::RomLoadingProgress(1.0));

        // Load RAM
        let ram_path = rom_path.with_extension("sav");
        match File::open(ram_path.as_path()) {
            Ok(mut ram_file) => {
                log::info!("Loading RAM");
                let mut buf = vec![0; CHUNK_SIZE];

                let mut pos = 0u32;
                while pos < rom_header.ram_size {
                    let to_read = ((rom_header.ram_size - pos) as usize).min(CHUNK_SIZE);
                    let n = ram_file.read(&mut buf[..to_read])?;
                    if n == 0 {
                        break;
                    }
                    Device::lock().fpga.sram_write(pos, &buf[..n])?;
                    pos += n as u32;
                }

                if rom_header.has_rtc {
                    // Read next 48 bytes for RTC data.
                    let n = ram_file.read(&mut buf[..48])?;
                    if n == 48 {
                        let mut rtc_state = RtcState::from_disk(&buf[0..20].try_into().unwrap());
                        let rtc_latched = RtcState::from_disk(&buf[20..40].try_into().unwrap());
                        let rtc_timestamp = u64::from_le_bytes(buf[40..48].try_into().unwrap());
                        let mut device = Device::lock();
                        let elapsed = device
                            .get_datetime()
                            .unix_timestamp()
                            .saturating_sub_unsigned(rtc_timestamp);
                        rtc_state.advance(elapsed as u64);
                        device.fpga.write_u32(REG_RTC_STATE, rtc_state.to_fpga())?;
                        device
                            .fpga
                            .write_u32(REG_RTC_LATCHED, rtc_latched.to_fpga())?;
                        log::info!(
                            "Loaded saved RTC state: {:?}, elapsed={}",
                            rtc_state,
                            elapsed
                        );
                    }
                }
            }
            Err(_) => {
                log::info!("Not loading RAM");
            }
        }

        let mut device = Device::lock();

        // Configure emulated cartridge control registers
        device
            .fpga
            .write_u32(REG_EMU_CART_CONFIG, rom_header.as_emu_cart_config())?;
        device.fpga.write_u32(REG_EMU_CART_ROM_ADDR, 0)?;
        device
            .fpga
            .write_u32(REG_EMU_CART_ROM_MASK, rom_header.rom_size - 1)?;
        device.fpga.write_u32(REG_EMU_CART_RAM_ADDR, 0)?;
        device
            .fpga
            .write_u32(REG_EMU_CART_RAM_MASK, rom_header.ram_size - 1)?;

        // If IMU is needed, enable vsync IRQ
        let irq_mask = if rom_header.has_sensor {
            // XXX: if other components need IMU too, switch to a global lease system
            device.imu.enable_accel().unwrap();
            0b1
        } else {
            0b0
        };
        device.fpga.write_u32(fpga::REG_IRQ_ENABLE, irq_mask)?;

        // Resume
        device.fpga.write_u32(fpga::REG_CONTROL, 0b1011)?;

        self.ram_path = Some(ram_path);
        self.rom_header = Some(rom_header);
        Ok(())
    }

    /// Persists the game save RAM to disk, if using an emulated cartridge.
    pub fn persist_ram(&mut self) -> Result<(), GameboyError> {
        let ram_path = match self.ram_path.as_ref() {
            Some(ram_path) => ram_path,
            None => return Ok(()),
        };

        let ram_size = self.rom_header.as_ref().map_or(0, |h| h.ram_size);
        log::info!("Saving RAM: {}", ram_path.display());

        let mut file = File::create(ram_path)?;
        const CHUNK_SIZE: usize = 8 * 1024;
        let mut buf = vec![0; CHUNK_SIZE].into_boxed_slice();
        let mut address: u32 = 0;
        let mut bytes_left = ram_size as usize;

        let mut device = Device::lock();
        while bytes_left > 0 {
            let to_read = CHUNK_SIZE.min(bytes_left);
            let data = &mut buf[0..to_read];
            device.fpga.sram_read(address, data)?;
            file.write(data)?;
            address += to_read as u32;
            bytes_left -= to_read;
        }

        if self.rom_header.as_ref().map_or(false, |h| h.has_rtc) {
            let rtc_state = RtcState::from_fpga(device.fpga.read_u32(REG_RTC_STATE)?);
            let rtc_latched = RtcState::from_fpga(device.fpga.read_u32(REG_RTC_LATCHED)?);
            file.write(&rtc_state.to_disk())?;
            file.write(&rtc_latched.to_disk())?;
            file.write(&(device.get_datetime().unix_timestamp() as u64).to_le_bytes())?;
            log::info!("Wrote RTC state: {:?}", rtc_state);
        }

        Ok(())
    }

    /// Return whether the current save game would need to be persisted to disk.
    pub fn needs_save_persist(&self) -> bool {
        self.ram_path.is_some()
    }
}

impl Bitstream for Gameboy {
    fn get_bitstream_path(&self) -> &'static str {
        return "gameboy.bit.gz";
    }

    fn on_after_program(&mut self) -> Result<(), String> {
        Device::lock().fpga.set_system_clock_rate(SYSTEM_CLOCK_RATE);
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), fpga::Error> {
        let mut device = Device::lock();

        // Enable/disable IMU as needed
        if !paused && self.rom_header.as_ref().map_or(false, |h| h.has_sensor) {
            device.imu.enable_accel().unwrap();
        } else {
            device.imu.disable_accel().unwrap();
        }

        device
            .fpga
            .write_u32(fpga::REG_CONTROL, 0b1010u32 | ((!paused) as u32))
    }

    fn reset(&mut self) -> Result<(), fpga::Error> {
        let mut device = Device::lock();
        device.fpga.write_u32(fpga::REG_CONTROL, 0b0000)?;
        device.fpga.write_u32(fpga::REG_CONTROL, 0b1010)?;
        Ok(())
    }

    fn on_vblank_irq(&mut self) {
        let mut device = Device::lock();
        let sample = device.imu.read_accel().unwrap();
        // Invert X and Y
        let accel_x = ((0x81D0 as f32) + ((0x70 as f32) * -sample.x)) as u16;
        let accel_y = ((0x81D0 as f32) + ((0x70 as f32) * -sample.y)) as u16;
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

#[derive(Copy, Clone, Debug)]
struct RtcState {
    seconds: u8,
    minutes: u8,
    hours: u8,
    days: u16,
    days_overflow: bool,
    halt: bool,
}

impl RtcState {
    pub fn from_fpga(data: u32) -> Self {
        // FPGA format: hodddddddddhhhhhmmmmmmssssss
        RtcState {
            seconds: ((data >> 0) & 0b111111) as u8,
            minutes: ((data >> 6) & 0b111111) as u8,
            hours: ((data >> 12) & 0b11111) as u8,
            days: ((data >> 17) & 0b111111111) as u16,
            days_overflow: ((data >> 26) & 1) == 1,
            halt: ((data >> 27) & 1) == 1,
        }
    }

    pub fn to_fpga(&self) -> u32 {
        (((self.seconds as u32) & 0b111111) << 0)
            | (((self.minutes as u32) & 0b111111) << 6)
            | (((self.hours as u32) & 0b11111) << 12)
            | (((self.days as u32) & 0b111111111) << 17)
            | ((self.days_overflow as u32) << 26)
            | ((self.halt as u32) << 27)
    }

    pub fn from_disk(data: &[u8; 20]) -> Self {
        let words = [
            u32::from_le_bytes(data[0..4].try_into().unwrap()),
            u32::from_le_bytes(data[4..8].try_into().unwrap()),
            u32::from_le_bytes(data[8..12].try_into().unwrap()),
            u32::from_le_bytes(data[12..16].try_into().unwrap()),
            u32::from_le_bytes(data[16..20].try_into().unwrap()),
        ];
        RtcState {
            seconds: (words[0] & 0b111111) as u8,
            minutes: (words[1] & 0b111111) as u8,
            hours: (words[2] & 0b11111) as u8,
            days: ((words[3] & 0xFF) | ((words[4] & 1) << 8)) as u16,
            halt: ((words[4] >> 6) & 1) == 1,
            days_overflow: ((words[4] >> 7) & 1) == 1,
        }
    }

    pub fn to_disk(&self) -> [u8; 20] {
        let mut data = [0u8; 20];
        data[0..4].copy_from_slice(&u32::to_le_bytes(self.seconds as u32));
        data[4..8].copy_from_slice(&u32::to_le_bytes(self.minutes as u32));
        data[8..12].copy_from_slice(&u32::to_le_bytes(self.hours as u32));
        data[12..16].copy_from_slice(&u32::to_le_bytes((self.days & 0xFF) as u32));
        let last = ((self.days as u32 & 0x100) >> 8)
            | ((self.halt as u32) << 6)
            | ((self.days_overflow as u32) << 7);
        data[16..20].copy_from_slice(&u32::to_le_bytes(last));
        data
    }

    fn compute_ticks(value: u16, ticks: &mut u64, wrap_point: u16, max_value: u16) -> u64 {
        let mut value = value as u64;
        if value >= (wrap_point as u64) {
            let needed = (max_value as u64) - value;
            if *ticks >= needed {
                *ticks -= needed;
                value = 0;
            } else {
                value += *ticks;
                *ticks = 0;
            }
        }
        value += *ticks;
        *ticks = value / (wrap_point as u64);
        value % (wrap_point as u64)
    }

    pub fn advance(&mut self, seconds: u64) {
        let mut ticks = seconds;
        self.seconds = Self::compute_ticks(self.seconds as u16, &mut ticks, 60, 1 << 6) as u8;
        self.minutes = Self::compute_ticks(self.minutes as u16, &mut ticks, 60, 1 << 6) as u8;
        self.hours = Self::compute_ticks(self.hours as u16, &mut ticks, 24, 1 << 5) as u8;
        self.days = Self::compute_ticks(self.days as u16, &mut ticks, 1 << 9, 1 << 9) as u16;
        if ticks > 0 {
            self.days_overflow = true;
        }
    }
}
