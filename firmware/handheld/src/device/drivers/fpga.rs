#![allow(dead_code)]

use std::{
    io::Read,
    time::{Duration, Instant},
};

use embedded_hal::{
    digital::{InputPin, OutputPin},
    spi::SpiDevice,
};
use esp_idf_svc::hal::{
    spi::{config::LineWidth, Operation, SpiDriver, SpiSharedDeviceDriver, SpiSoftCsDeviceDriver},
    units::Hertz,
};
use thiserror::Error;

use crate::device::DisplayMode;

pub const REG_CONTROL: u32 = 0x0000_0000;
pub const REG_BUTTON: u32 = 0x0000_0004;
pub const REG_DISPLAY: u32 = 0x0000_0008;
pub const REG_IRQ_ENABLE: u32 = 0x0000_000C;
pub const REG_IRQ_STATUS: u32 = 0x0000_0010;
pub const REG_STATUS: u32 = 0x0000_0014;
pub const REG_OVERLAY_XCTRL: u32 = 0x0000_0100;
pub const REG_OVERLAY_YCTRL: u32 = 0x0000_0104;
pub const REG_FB_DIM: u32 = 0x0000_0200;

pub const MAX_SPI_READ_CLOCK: Hertz = Hertz(16_000_000);

pub type SpiDataDriver<'a> =
    SpiSoftCsDeviceDriver<'a, SpiSharedDeviceDriver<'a, &'a SpiDriver<'a>>, &'a SpiDriver<'a>>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("gpio error")]
    PinError,
    #[error("error programming fpga")]
    ProgramError,
    #[error("error reading bitstream")]
    BitstreamError,
    #[error("spi error")]
    SpiError,
}

pub struct Fpga<
    'a,
    PinDone: InputPin,
    PinProgramB: OutputPin,
    PinInitB: InputPin,
    ProgramSpi: SpiDevice,
