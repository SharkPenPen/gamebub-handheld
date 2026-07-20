use esp_idf_svc::sys::{self as esp_idf_sys, EspError};

/// Set up and install the TinyUSB driver.
///
/// This takes over the USB PHY, removing the Serial/JTAG built-in device,
/// and instead exposes the MSC device.
pub fn setup_tinyusb() -> Result<(), EspError> {
    // TODO: fill in descriptors
    // TODO: support combined serial CDC device
    let tinyusb_config = esp_idf_sys::tinyusb_config_t {
        string_descriptor: std::ptr::null_mut(),
        string_descriptor_count: 0,
        __bindgen_anon_1: esp_idf_sys::tinyusb_config_t__bindgen_ty_1 {
            device_descriptor: std::ptr::null_mut(),
        },
        __bindgen_anon_2: esp_idf_sys::tinyusb_config_t__bindgen_ty_2 {
            __bindgen_anon_1: esp_idf_sys::tinyusb_config_t__bindgen_ty_2__bindgen_ty_1 {
                configuration_descriptor: std::ptr::null_mut(),
            },
        },
        external_phy: false,
        // TODO: handle self-powered VBUS monitoring
        self_powered: false,
        vbus_monitor_io: 0,
    };
    log::info!("Installing TinyUSB");
    let result = unsafe { esp_idf_sys::tinyusb_driver_install(&tinyusb_config) };
    EspError::convert(result)
}

/// Tear down / uninstall the TinyUSB driver, re-exposing to Serial/JTAG USB device.
pub fn teardown_tinyusb() -> Result<(), EspError> {
    let result = unsafe { esp_idf_sys::tinyusb_driver_uninstall() };
    EspError::convert(result)?;

    // Re-initialize Serial/JTAG.
    let phy_config = esp_idf_sys::usb_phy_config_t {
        controller: esp_idf_sys::usb_phy_controller_t_USB_PHY_CTRL_SERIAL_JTAG,
        target: esp_idf_sys::usb_phy_target_t_USB_PHY_TARGET_INT,
        otg_mode: 0,
        otg_speed: 0,
        ext_io_conf: std::ptr::null(),
        otg_io_conf: std::ptr::null(),
    };
    let mut jtag_phy: esp_idf_sys::usb_phy_handle_t = std::ptr::null_mut();
    let result = unsafe { esp_idf_sys::usb_new_phy(&phy_config, &mut jtag_phy) };
    EspError::convert(result)?;

    log::info!("Re-initialized Serial/JTAG USB device");
    Ok(())
}
