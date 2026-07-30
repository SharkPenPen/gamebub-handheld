use crate::device::Device;
use log;
use std::thread;
use std::time::Duration;

/// 硬件焊接自检，返回 Ok(()) 表示所有关键项通过
pub fn run_tests(device: &mut Device) -> anyhow::Result<()> {
    log::info!("=== Hardware Self-Test ===");

    let mut failures = 0u32;

    // ── 电源检测 ──
    {
        let vbus = device.get_vbus_pgood();
        if vbus {
            log::info!("[PWR] VBUS present: OK");
        } else {
            log::warn!(
                "[PWR] VBUS not present. Check: USB connector, VBUS power path, VBUS detect (GPIO{})",
                extract_gpio("VBUS pgood", &device.pin_vbus_pgood)
            );
            failures += 1;
        }

        let charging = device.get_battery_is_charging();
        if charging {
            log::info!("[PWR] Battery is charging: OK");
        } else {
            log::warn!("[PWR] Battery not charging (may be full or absent)");
        }

        match device.fuel_gauge.get_battery_voltage() {
            Ok(v) => log::info!("[PWR] Battery voltage: {:.2} V", v),
            Err(e) => {
                log::warn!(
                    "[PWR] Failed to read battery voltage: {}. Check: Fuel gauge U? (MAX17048, addr 0x36), I2C lines",
                    e
                );
                failures += 1;
            }
        }
    }

    // ── I2C 总线扫描 ──
    {
        let i2c_lock = device.i2c.lock().unwrap();
        let to_test = [
            (0x18, "DAC (TLV320DAC3101)"),
            (0x20, "IO Expander (TCA9535)"),
            (0x36, "Fuel Gauge (MAX17048)"),
            (0x51, "RTC (PCF8563)"),
            (0x6A, "IMU (LSM6DS3TRC)"),
        ];
        for (addr, name) in &to_test {
            match i2c_lock.write(*addr, &[]) {
                Ok(()) => log::info!("[I2C] 0x{:02X} ({}): OK", addr, name),
                Err(_) => {
                    log::warn!(
                        "[I2C] 0x{:02X} ({}): FAIL - no ACK. Check: {} chip, I2C lines \
                         (SDA/SCL), pull-up resistors",
                        addr, name, name
                    );
                    failures += 1;
                }
            }
        }
    }

    // ── 按键 ──
    {
        let home = device.button_home.is_high();
        let vol_up = device.button_vol_up.is_high();
        let vol_down = device.button_vol_down.is_high();
        let power = device.button_power.is_high();
        if home && vol_up && vol_down && power {
            log::info!("[BTN] Home, Vol+, Vol-, Power: all high (released)");
        } else {
            log::warn!(
                "[BTN] One or more buttons low. Check: buttons near GPIOs: \
                 Home(GPIO{}), Vol+(GPIO{}), Vol-(GPIO{}), Power(GPIO{})",
                extract_gpio("Home", &device.button_home),
                extract_gpio("VolUp", &device.button_vol_up),
                extract_gpio("VolDown", &device.button_vol_down),
                extract_gpio("Power", &device.button_power)
            );
            failures += 1;
        }
    }

    // ── IO 扩展器 ──
    {
        match device.io_expander.get_pins() {
            Ok(pins) => log::info!("[IOEXP] Pins: {:?}", pins),
            Err(e) => {
                log::warn!(
                    "[IOEXP] Failed to read IO expander: {}. Check: TCA9535 (addr 0x20), I2C",
                    e
                );
                failures += 1;
            }
        }
    }

    // ── FPGA 配置引脚静态电平 ──
    {
        match device.fpga.get_init_b() {
            Ok(true) => log::info!("[FPGA] INIT_B: HIGH (initial state OK)"),
            Ok(false) => {
                log::warn!("[FPGA] INIT_B: LOW (unexpected). Check: FPGA pin INIT_B (GPIO8), FPGA power");
                failures += 1;
            }
            Err(e) => {
                log::warn!("[FPGA] Cannot read INIT_B: {}. Check: GPIO8 (INIT_B)", e);
                failures += 1;
            }
        }

        match device.fpga.get_done() {
            Ok(false) => log::info!("[FPGA] DONE: LOW (correct, not programmed)"),
            Ok(true) => {
                log::warn!("[FPGA] DONE: HIGH (unexpected). Check: FPGA pin DONE (GPIO17 rev1 / GPIO6 rev2)");
                failures += 1;
            }
            Err(e) => {
                log::warn!("[FPGA] Cannot read DONE: {}. Check: GPIO for DONE", e);
                failures += 1;
            }
        }
    }

    // ── FPGA PROGRAM_B 脉冲测试 ──
    {
        log::info!("[FPGA] Pulsing PROGRAM_B...");
        if let Err(e) = device.fpga.set_program_b(false) {
            log::warn!(
                "[FPGA] Cannot set PROGRAM_B low: {}. Check: GPIO for PROGRAM_B (GPIO18 rev1 / GPIO7 rev2)",
                e
            );
            failures += 1;
        } else {
            thread::sleep(Duration::from_micros(10));
            match device.fpga.get_init_b() {
                Ok(false) => log::info!("[FPGA] INIT_B went low (correct)"),
                Ok(true) => {
                    log::warn!("[FPGA] INIT_B did NOT go low when PROGRAM_B asserted. Check: FPGA connections, pull-ups");
                    failures += 1;
                }
                Err(e) => {
                    log::warn!("[FPGA] Cannot read INIT_B after PROGRAM_B low: {}", e);
                    failures += 1;
                }
            }
            if let Err(e) = device.fpga.set_program_b(true) {
                log::warn!("[FPGA] Cannot set PROGRAM_B high: {}", e);
                failures += 1;
            } else {
                thread::sleep(Duration::from_millis(6));
                match device.fpga.get_init_b() {
                    Ok(true) => log::info!("[FPGA] INIT_B went high (ready for config)"),
                    Ok(false) => {
                        log::warn!("[FPGA] INIT_B did NOT go high after release. Check: FPGA power, config mode pins, pull-ups");
                        failures += 1;
                    }
                    Err(e) => {
                        log::warn!("[FPGA] Cannot read INIT_B after release: {}", e);
                        failures += 1;
                    }
                }
            }
        }
    }

    // ── SD 卡 ──
    {
        if device.sdcard.is_some() {
            log::info!("[SD] SD card mounted: OK");
        } else {
            log::warn!(
                "[SD] SD card not mounted. Check: SD socket J?, detect GPIO{}, SDIO lines, pull-ups",
                // 此处 GPIO 编号可从 SD detect 引脚获取，若不可得则输出占位符
                "?"
            );
            failures += 1;
        }
    }

    // ── RTC ──
    {
        match device.rtc.read_datetime() {
            Ok(Some(dt)) => log::info!(
                "[RTC] Time: {:02}:{:02}:{:02} (W{}) {}/{}/{}",
                dt.hours, dt.minutes, dt.seconds, dt.weekdays, dt.months, dt.days, dt.years
            ),
            Ok(None) => {
                log::warn!("[RTC] RTC has invalid time (VL flag set). Check: RTC (PCF8563, addr 0x51), backup battery, I2C");
                failures += 1;
            }
            Err(e) => {
                log::warn!("[RTC] Failed to read RTC: {}. Check: RTC (PCF8563, addr 0x51), I2C lines", e);
                failures += 1;
            }
        }
    }

    // ── IMU ──
    {
        match device.imu.read_accel() {
            Ok(sample) => log::info!(
                "[IMU] Accel: X={:.3} Y={:.3} Z={:.3} g",
                sample.x, sample.y, sample.z
            ),
            Err(e) => {
                log::warn!(
                    "[IMU] Failed to read accelerometer: {}. Check: IMU (LSM6DS3TRC, addr 0x6A), I2C",
                    e
                );
                failures += 1;
            }
        }
    }

    // ── DAC 中断状态 ──
    {
        match device.dac.get_interrupt_status() {
            Ok(st) => log::info!("[DAC] Interrupts: {:?}", st),
            Err(e) => {
                log::warn!(
                    "[DAC] Failed to read DAC interrupts: {}. Check: DAC (TLV320DAC3101, addr 0x18), I2C",
                    e
                );
                failures += 1;
            }
        }
    }

    log::info!("=== Self-test: {} failures ===", failures);

    if failures > 0 {
        anyhow::bail!(
            "Hardware self-test failed with {} error(s). Check logs for details.",
            failures
        );
    }

    Ok(())
}

/// 从 PinDriver 中提取 GPIO 编号
fn extract_gpio<P: esp_idf_svc::hal::gpio::Pin>(_label: &str, pin: &P) -> i32 {
    pin.pin() as i32
}