> {
    pin_done: PinDone,
    pin_program_b: PinProgramB,
    pin_init_b: PinInitB,
    data_spi: Vec<(SpiDataDriver<'a>, Hertz)>,
    program_spi: ProgramSpi,
    system_clock: Hertz,
}

impl<'a, PinDone, PinProgramB, PinInitB, ProgramSpi>
    Fpga<'a, PinDone, PinProgramB, PinInitB, ProgramSpi>
where
    PinDone: InputPin,
    PinProgramB: OutputPin,
    PinInitB: InputPin,
    ProgramSpi: SpiDevice,
{
    pub fn new(
        pin_done: PinDone,
        pin_program_b: PinProgramB,
        pin_init_b: PinInitB,
        data_spi: Vec<(SpiDataDriver<'a>, Hertz)>,
        program_spi: ProgramSpi,
    ) -> Self {
        Fpga {
            pin_done,
            pin_program_b,
            pin_init_b,
            data_spi,
            program_spi,
            system_clock: Hertz(8 * 1024 * 1024),
        }
    }

    pub fn program(&mut self, bitstream: &mut dyn Read) -> Result<(), Error> {
        let start_time = Instant::now();
        while self.pin_init_b.is_low().map_err(|_| Error::PinError)? {
            if start_time.elapsed() > Duration::from_millis(35) {
                return Err(Error::ProgramError);
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        self.pin_program_b.set_low().map_err(|_| Error::PinError)?;
        std::thread::sleep(Duration::from_millis(1));
        if self.pin_init_b.is_high().map_err(|_| Error::PinError)? {
            return Err(Error::ProgramError);
        }
        self.pin_program_b.set_high().map_err(|_| Error::PinError)?;

        std::thread::sleep(Duration::from_millis(5));
        if self.pin_init_b.is_low().map_err(|_| Error::PinError)? {
            return Err(Error::ProgramError);
        }

        log::info!("FPGA is in program mode");

        let mut bitstream_header = [0u8; 129];
        bitstream
            .read(&mut bitstream_header)
            .map_err(|_| Error::BitstreamError)?;

        const CHUNK_SIZE: usize = 16 * 1024;
        let mut buf = vec![0; CHUNK_SIZE].into_boxed_slice();
        loop {
            let n = bitstream
                .read(&mut buf)
                .map_err(|_| Error::BitstreamError)?;
            if n == 0 {
                break;
            }
            self.program_spi
                .write(&buf[..n])
                .map_err(|_| Error::ProgramError)?;
        }

        log::info!(
            "Programmed FPGA, done={}",
            self.pin_done.is_high().map_err(|_| Error::PinError)?
        );

        Ok(())
    }

    // ─── 以下为自检新增的公开方法 ───

    /// 读取 INIT_B 引脚电平（高 → FPGA 配置就绪）
    pub fn get_init_b(&mut self) -> Result<bool, Error> {
        self.pin_init_b.is_high().map_err(|_| Error::PinError)
    }

    /// 读取 DONE 引脚电平（高 → 配置成功）
    pub fn get_done(&mut self) -> Result<bool, Error> {
        self.pin_done.is_high().map_err(|_| Error::PinError)
    }

    /// 控制 PROGRAM_B 引脚电平（false = 拉低，触发重配置；true = 释放）
    pub fn set_program_b(&mut self, level: bool) -> Result<(), Error> {
        if level {
            self.pin_program_b.set_high().map_err(|_| Error::PinError)
        } else {
            self.pin_program_b.set_low().map_err(|_| Error::PinError)
        }
    }

    // ─── 原有其余代码保持不变（以下所有方法均保留原样） ───

    pub fn set_system_clock_rate(&mut self, rate: Hertz) {
        self.system_clock = rate;
    }

    fn spi_transaction(
        &mut self,
        max_clock: Option<Hertz>,
        operations: &mut [Operation],
    ) -> Result<(), Error> {
        let driver = &mut self.data_spi.iter_mut().find(|(_, clock)| match max_clock {
            Some(max_clock) => *clock <= max_clock,
            None => true,
        });
        let driver = match driver {
            Some(driver) => driver,
            None => panic!("No suitable spi for max clock {:?}", max_clock),
        };
        driver
            .0
            .transaction(operations)
            .map_err(|_| Error::SpiError)
    }

    const fn spi_command(
        read: bool,
        word_size: FpgaSpiWordSize,
        byte_swap: bool,
        auto_increment: bool,
    ) -> u8 {
        (read as u8)
            | ((word_size as u8) << 1)
            | ((byte_swap as u8) << 3)
            | ((auto_increment as u8) << 4)
    }

    pub fn spi_write(
        &mut self,
        max_clock: Option<Hertz>,
        command: SpiCommand,
        address: u32,
        data: &[u8],
    ) -> Result<(), Error> {
        let width = LineWidth::Quad;
        let mut command = command.as_write_command();
        command |= (width as u8) << 5;
        let address = address.to_be_bytes();
        self.spi_transaction(
            max_clock,
            &mut [
                Operation::Write(&[command]),
                Operation::WriteWithWidth(&address, width),
                Operation::WriteWithWidth(&data, width),
            ],
        )
    }

    pub fn spi_read(
        &mut self,
        max_clock: Option<Hertz>,
        command: SpiCommand,
        address: u32,
        buffer: &mut [u8],
    ) -> Result<(), Error> {
        let width = LineWidth::Quad;
        let mut command = command.as_read_command();
        command |= (width as u8) << 5;
        let address = address.to_be_bytes();
        const DUMMY_BYTES: usize = 8;
        let mut dummy = [0u8; DUMMY_BYTES];
        self.spi_transaction(
            max_clock,
            &mut [
                Operation::Write(&[command]),
                Operation::WriteWithWidth(&address, width),
                Operation::ReadWithWidth(&mut dummy, width),
                Operation::ReadWithWidth(buffer, width),
            ],
        )
    }

    pub fn write_u32(&mut self, address: u32, data: u32) -> Result<(), Error> {
        let command = SpiCommand {
            word_size: FpgaSpiWordSize::Bits32,
            byte_swap: false,
            increment_address: true,
        };
        let data = data.to_be_bytes();
        self.spi_write(None, command, address, &data)
    }

    pub fn read_u32(&mut self, address: u32) -> Result<u32, Error> {
        let mut data = [0u8; 4];
        let command = SpiCommand {
            word_size: FpgaSpiWordSize::Bits32,
            byte_swap: false,
            increment_address: true,
        };
        self.spi_read(Some(MAX_SPI_READ_CLOCK), command, address, &mut data)?;
        Ok(u32::from_be_bytes(data))
    }

    pub fn sram_write(&mut self, address: u32, data: &[u8]) -> Result<(), Error> {
        let address = 0x1000_0000 | address;
        let command = SpiCommand {
            word_size: FpgaSpiWordSize::Bits16,
            byte_swap: true,
            increment_address: true,
        };
        let max_clock = (self.system_clock.0 * 16) / (4 * 3);
        self.spi_write(Some(Hertz(max_clock)), command, address, data)
    }

    pub fn sram_read(&mut self, address: u32, data: &mut [u8]) -> Result<(), Error> {
        let address = 0x1000_0000 | address;
        let command = SpiCommand {
            word_size: FpgaSpiWordSize::Bits16,
            byte_swap: true,
            increment_address: true,
        };
        let max_clock = ((self.system_clock.0 * 16) / (4 * 3)).min(MAX_SPI_READ_CLOCK.0);
        self.spi_read(Some(Hertz(max_clock)), command, address, data)
    }

    pub fn sdram_write(&mut self, address: u32, data: &[u8]) -> Result<(), Error> {
        let address = 0x2000_0000 | address;
        let command = SpiCommand {
            word_size: FpgaSpiWordSize::Bits32,
            byte_swap: true,
            increment_address: true,
        };
        let max_clock = (self.system_clock.0 as f32) * (32.0 / 4.0) / 3.35;
        self.spi_write(Some(Hertz(max_clock as u32)), command, address, data)
    }

    pub fn sdram_read(&mut self, address: u32, data: &mut [u8]) -> Result<(), Error> {
        let address = 0x2000_0000 | address;
        let command = SpiCommand {
            word_size: FpgaSpiWordSize::Bits32,
            byte_swap: true,
            increment_address: true,
        };
        let max_clock =
            (((self.system_clock.0 as f32) * (32.0 / 4.0) / 3.35) as u32).min(MAX_SPI_READ_CLOCK.0);
        self.spi_read(Some(Hertz(max_clock)), command, address, data)
    }

    pub fn set_overlay_bounds(
        &mut self,
        start_x: u8,
        end_x: u8,
        scroll_x: u8,
        start_y: u8,
        end_y: u8,
        scroll_y: u8,
    ) -> Result<(), Error> {
        let config_x = ((start_x as u32) & 0xFF) << 16
            | ((end_x as u32) & 0xFF) << 8
            | ((scroll_x as u32) & 0xFF);
        let config_y = ((start_y as u32) & 0xFF) << 16
            | ((end_y as u32) & 0xFF) << 8
            | ((scroll_y as u32) & 0xFF);
        self.write_u32(REG_OVERLAY_XCTRL, config_x)?;
        self.write_u32(REG_OVERLAY_YCTRL, config_y)?;
        Ok(())
    }

    pub fn hide_overlay(&mut self) -> Result<(), Error> {
        self.set_overlay_bounds(0, 0, 0, 0, 0, 0)
    }

    pub fn write_overlay(&mut self, offset: u32, data: &[u8]) -> Result<(), Error> {
        let command = SpiCommand {
            word_size: FpgaSpiWordSize::Bits16,
            byte_swap: true,
            increment_address: true,
        };
        let max_clock = (self.system_clock.0 * 16) / (4 * 2);
        self.spi_write(Some(Hertz(max_clock)), command, 0x38000000 | offset, data)
    }

    pub fn get_cartridge_slot_button(&mut self) -> Result<bool, Error> {
        Ok((self.read_u32(REG_STATUS)? & 1) != 0)
    }

    pub fn set_display_mode(&mut self, new_mode: DisplayMode) -> Result<(), Error> {
        self.write_u32(REG_DISPLAY, (new_mode == DisplayMode::External) as u32)
    }
}

#[allow(unused)]
#[derive(Copy, Clone)]
pub enum FpgaSpiWordSize {
    Bits8 = 0,
    Bits16 = 1,
    Bits32 = 2,
    Bits64 = 3,
}

#[derive(Copy, Clone)]
pub struct SpiCommand {
    pub word_size: FpgaSpiWordSize,
    pub byte_swap: bool,
    pub increment_address: bool,
}

impl SpiCommand {
    fn as_read_command(self) -> u8 {
        (1u8)
            | ((self.word_size as u8) << 1)
            | ((self.byte_swap as u8) << 3)
            | ((self.increment_address as u8) << 4)
    }

    fn as_write_command(self) -> u8 {
        (0u8)
            | ((self.word_size as u8) << 1)
            | ((self.byte_swap as u8) << 3)
            | ((self.increment_address as u8) << 4)
    }
}
