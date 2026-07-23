use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Context;    // ← 新增
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

/// Time it may take for FPGA power rails to stabilize after enable.
const FPGA_POWER_DELAY: Duration = Duration::from_millis(5);

static DEVICE: OnceLock<Mutex<Device>> = OnceLock::new();

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum DisplayMode {
    None,
    Internal,
    External,
}

/// Main container for device hardware.
pub struct Device<'a> {
    /// Status led, active-high.
    #[allow(unused)]
    led: PinDriver<'a, AnyOutputPin, Output>,

    /// FPGA power in, active-high.
    fpga_power: PinDriver<'a, AnyOutputPin, Output>,

    /// The I2C bus.
    #[allow(unused)]
    i2c: &'a Mutex<I2cDriver<'a>>,

    /// LCD backlight PWM driver.
    lcd_backlight: LedcDriver<'a>,
    lcd_backlight_duty: u16,

    /// LCD driver
    pub lcd: drivers::lcd::ILI9488<
        PinDriver<'a, AnyOutputPin, Output>,
        PinDriver<'a, AnyOutputPin, Output>,
        SpiSoftCsDeviceDriver<'a, SpiSharedDeviceDriver<'a, &'a SpiDriver<'a>>, &'a SpiDriver<'a>>,
    >,

    /// Display mode (if initialized)
    pub display_mode: DisplayMode,

    /// DAC driver
    pub dac: drivers::dac::TLV320DAC3101<
        PinDriver<'a, AnyOutputPin, Output>,
        MutexI2C<'a, I2cDriver<'a>>,
    >,

    /// FPGA driver
    pub fpga: drivers::fpga::Fpga<
        'a,
        PinDriver<'a, AnyInputPin, Input>,
        PinDriver<'a, AnyOutputPin, Output>,
        PinDriver<'a, AnyIOPin, Input>,
        SpiDeviceDriver<'a, &'a SpiDriver<'a>>,
    >,

    /// RTC driver
    pub rtc: drivers::rtc::PCF8563<MutexI2C<'a, I2cDriver<'a>>>,

    /// Battery fuel gauge driver
    pub fuel_gauge: drivers::fuel_gauge::MAX17048<MutexI2C<'a, I2cDriver<'a>>>,

    /// IMU driver
    pub imu: drivers::imu::LSM6DS3TRC<MutexI2C<'a, I2cDriver<'a>>>,

    io_expander: drivers::io_expander::TCA9535<MutexI2C<'a, I2cDriver<'a>>>,
    button_home: PinDriver<'a, AnyInputPin, Input>,
    button_vol_up: PinDriver<'a, AnyInputPin, Input>,
    button_vol_down: PinDriver<'a, AnyInputPin, Input>,
    button_power: PinDriver<'a, AnyIOPin, InputOutput>,
    pin_irq: PinDriver<'a, AnyInputPin, Input>,
    pin_vbus_pgood: PinDriver<'a, AnyIOPin, Input>,
    pin_batt_chg: PinDriver<'a, AnyInputPin, Input>,

    /// Sdcard,
    pub sdcard: Option<Sdcard>,
}

