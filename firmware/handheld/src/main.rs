use anyhow::Context;
use flate2::read::GzDecoder;

use device::Device;

use crate::ui::UI;

mod bitstream;
mod device;
mod kvs;
pub mod ui;
mod util;
mod worker;

// ==========================================
// 【诊断扫描与延时模块】
// ==========================================
fn diagnose_hardware_before_init() {
    use esp_idf_hal::delay::FreeRtos;
    use esp_idf_hal::gpio::{Input, PinDriver, Pull};
    use esp_idf_hal::i2c::*;
    use esp_idf_hal::peripherals::Peripherals;

    log::info!("[DIAG-1] 强制等待 1秒，让 1.8V 电源轨完全稳定...");
    FreeRtos::delay_ms(1000);

    log::info!("[DIAG-2] 开始扫描 I2C 总线上的所有设备...");
    let peripherals = Peripherals::take().unwrap();
    let i2c = peripherals.i2c0;
    let sda = peripherals.pins.gpio8;
    let scl = peripherals.pins.gpio9;
    let i2c_driver = I2cDriver::new(i2c, sda, scl, I2cConfig::new().baudrate(100.kHz().into())).unwrap();

    let mut found_dac = false;
    for addr in 1..127 {
        match i2c_driver.write_read(addr as u8, &[0x00], &mut [0u8; 1]) {
            Ok(_) => {
                if addr == 0x18 { found_dac = true; }
                log::info!("[I2C] 发现设备 -> 地址: 0x{:02X}", addr);
            }
            Err(_) => continue,
        }
    }

    if found_dac {
        log::info!("[DIAG-3] ✅ 检测结果：I2C 总线成功连接到 DAC 芯片 (0x18)！");
    } else {
        log::error!("[DIAG-3] ❌ 检测结果：I2C 总线**扫描不到** DAC 芯片 (0x18)！");
        log::error!("   -> 重点排查：DAC 的 3脚/17脚 是否真的收到了 1.8V？");
        log::error!("   -> 重点排查：DAC 底部的散热焊盘（EP）是否虚焊、悬空？");
    }

    match PinDriver::new(peripherals.pins.gpio0, Input) {
        Ok(mut done_pin) => {
            done_pin.set_pull(Pull::None).unwrap();
            if done_pin.is_high() {
                log::info!("[DIAG-4] ✅ FPGA DONE 信号为高电平 (3.3V)，FPGA 配置成功。");
            } else {
                log::error!("[DIAG-4] ❌ FPGA DONE 信号为低电平 (0V)，FPGA 没加载成功！");
            }
        }
        Err(e) => log::error!("[DIAG-4] 无法读取 FPGA DONE 状态: {:?}", e),
    }
}
// ==========================================

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    esp_idf_svc::log::set_target_level("gpio", log::LevelFilter::Warn).unwrap();

    kvs::Kvs::init()?;
    log::info!("Device revision {:?}", kvs::keys::DEVICE_REVISION.get());
    log::info!("Serial: {:?}", kvs::keys::DEVICE_SERIAL.get());

    cfg_if::cfg_if! {
        if #[cfg(feature = "rev1")] {
            let required_revision = 1;
        } else if #[cfg(feature = "rev2")] {
            let required_revision = 2;
        } else {
            compile_error!("No board revision selected");
        }
    };
    if kvs::keys::DEVICE_REVISION.get() != Some(required_revision) {
        anyhow::bail!(
            "Incompatible firmware revision: device={:?} firmware={}",
            kvs::keys::DEVICE_REVISION.get(),
            required_revision
        );
    }

    log::info!("--- 开启【硬件健康扫描】模式 ---");
    diagnose_hardware_before_init();

    log::info!("--- 尝试进入原作者的标准初始化流程 ---");
    match Device::init() {
        Ok(_) => log::info!("标准初始化流程正常结束！"),
        Err(e) => {
            log::error!("标准初始化遇到错误: {:?}", e);
            log::error!("注意：如果前面诊断已经扫描到 0x18，而这里爆错，那就是 DAC 固件时序或 ID 验证的问题！");
        }
    }

    let mut device = Device::lock();
    let is_sdcard_mounted = device.sdcard.is_some();
    device.set_brightness(kvs::keys::BRIGHTNESS.get().unwrap());

    worker::start();

    fn program_fpga(device: &mut Device) -> anyhow::Result<()> {
        let bitstream = util::open_system_file("boot.bit.gz").context("Failed to read bitstream")?;
        let mut bitstream = GzDecoder::new(bitstream);
        device.fpga.program(&mut bitstream).context("Failed to program FPGA")?;
        device.lcd.enable_fpga_control()?;
        Ok(())
    }
    let fpga_program_result = program_fpga(&mut device);

    let mut ui = UI::new(&mut device);
    std::mem::drop(device);

    if !is_sdcard_mounted {
        show_fatal_error("Error mounting SD card".into());
    } else if let Err(error) = fpga_program_result {
        show_fatal_error_anyhow(error);
    }

    ui.run();
}

fn show_fatal_error_anyhow(error: anyhow::Error) {
    use std::fmt::Write;
    let mut message = format!("{}\n", error);
    for e in error.chain().skip(1) {
        let _ = write!(message, "\n{}", e);
    }
    show_fatal_error(message);
}

fn show_fatal_error(error: String) {
    Device::lock().lcd.enable_mcu_control().unwrap();
    ui::send(ui::Message::FatalError(error));
    ui::send(ui::Message::Redraw);
}
