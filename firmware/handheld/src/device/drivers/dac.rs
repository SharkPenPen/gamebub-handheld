#![allow(dead_code)]

use std::time::Duration;

use embedded_hal::digital::OutputPin;
use embedded_hal::i2c::I2c;
use thiserror::Error;

const ADDRESS: u8 = 0x18;

#[derive(Debug, Error)]
pub enum Error {
    #[error("gpio error")]
    PinError,
    #[error("i2c communication error")]
    I2cError,
}

#[derive(Copy, Clone)]
pub struct InterruptStatus {
    pub left_short_circuit: bool,
    pub right_short_circuit: bool,
    pub headset_button_pressed: bool,
    pub headset_detected: bool,
    pub left_dac_power: bool,
    pub right_dac_power: bool,
}

pub struct TLV320DAC3101<PinReset: OutputPin, I2C: I2c> {
    pin_reset: PinReset,
    i2c: I2C,
    page: u8,
    volume: u8,
    mute: bool,
}

impl<PinReset, I2C> TLV320DAC3101<PinReset, I2C>
where
    PinReset: OutputPin,
    I2C: I2c,
{
    pub fn new(pin_reset: PinReset, i2c: I2C) -> Self {
        TLV320DAC3101 {
            pin_reset,
            i2c,
            page: 0,
            volume: 0,
            mute: false,
        }
    }

    pub fn reset(&mut self) -> Result<(), Error> {
        self.pin_reset.set_low().map_err(|_| Error::PinError)?;
        std::thread::sleep(Duration::from_micros(1));
        self.pin_reset.set_high().map_err(|_| Error::PinError)?;
        std::thread::sleep(Duration::from_millis(1));
        Ok(())
    }

    pub fn reset_hold(&mut self) -> Result<(), Error> {
        self.pin_reset.set_low().map_err(|_| Error::PinError)
    }

    // ===============================================================
    // 重点修改：在这里加入了 ID 验证和日志输出
    // ===============================================================
    pub fn init(&mut self) -> Result<(), Error> {
        log::info!("[DAC] 1. 正在拉高 RESET 引脚...");
        self.reset()?;
        
        log::info!("[DAC] 2. 尝试读取 ID 以确认硬件 I2C 握手...");
        let mut id_buffer = [0u8; 1];
        match self.i2c.write_read(ADDRESS, &[0x00], &mut id_buffer) {
            Ok(_) => {
                let actual_id = id_buffer[0];
                log::info!("[DAC] ✅ 读到的 ID 字节为: 0x{:02X}", actual_id);
                if actual_id != 0x18 {
                    log::error!("[DAC] ❌ ID 不匹配！期望 0x18，实际收到 0x{:02X}", actual_id);
                    log::error!("   -> 注意：如果 I2C 扫到了地址但 ID 是 0xFF，可能是 FPGA 没提供 MCLK 时钟信号，或者 QFN 底部悬空！");
                    return Err(Error::I2cError);
                } else {
                    log::info!("[DAC] ✅ ID 校验通过！芯片已准备好。");
                }
            }
            Err(e) => {
                log::error!("[DAC] ❌ I2C 读操作直接失败: {:?}", e);
                log::error!("   -> 请检查 3脚、17脚的 1.8V 是否供电，以及 I2C 总线 SDA/SCL 是否连锡。");
                return Err(Error::I2cError);
            }
        }

        // 下面是原作者标准的初始化寄存器的代码（不需要动）
        self.write_reg(0, 0x04, 0x00)?;
        self.write_reg(0, 0x0B, 0x81)?;
        self.write_reg(0, 0x0C, 0x82)?;
        self.write_reg(0, 0x0D, 0x00)?;
        self.write_reg(0, 0x0E, 0x80)?;
        self.write_reg(0, 0x1B, 0x00)?;
        self.write_reg(0, 0x3C, 0x07)?;
        self.write_reg(0, 0x00, 0x08)?;
        self.write_reg(0, 0x01, 0x04)?;
        self.write_reg(0, 0x00, 0x00)?;
        self.write_reg(0, 0x74, 0x00)?;

        self.write_reg(1, 0x1F, 0x0C)?;
        self.write_reg(1, 0x21, 0x4E)?;
        self.write_reg(1, 0x23, 0x44)?;
        self.write_reg(1, 0x28, 0x06)?;
        self.write_reg(1, 0x29, 0x06)?;
        self.write_reg(1, 0x2A, 0x1C)?;
        self.write_reg(1, 0x24, 0x92)?;
        self.write_reg(1, 0x25, 0x92)?;
        self.write_reg(1, 0x26, 0x92)?;
        self.write_reg(1, 0x1F, 0xC2)?;
        self.write_reg(1, 0x20, 0x86)?;

        self.write_reg(0, 0x3F, 0xD4)?;
        self.write_reg(0, 0x41, 0xD4)?;
        self.write_reg(0, 0x42, 0xD4)?;
        self.write_reg(0, 0x40, 0x00)?;
        self.write_reg(0, 0x00, 0x00)?;

        log::info!("[DAC] 初始化完成！");
        Ok(())
    }

    fn write_reg(&mut self, page: u8, reg: u8, data: u8) -> Result<(), Error> {
        if self.page != page {
            self.i2c.write(ADDRESS, &[0x00, page]).map_err(|_| Error::I2cError)?;
            self.page = page;
        }
        self.i2c.write(ADDRESS, &[reg, data]).map_err(|_| Error::I2cError)
    }
}
