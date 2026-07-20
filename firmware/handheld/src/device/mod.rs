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

        let peripherals = Peripherals::take()?;
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

        // Status LED
        let mut led = PinDriver::output(pin_led)?;
        led.set_low()?;

        // TODO: see if we can avoid keeping FPGA power on all the time
        let mut fpga_power = PinDriver::output(pin_fpga_power)?;
        fpga_power.set_high()?;
        let fpga_power_time = Instant::now();

        // Initialize I2C
        // TODO: see if there's a good way to do this without making and leaking a Box
        let i2c_config = I2cConfig::new().baudrate(400.kHz().into());
        let i2c = I2cDriver::new(peripherals.i2c0, pin_i2c_sda, pin_i2c_scl, &i2c_config)?;
        let i2c = &*Box::leak(Box::new(Mutex::new(i2c)));

        let pin_irq = PinDriver::input(pin_irq)?;

        // LCD backlight
        let mut lcd_backlight = LedcDriver::new(
            peripherals.ledc.channel0,
            LedcTimerDriver::new(
                peripherals.ledc.timer0,
                &ledc::config::TimerConfig::new()
                    .frequency(25.kHz().into())
                    .resolution(ledc::config::Resolution::Bits11),
            )?,
            pin_lcd_backlight,
        )?;
        lcd_backlight.set_duty_cycle_fully_off().unwrap();

        // Setup SPI
        // TODO: see if there's a good way to do this without making and leaking a Box
        // Use DMA transfers, with an auto-assigned channel, and a maximum transfer size of 32 KiB.
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
                )?));
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
                )?));
                let lcd_spi_driver = &*Box::leak(Box::new(SpiDriver::new(
                    peripherals.spi3,
                    pin_lcd_spi_clk,
                    pin_lcd_spi_d0,
                    Option::<AnyInputPin>::None,
                    &spi_driver_config,
                )?));
            }
        }

        // Setup LCD
        log::info!("Initializing LCD");
        let lcd_spi_config = spi::config::Config::new().baudrate(10.MHz().into());
        let lcd_spi = SpiSoftCsDeviceDriver::new(
            SpiSharedDeviceDriver::new(lcd_spi_driver, &lcd_spi_config)?,
            pin_lcd_cs,
            gpio::Level::High,
        )?;
        let lcd_reset = PinDriver::output(pin_lcd_reset)?;
        let lcd_dc = PinDriver::output(pin_lcd_dc)?;
        let mut lcd = drivers::lcd::ILI9488::new(lcd_reset, lcd_dc, lcd_spi);
        lcd.init()?;

        // Setup I/O expander
        let mut io_expander = drivers::io_expander::TCA9535::new(MutexI2C::new(&i2c));
        io_expander.get_pins()?;

        // Direct buttons
        let button_home = PinDriver::input(pin_home)?;
        let button_vol_up = PinDriver::input(pin_vol_up)?;
        let button_vol_down = PinDriver::input(pin_vol_down)?;
        let mut button_power = PinDriver::input_output_od(pin_power_switch)?;
        button_power.set_high()?;

        // Setup RTC
        let rtc = drivers::rtc::PCF8563::new(MutexI2C::new(&i2c));

        // Setup battery fuel gauge
        let mut fuel_gauge = drivers::fuel_gauge::MAX17048::new(MutexI2C::new(&i2c));
        let _ = fuel_gauge.set_alert_soc_change(true); // fuel gauge won't work without a battery
        let mut pin_vbus_pgood = PinDriver::input(pin_vbus_pgood)?;
        pin_vbus_pgood.set_pull(gpio::Pull::Up)?;
        let pin_batt_chg = PinDriver::input(pin_batt_chg)?;

        // Setup IMU
        let mut imu = drivers::imu::LSM6DS3TRC::new(MutexI2C::new(&i2c));
        imu.init()?;

        // Ensure fpga power has stabilized.
        let time_since_fpga_power = Instant::now().duration_since(fpga_power_time);
        std::thread::sleep(FPGA_POWER_DELAY.saturating_sub(time_since_fpga_power));

        // Setup DAC (requires fpga_power on)
        log::info!("Initializing DAC");
        let dac_reset = PinDriver::output(pin_dac_reset)?;
        let mut dac = drivers::dac::TLV320DAC3101::new(dac_reset, MutexI2C::new(&i2c));
        dac.init()?;
        dac.configure_interrupts()?;
        dac.set_volume(kvs::keys::VOLUME.get().unwrap())?;
        dac.set_mute(false)?;
        let headphones_detected = dac.get_headphones_detected()?;
        dac.set_headphones_enabled(headphones_detected)?;
        dac.set_speakers_enabled(!headphones_detected)?;

        // Setup FPGA (without programming)
        let fpga_done = PinDriver::input(pin_fpga_done)?;
        let mut fpga_program_b = PinDriver::output_od(pin_fpga_program_b)?;
        fpga_program_b.set_high()?; // Initializing pin sets this to low -- release it to high-z immediately.
        let fpga_init_b = PinDriver::input(pin_fpga_init_b)?;

        let spi_rates = [40.MHz(), 20.MHz(), 16.MHz(), 10.MHz()];
        let fpga_data_spis = spi_rates
            .iter()
            .map(|&rate| {
                let config = spi::config::Config::new()
                    .baudrate(rate.into())
                    .duplex(spi::config::Duplex::Half);
                // SAFETY: this CS pin is used in all the data SPIs.
                // They're used in the same mode (output), and controlled only by software,
                // and never used at the same time (either read *or* write are used at any time).
                // There doesn't appear to be any other way to use multiple clock speeds.
                let cs_pin = unsafe { pin_fpga_spi_cs.clone_unchecked() };
                let mut spi = SpiSoftCsDeviceDriver::new(
                    SpiSharedDeviceDriver::new(fpga_spi_driver, &config).unwrap(),
                    cs_pin,
                    gpio::Level::High,
                )
                .unwrap();
                spi.cs_pre_delay_us(100); // FPGA spi requires >35uS or so to stabilize after nCS.
                (spi, Hertz::from(rate))
            })
            .collect::<Vec<_>>();
        let fpga_program_config = spi::config::Config::new().baudrate(80.MHz().into());
        let fpga_program_spi = SpiDeviceDriver::new(
            fpga_spi_driver,
            Option::<AnyOutputPin>::None,
            &fpga_program_config,
        )?;
        let fpga = drivers::fpga::Fpga::new(
            fpga_done,
            fpga_program_b,
            fpga_init_b,
            fpga_data_spis,
            fpga_program_spi,
        );

        // Mount sdcard to /sdcard
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
        device.init_datetime();
        DEVICE
            .set(Mutex::new(device))
            .map_err(|_| ())
            .expect("Device already initialized");

        Device::setup_interrupts();

        Ok(())
    }

    pub fn get() -> &'static Mutex<Device<'static>> {
        DEVICE.get().unwrap()
    }

    pub fn lock() -> MutexGuard<'static, Device<'static>> {
        Device::get().lock().unwrap()
    }

    /// Enable or disable FPGA power.
    ///
    /// Note that it may take around 5ms to stabilize.
    pub fn set_fpga_power(&mut self, enable: bool) -> Result<(), anyhow::Error> {
        // TODO: maybe return a Future that completes after it's stable?
        self.fpga_power.set_level(enable.into())?;
        Ok(())
    }

    /// Display a framebuffer.
    ///
    /// Currently always an FPGA overlay.
    pub fn display_framebuffer_raw(&mut self, raw: &[u8]) {
        let _ = self.fpga.write_overlay(0, raw);
        let _ = self.fpga.set_overlay_bounds(0x0, 0xFF, 0x0, 0x0, 0xFF, 0x0);
    }

    /// Gracefully turn the device off.
    pub fn power_off(&mut self) -> ! {
        log::info!("Powering off");
        self.prepare_for_power_off();

        // Hold down the power button until the device shuts off.
        let _ = self.button_power.set_low();
        loop {
            std::thread::park();
        }
    }

    /// Gracefully soft reset.
    pub fn reboot(&mut self) -> ! {
        log::info!("Rebooting");
        self.prepare_for_power_off();
        esp_idf_svc::hal::reset::restart();
    }

    /// Prepare for power-off or reset, by powering down peripherals and saving state.
    ///
    /// The device must go through a [soft] reset to function correctly after this.
    fn prepare_for_power_off(&mut self) {
        let _ = self.change_display_mode(DisplayMode::None);
        // Hold DAC in reset, otherwise it interferes with I2C if rebooting.
        let _ = self.dac.reset_hold();
        // TODO: high-Z SPI, other FPGA-power-domain pins.
        let _ = self.set_fpga_power(false);
        kvs::keys::flush_all();
    }

    /// Set the LCD brightness. The input is a float in the range [0.0, 1.0].
    pub fn set_brightness(&mut self, brightness: f32) {
        // Brightness is perceived non-linearly -- 50% brightness is one step
        // less bright than 100%, 25% is one step less than 50%, etc.
        // Note that <1% duty cycle seems to be completely black. So we scale
        // brightness appropriately such that 0.0 maps to 1%.
        let max_duty = self.lcd_backlight.get_max_duty() as f32;
        let duty = ((0.99 * max_duty.powf(brightness)) + (0.01 * max_duty)) as u16;
        log::info!(
            "Setting LCD brightness to {} ({} / {})",
            brightness,
            duty,
            max_duty
        );
        self.lcd_backlight_duty = duty;
        if self.display_mode == DisplayMode::Internal {
            self.lcd_backlight.set_duty_cycle(duty).unwrap();
        }
    }

    /// Initialize the system time (after boot).
    ///
    /// Reads the time from the RTC. Sets a default time if no time is set.
    /// Then sets it in esp-idf (via libc settimeofday).
    fn init_datetime(&mut self) {
        if self.rtc.read_datetime().unwrap().is_none() {
            log::warn!("No date set, resetting");
            self.rtc
                .write_datetime(drivers::rtc::Datetime::default())
                .unwrap();
        }
        Device::set_esp_datetime(self.get_datetime());
    }

    /// Set the esp-idf system time.
    fn set_esp_datetime(dt: time::OffsetDateTime) {
        let timeval = esp_idf_svc::sys::timeval {
            tv_sec: dt.unix_timestamp(),
            tv_usec: 0,
        };
        unsafe {
            // SAFETY: this is safe to call with valid or null pointers.
            esp_idf_svc::sys::settimeofday(&timeval, std::ptr::null());
        }
    }

    /// Get the Device datetime.
    pub fn get_datetime(&mut self) -> time::OffsetDateTime {
        let rtc_time = self.rtc.read_datetime().unwrap();
        let ts = rtc_time
            .and_then(|dt| dt.as_timestamp())
            .unwrap_or(drivers::rtc::TIMESTAMP_2000);
        time::OffsetDateTime::from_unix_timestamp(ts as i64).unwrap()
    }

    /// Set the Device datetime.
    pub fn set_datetime(&mut self, dt: time::OffsetDateTime) {
        log::info!("Setting system time: {:?}", dt);
        Device::set_esp_datetime(dt);
        let ts = dt.unix_timestamp();
        let dt = drivers::rtc::Datetime::from_timestamp(ts as u64).unwrap_or_default();
        self.rtc.write_datetime(dt).unwrap();
    }

    /// Update the display mode (internal vs external).
    pub fn change_display_mode(&mut self, new_mode: DisplayMode) -> Result<(), anyhow::Error> {
        let old_mode = self.display_mode;
        if old_mode == new_mode {
            return Ok(());
        }
        log::info!("Display mode: {:?}", new_mode);

        if old_mode == DisplayMode::Internal {
            self.lcd_backlight.set_duty_cycle_fully_off().unwrap();
            self.lcd.enter_sleep()?;
        }

        self.fpga.set_display_mode(new_mode)?;

        if new_mode == DisplayMode::Internal {
            self.lcd.exit_sleep()?;

            // Let LCD stabilize and refresh before turning on backlight. Measured empirically.
            std::thread::sleep(Duration::from_millis(200));
            self.lcd_backlight
                .set_duty_cycle(self.lcd_backlight_duty)
                .unwrap();
        }

        self.display_mode = new_mode;
        Ok(())
    }

    pub fn get_display_mode(&self) -> DisplayMode {
        self.display_mode
    }

    pub fn get_battery_is_charging(&self) -> bool {
        self.pin_batt_chg.is_high()
    }

    pub fn get_vbus_pgood(&self) -> bool {
        self.pin_vbus_pgood.is_low() // Active low
    }
}
