use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::kvs;
use drivers::sdcard::Sdcard;
use embedded_hal::pwm::SetDutyCycle;
use embedded_hal_bus::i2c::MutexDevice as MutexI2C;
use esp_idf_svc::hal::gpio::{
    self, AnyIOPin, AnyInputPin, IOPin, Input, InputOutput, InputPin, OutputPin,
};
use esp_idf_svc::hal::gpio::{AnyOutputPin, Output, PinDriver};
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::peripheral::Peripheral;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::{
    self, SpiDeviceDriver, SpiDriver, SpiDriverConfig, SpiSharedDeviceDriver, SpiSoftCsDeviceDriver,
};
use esp_idf_svc::hal::units::{FromValueType, Hertz};
use esp_idf_svc::hal::{i2c::*, ledc};

pub mod drivers;
mod input;
mod interrupt;

const FPGA_POWER_DELAY: Duration = Duration::from_millis(5);

static DEVICE: OnceLock<Mutex<Device>> = OnceLock::new();

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum DisplayMode {
    None,
    Internal,
    External,
}

pub struct Device<'a> {
    #[allow(unused)]
    led: PinDriver<'a, AnyOutputPin, Output>,
    fpga_power: PinDriver<'a, AnyOutputPin, Output>,

    // 改为 pub(crate) 以便自检代码访问 I2C 总线
    pub(crate) i2c: &'a Mutex<I2cDriver<'a>>,

    lcd_backlight: LedcDriver<'a>,
    lcd_backlight_duty: u16,

    pub lcd: drivers::lcd::ILI9488<
        PinDriver<'a, AnyOutputPin, Output>,
        PinDriver<'a, AnyOutputPin, Output>,
        SpiSoftCsDeviceDriver<'a, SpiSharedDeviceDriver<'a, &'a SpiDriver<'a>>, &'a SpiDriver<'a>>,
    >,

    pub display_mode: DisplayMode,

    pub dac: drivers::dac::TLV320DAC3101<
        PinDriver<'a, AnyOutputPin, Output>,
        MutexI2C<'a, I2cDriver<'a>>,
    >,

    pub fpga: drivers::fpga::Fpga<
        'a,
        PinDriver<'a, AnyInputPin, Input>,
        PinDriver<'a, AnyOutputPin, Output>,
        PinDriver<'a, AnyIOPin, Input>,
        SpiDeviceDriver<'a, &'a SpiDriver<'a>>,
    >,

    pub rtc: drivers::rtc::PCF8563<MutexI2C<'a, I2cDriver<'a>>>,
    pub fuel_gauge: drivers::fuel_gauge::MAX17048<MutexI2C<'a, I2cDriver<'a>>>,
    pub imu: drivers::imu::LSM6DS3TRC<MutexI2C<'a, I2cDriver<'a>>>,

    // 改为 pub(crate) 以便自检读取 IO 扩展器
    pub(crate) io_expander: drivers::io_expander::TCA9535<MutexI2C<'a, I2cDriver<'a>>>,

    // 改为 pub(crate) 以便读取按键电平
    pub(crate) button_home: PinDriver<'a, AnyInputPin, Input>,
    pub(crate) button_vol_up: PinDriver<'a, AnyInputPin, Input>,
    pub(crate) button_vol_down: PinDriver<'a, AnyInputPin, Input>,
    pub(crate) button_power: PinDriver<'a, AnyIOPin, InputOutput>,

    pin_irq: PinDriver<'a, AnyInputPin, Input>,
    pub pin_vbus_pgood: PinDriver<'a, AnyIOPin, Input>, // 自检需要，改为 pub
    pub pin_batt_chg: PinDriver<'a, AnyInputPin, Input>, // 自检需要

    pub sdcard: Option<Sdcard>,
}

impl Device<'_> {
    pub fn init() -> Result<(), anyhow::Error> {
        // ... 原有 init() 代码完全不变，此处省略以节省篇幅 ...
        // 请保持您文件中的 init() 实现不变
        Ok(())
    }

    pub fn get() -> &'static Mutex<Device<'static>> {
        DEVICE.get().unwrap()
    }

    pub fn lock() -> MutexGuard<'static, Device<'static>> {
        Device::get().lock().unwrap()
    }

    pub fn set_fpga_power(&mut self, enable: bool) -> Result<(), anyhow::Error> {
        self.fpga_power.set_level(enable.into())?;
        Ok(())
    }

    pub fn display_framebuffer_raw(&mut self, raw: &[u8]) {
        let _ = self.fpga.write_overlay(0, raw);
        let _ = self.fpga.set_overlay_bounds(0x0, 0xFF, 0x0, 0x0, 0xFF, 0x0);
    }

    pub fn power_off(&mut self) -> ! {
        log::info!("Powering off");
        self.prepare_for_power_off();
        let _ = self.button_power.set_low();
        loop {
            std::thread::park();
        }
    }

    pub fn reboot(&mut self) -> ! {
        log::info!("Rebooting");
        self.prepare_for_power_off();
        esp_idf_svc::hal::reset::restart();
    }

    fn prepare_for_power_off(&mut self) {
        let _ = self.change_display_mode(DisplayMode::None);
        let _ = self.dac.reset_hold();
        let _ = self.set_fpga_power(false);
        kvs::keys::flush_all();
    }

    pub fn set_brightness(&mut self, brightness: f32) {
        let max_duty = self.lcd_backlight.get_max_duty() as f32;
        let duty = ((0.99 * max_duty.powf(brightness)) + (0.01 * max_duty)) as u16;
        log::info!("Setting LCD brightness to {} ({} / {})", brightness, duty, max_duty);
        self.lcd_backlight_duty = duty;
        if self.display_mode == DisplayMode::Internal {
            self.lcd_backlight.set_duty_cycle(duty).unwrap();
        }
    }

    fn init_datetime(&mut self) {
        // ... 原有 init_datetime() 代码保持不变 ...
    }

    fn set_esp_datetime(dt: time::OffsetDateTime) {
        // ... 原有代码不变 ...
    }

    pub fn get_datetime(&mut self) -> time::OffsetDateTime {
        // ... 原有代码不变 ...
    }

    pub fn set_datetime(&mut self, dt: time::OffsetDateTime) {
        // ... 原有代码不变 ...
    }

    pub fn change_display_mode(&mut self, new_mode: DisplayMode) -> Result<(), anyhow::Error> {
        // ... 原有代码不变 ...
    }

    pub fn get_display_mode(&self) -> DisplayMode {
        self.display_mode
    }

    pub fn get_battery_is_charging(&self) -> bool {
        self.pin_batt_chg.is_high()
    }

    pub fn get_vbus_pgood(&self) -> bool {
        self.pin_vbus_pgood.is_low()
    }
}