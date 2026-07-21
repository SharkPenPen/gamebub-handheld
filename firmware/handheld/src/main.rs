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

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    esp_idf_svc::log::set_target_level("gpio", log::LevelFilter::Warn).unwrap();

    kvs::Kvs::init()?;
    log::info!("Device revision {:?}", kvs::keys::DEVICE_REVISION.get());
    log::info!("Serial: {:?}", kvs::keys::DEVICE_SERIAL.get());

    // Check that the firmware is compatible with the listed revision.
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

    // Proceed to initialize device.
    Device::init()?;
    let mut device = Device::lock();
    let is_sdcard_mounted = device.sdcard.is_some();
    device.set_brightness(kvs::keys::BRIGHTNESS.get().unwrap());

    // Setup workers.
    worker::start();

    // Initial programming FPGA
    fn program_fpga(device: &mut Device) -> anyhow::Result<()> {
        let bitstream =
            util::open_system_file("boot.bit.gz").context("Failed to read bitstream")?;
        let mut bitstream = GzDecoder::new(bitstream);
        device
            .fpga
            .program(&mut bitstream)
            .context("Failed to program FPGA")?;
        device.lcd.enable_fpga_control()?;
        Ok(())
    }
    let fpga_program_result = program_fpga(&mut device);

    // Setup UI
    let mut ui = UI::new(&mut device);
    std::mem::drop(device);

    // If there was an error programming the FPGA, show it now.
    if !is_sdcard_mounted {
        show_fatal_error("Error mounting SD card".into());
    } else if let Err(error) = fpga_program_result {
        show_fatal_error_anyhow(error);
    }

    // Run UI in this thread.
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