impl Device<'_> {
    pub fn init() -> Result<(), anyhow::Error> {
        if let Some(_) = DEVICE.get() {
            panic!("Device already initialized.");
        }

        log::info!("[INIT] 开始 Device 硬件初始化");

        let peripherals = Peripherals::take().context("无法获取外设句柄（ESP-IDF 错误）")?;
        cfg_if::cfg_if! {
            if #[cfg(feature = "rev1")] {
                let pin_led = peripherals.pins.gpio3.downgrade_output();
                let pin_irq = peripherals.pins.gpio2.downgrade_input();
                let pin_home = peripherals.pins.gpio0.downgrade_input();
                let pin_vol_up = peripherals.pins.gpio4.downgrade_input();
                let pin_vol_down = peripherals.pins.gpio5.downgrade_input();
                let pin_power_switch = peripherals.pins.gpio1.downgrade();
                let pin_vbus_pgood = peripherals.pins.gpio41.downgrade();
                let pin_batt_chg = peripherals.pins.gpio42.downgrade_input();
                let pin_lcd_backlight = peripherals.pins.gpio6.downgrade_output();
                let pin_lcd_reset = peripherals.pins.gpio7.downgrade_output();
                let pin_lcd_cs = peripherals.pins.gpio15.downgrade_output();
                let pin_lcd_dc = peripherals.pins.gpio16.downgrade_output();
                let pin_fpga_power = peripherals.pins.gpio46.downgrade_output();
                let pin_fpga_init_b = peripherals.pins.gpio8.downgrade();
                let pin_fpga_done = peripherals.pins.gpio17.downgrade_input();
                let pin_fpga_program_b = peripherals.pins.gpio18.downgrade_output();
                let mut pin_fpga_spi_cs = peripherals.pins.gpio10.downgrade_output();
                let pin_spi_clk = peripherals.pins.gpio12.downgrade_output();
                let pin_spi_d0 = peripherals.pins.gpio11.downgrade();
                let pin_spi_d1 = peripherals.pins.gpio13.downgrade();
                let pin_spi_d2 = peripherals.pins.gpio14.downgrade();
                let pin_spi_d3 = peripherals.pins.gpio9.downgrade();
                let pin_i2c_scl = peripherals.pins.gpio39.downgrade();
                let pin_i2c_sda = peripherals.pins.gpio38.downgrade();
                let pin_sdio_clk = peripherals.pins.gpio45.downgrade_output();
                let pin_sdio_cmd = peripherals.pins.gpio48.downgrade();
                let pin_sdio_d0 = peripherals.pins.gpio35.downgrade();
                let pin_sdio_d1 = peripherals.pins.gpio36.downgrade();
                let pin_sdio_d2 = peripherals.pins.gpio21.downgrade();
                let pin_sdio_d3 = peripherals.pins.gpio47.downgrade();
                let pin_sd_detect = peripherals.pins.gpio37.downgrade_input();
                let pin_dac_reset = peripherals.pins.gpio40.downgrade_output();
            } else if #[cfg(feature = "rev2")] {
                let pin_led = peripherals.pins.gpio36.downgrade_output();
                let pin_irq = peripherals.pins.gpio16.downgrade_input();
                let pin_home = peripherals.pins.gpio0.downgrade_input();
                let pin_vol_up = peripherals.pins.gpio41.downgrade_input();
                let pin_vol_down = peripherals.pins.gpio40.downgrade_input();
                let pin_power_switch = peripherals.pins.gpio15.downgrade();
                let pin_vbus_pgood = peripherals.pins.gpio17.downgrade();
                let pin_batt_chg = peripherals.pins.gpio39.downgrade_input();
                let pin_lcd_backlight = peripherals.pins.gpio42.downgrade_output();
                let pin_lcd_reset = peripherals.pins.gpio1.downgrade_output();
                let pin_lcd_cs = peripherals.pins.gpio2.downgrade_output();
                let pin_lcd_dc = peripherals.pins.gpio3.downgrade_output();
                let pin_lcd_spi_clk = peripherals.pins.gpio4.downgrade_output();
                let pin_lcd_spi_d0 = peripherals.pins.gpio5.downgrade_output();
                let pin_fpga_power = peripherals.pins.gpio46.downgrade_output();
                let pin_fpga_init_b = peripherals.pins.gpio8.downgrade();
                let pin_fpga_done = peripherals.pins.gpio6.downgrade_input();
                let pin_fpga_program_b = peripherals.pins.gpio7.downgrade_output();
                let mut pin_fpga_spi_cs = peripherals.pins.gpio10.downgrade_output();
                let pin_fpga_spi_clk = peripherals.pins.gpio12.downgrade_output();
                let pin_fpga_spi_d0 = peripherals.pins.gpio11.downgrade();
                let pin_fpga_spi_d1 = peripherals.pins.gpio13.downgrade();
                let pin_fpga_spi_d2 = peripherals.pins.gpio14.downgrade();
                let pin_fpga_spi_d3 = peripherals.pins.gpio9.downgrade();
                let pin_i2c_scl = peripherals.pins.gpio37.downgrade();
                let pin_i2c_sda = peripherals.pins.gpio38.downgrade();
                let pin_sdio_clk = peripherals.pins.gpio33.downgrade_output();
                let pin_sdio_cmd = peripherals.pins.gpio47.downgrade();
                let pin_sdio_d0 = peripherals.pins.gpio34.downgrade();
                let pin_sdio_d1 = peripherals.pins.gpio48.downgrade();
                let pin_sdio_d2 = peripherals.pins.gpio21.downgrade();
                let pin_sdio_d3 = peripherals.pins.gpio26.downgrade();
                let pin_sd_detect = peripherals.pins.gpio35.downgrade_input();
                let pin_dac_reset = peripherals.pins.gpio45.downgrade_output();
            }
        }
        log::info!("[INIT] 引脚配置完成（{} 版本）", if cfg!(feature = "rev1") { "rev1" } else { "rev2" });

        // Status LED
        log::info!("[INIT] 初始化状态 LED");
        let mut led = PinDriver::output(pin_led).context("状态 LED 引脚初始化失败（检查 GPIO 是否被占用或短路）")?;
        led.set_low()?;

        // FPGA power
        log::info!("[INIT] 开启 FPGA 电源");
        let mut fpga_power = PinDriver::output(pin_fpga_power).context("FPGA 电源引脚初始化失败（检查 pin_fpga_power 硬件连接）")?;
        fpga_power.set_high()?;
        let fpga_power_time = Instant::now();

        // Initialize I2C
        log::info!("[INIT] 初始化 I2C 总线（400kHz）");
        let i2c_config = I2cConfig::new().baudrate(400.kHz().into());
        let i2c = I2cDriver::new(peripherals.i2c0, pin_i2c_sda, pin_i2c_scl, &i2c_config)
            .context("I2C 驱动初始化失败：请检查 SDA/SCL 引脚焊接、上拉电阻以及是否与其他设备短路")?;
        let i2c = &*Box::leak(Box::new(Mutex::new(i2c)));

        let pin_irq = PinDriver::input(pin_irq).context("IRQ 引脚配置失败（检查 IO 口）")?;

        // LCD backlight
        log::info!("[INIT] 初始化 LCD 背光 PWM");
        let mut lcd_backlight = LedcDriver::new(
            peripherals.ledc.channel0,
            LedcTimerDriver::new(
                peripherals.ledc.timer0,
                &ledc::config::TimerConfig::new()
                    .frequency(25.kHz().into())
                    .resolution(ledc::config::Resolution::Bits11),
            ).context("LCD 背光 PWM 定时器初始化失败")?,
            pin_lcd_backlight,
        ).context("LCD 背光 PWM 通道初始化失败（检查 pin_lcd_backlight 是否焊接正确）")?;
        lcd_backlight.set_duty_cycle_fully_off().unwrap();

        // Setup SPI
        log::info!("[INIT] 初始化 SPI 总线");
        let spi_driver_config = SpiDriverConfig::new().dma(spi::Dma::Auto(32 * 1024));
        cfg_if::cfg_if! {
            if #[cfg(feature = "rev1")] {
                let spi_driver = &*Box::leak(Box::new(SpiDriver::new_quad(
                    peripherals.spi2,
                    pin_spi_clk,
                    pin_spi_d0,
                    pin_spi_d1,
                    pin_spi_d2,
                    pin_spi_d3,
                    &spi_driver_config,
                ).context("SPI 总线（Quad模式）初始化失败（检查 SPI 引脚焊接）")?));
                let fpga_spi_driver = spi_driver;
                let lcd_spi_driver = spi_driver;
            } else if #[cfg(feature = "rev2")] {
                let fpga_spi_driver = &*Box::leak(Box::new(SpiDriver::new_quad(
                    peripherals.spi2,
                    pin_fpga_spi_clk,
                    pin_fpga_spi_d0,
                    pin_fpga_spi_d1,
                    pin_fpga_spi_d2,
                    pin_fpga_spi_d3,
                    &spi_driver_config,
                ).context("FPGA SPI 总线初始化失败（检查 rev2 原理图对应引脚：CLK=12, D0=11, D1=13, D2=14, D3=9）")?));
                let lcd_spi_driver = &*Box::leak(Box::new(SpiDriver::new(
                    peripherals.spi3,
                    pin_lcd_spi_clk,
                    pin_lcd_spi_d0,
                    Option::<AnyInputPin>::None,
                    &spi_driver_config,
                ).context("LCD SPI 总线初始化失败（检查 rev2：CLK=4, D0=5）")?));
            }
        }

        // Setup LCD
        log::info!("[INIT] 开始初始化 LCD");
        let lcd_spi_config = spi::config::Config::new().baudrate(10.MHz().into());
        let lcd_spi = SpiSoftCsDeviceDriver::new(
            SpiSharedDeviceDriver::new(lcd_spi_driver, &lcd_spi_config).context("LCD SPI 设备创建失败")?,
            pin_lcd_cs,
            gpio::Level::High,
        ).context("LCD CS 引脚配置失败")?;
        let lcd_reset = PinDriver::output(pin_lcd_reset).context("LCD 复位引脚配置失败")?;
        let lcd_dc = PinDriver::output(pin_lcd_dc).context("LCD DC 引脚配置失败")?;
        let mut lcd = drivers::lcd::ILI9488::new(lcd_reset, lcd_dc, lcd_spi);
        lcd.init().context("LCD 初始化失败：请检查 LCD 连接、复位电平及 SPI 信号完整性")?;
        log::info!("[INIT] LCD 初始化成功");

        // Setup I/O expander
        log::info!("[INIT] 初始化 I/O 扩展器 TCA9535（I2C）");
        let mut io_expander = drivers::io_expander::TCA9535::new(MutexI2C::new(&i2c));
        io_expander.get_pins().context("TCA9535 通信失败：检查 I2C 地址、焊接及电源")?;

        // Direct buttons
        log::info!("[INIT] 初始化按键 GPIO");
        let button_home = PinDriver::input(pin_home).context("Home 按键引脚配置失败")?;
        let button_vol_up = PinDriver::input(pin_vol_up).context("Vol+ 按键引脚配置失败")?;
        let button_vol_down = PinDriver::input(pin_vol_down).context("Vol- 按键引脚配置失败")?;
        let mut button_power = PinDriver::input_output_od(pin_power_switch).context("电源键引脚配置失败")?;
        button_power.set_high()?;

        // Setup RTC
        log::info!("[INIT] 初始化 RTC (PCF8563)");
        let rtc = drivers::rtc::PCF8563::new(MutexI2C::new(&i2c));

        // Setup battery fuel gauge
        log::info!("[INIT] 初始化电池计量计 MAX17048");
        let mut fuel_gauge = drivers::fuel_gauge::MAX17048::new(MutexI2C::new(&i2c));
        let _ = fuel_gauge.set_alert_soc_change(true); // 无电池时可能失败，忽略
        let mut pin_vbus_pgood = PinDriver::input(pin_vbus_pgood).context("VBUS PG 引脚配置失败")?;
        pin_vbus_pgood.set_pull(gpio::Pull::Up)?;
        let pin_batt_chg = PinDriver::input(pin_batt_chg).context("充电状态引脚配置失败")?;

        // Setup IMU
        log::info!("[INIT] 初始化 IMU (LSM6DS3TRC)");
        let mut imu = drivers::imu::LSM6DS3TRC::new(MutexI2C::new(&i2c));
        imu.init().context("IMU 初始化失败：检查 I2C 连接、电源；芯片焊接是否正常？")?;

        // Ensure fpga power has stabilized.
        let time_since_fpga_power = Instant::now().duration_since(fpga_power_time);
        std::thread::sleep(FPGA_POWER_DELAY.saturating_sub(time_since_fpga_power));
        log::info!("[INIT] FPGA 电源稳定等待完成");

        // Setup DAC (requires fpga_power on)
        log::info!("[INIT] 开始初始化 DAC (TLV320DAC3101)");
        let dac_reset = PinDriver::output(pin_dac_reset).context("DAC 复位引脚配置失败")?;
        let mut dac = drivers::dac::TLV320DAC3101::new(dac_reset, MutexI2C::new(&i2c));
        dac.init().context("DAC 初始化失败：检查 DAC 电源、I2C 响应及硬件复位")?;
        dac.configure_interrupts().context("DAC 中断配置失败")?;
        dac.set_volume(kvs::keys::VOLUME.get().unwrap())?;
        dac.set_mute(false)?;
        let headphones_detected = dac.get_headphones_detected().context("DAC 耳机检测失败")?;
        dac.set_headphones_enabled(headphones_detected)?;
        dac.set_speakers_enabled(!headphones_detected)?;
        log::info!("[INIT] DAC 初始化成功（耳机={}）", headphones_detected);

        // Setup FPGA (without programming)
        log::info!("[INIT] 初始化 FPGA（不烧录逻辑）");
        let fpga_done = PinDriver::input(pin_fpga_done).context("FPGA DONE 引脚配置失败")?;
        let mut fpga_program_b = PinDriver::output_od(pin_fpga_program_b).context("FPGA PROGRAM_B 引脚配置失败")?;
        fpga_program_b.set_high()?;
        let fpga_init_b = PinDriver::input(pin_fpga_init_b).context("FPGA INIT_B 引脚配置失败")?;

        let spi_rates = [40.MHz(), 20.MHz(), 16.MHz(), 10.MHz()];
        let fpga_data_spis = spi_rates
            .iter()
            .map(|&rate| {
                let config = spi::config::Config::new()
                    .baudrate(rate.into())
                    .duplex(spi::config::Duplex::Half);
                let cs_pin = unsafe { pin_fpga_spi_cs.clone_unchecked() };
                let mut spi = SpiSoftCsDeviceDriver::new(
                    SpiSharedDeviceDriver::new(fpga_spi_driver, &config).unwrap(),
                    cs_pin,
                    gpio::Level::High,
                )
                .unwrap();
                spi.cs_pre_delay_us(100);
                (spi, Hertz::from(rate))
            })
            .collect::<Vec<_>>();
        log::info!("[INIT] 为 FPGA 创建了 {} 个速度等级的 SPI 设备", spi_rates.len());

        let fpga_program_config = spi::config::Config::new().baudrate(80.MHz().into());
        let fpga_program_spi = SpiDeviceDriver::new(
            fpga_spi_driver,
            Option::<AnyOutputPin>::None,
            &fpga_program_config,
        ).context("FPGA 烧录 SPI 初始化失败")?;
        let fpga = drivers::fpga::Fpga::new(
            fpga_done,
            fpga_program_b,
            fpga_init_b,
            fpga_data_spis,
            fpga_program_spi,
        );

        // Mount sdcard to /sdcard
        log::info!("[INIT] 挂载 SD 卡");
        let sdcard = drivers::sdcard::mount_sdcard(
            "/sdcard",
            pin_sdio_clk,
            pin_sdio_cmd,
            pin_sdio_d0,
            pin_sdio_d1,
            pin_sdio_d2,
            pin_sdio_d3,
            Some(pin_sd_detect),
        )
        .ok();
        if sdcard.is_some() {
            log::info!("[INIT] SD 卡挂载成功");
        } else {
            log::warn!("[INIT] SD 卡挂载失败（可能未插入或硬件问题）");
        }

        let mut device = Device {
            led,
            fpga_power,
            i2c,
            lcd_backlight,
            lcd_backlight_duty: 0,
            lcd,
            display_mode: DisplayMode::None,
            dac,
            fpga,
            fuel_gauge,
            io_expander,
            button_home,
            button_power,
            button_vol_up,
            button_vol_down,
            pin_irq,
            pin_batt_chg,
            pin_vbus_pgood,
            rtc,
            imu,
            sdcard,
        };
        log::info!("[INIT] 所有外设结构体创建完毕");

        device.init_datetime();
        DEVICE
            .set(Mutex::new(device))
            .map_err(|_| ())
            .expect("Device already initialized");

        Device::setup_interrupts();
        log::info!("[INIT] Device 初始化全部完成（中断已注册）");

        Ok(())
    }

    // ... 后面的方法保持原样，这里省略以节省篇幅 ...
    // 实际上你需要保留 Device 的所有其他方法，请把上一轮我发的完整文件中后面部分也复制进去。
    // 为了简洁，这里只贴了 init() 修改的部分，实际应用中务必把完整文件替换。
}
