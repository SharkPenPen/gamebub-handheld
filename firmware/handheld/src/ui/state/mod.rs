use std::{cell::RefCell, path::Path, path::PathBuf, rc::Rc, time::Duration};

use slint::{ComponentHandle, Global, Timer, TimerMode, Weak};

use crate::device::Device;
use crate::kvs;

use super::slint::{
    Backend, MainWindow, ScreenId, SettingDatetime, SettingEntry, SettingType, SettingValue,
};

mod game;
mod main_menu;
mod rom_select;
mod settings;
mod tools;

pub struct UiState {
    root: Weak<MainWindow>,

    rom_select_directory: PathBuf,
    settings_model: Rc<settings::SettingsModel>,
}

impl UiState {
    pub fn new(root: &MainWindow, device: &mut Device) -> Rc<RefCell<Self>> {
        let rom_select_directory = kvs::keys::LAST_ROM_PATH
            .get()
            .map(|mut p| {
                p.pop();
                p
            })
            .unwrap_or_else(|| Path::new(rom_select::BASE_DIR).to_path_buf());
        let state: UiState = UiState {
            root: root.as_weak(),
            settings_model: Rc::new(settings::SettingsModel::new(device)),
            rom_select_directory,
        };
        let state = Rc::new(RefCell::new(state));
        state.borrow_mut().setup(state.clone(), device);
        state
    }

    pub fn update_battery_level(&mut self, level: f32) {
        let level = level.round() as i32;
        log::info!("Battery level: {:?}%", level);
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_battery_level(level);
    }

    fn setup(&mut self, state: Rc<RefCell<UiState>>, device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let battery_level = device.fuel_gauge.get_battery_level().unwrap_or(0.0);
        self.update_battery_level(battery_level);
        backend.set_volume_level(((kvs::keys::VOLUME.get().unwrap() as i32) * 100) / 255);
        backend.set_brightness_level((kvs::keys::BRIGHTNESS.get().unwrap() * 100.0) as i32);
        backend.set_device_revision(kvs::keys::DEVICE_REVISION.get().unwrap() as i32);
        backend.set_device_serial(kvs::keys::DEVICE_SERIAL.get().unwrap_or_default().into());

        self.setup_main_menu(&state, device);
        self.setup_game(&state, device);
        self.setup_tools(&state, device);
        self.setup_rom_select(&state, device);
        self.setup_settings(&state, device);

        let state_ = state.clone();
        backend.on_screen_enter(move |screen| {
            let mut state = state_.borrow_mut();
            // Called when a new screen is entered, before the new frame is rendered.
            log::info!("Screen enter: {:?}", screen);
            match screen {
                ScreenId::Settings => state.on_settings_enter(),
                _ => {}
            }
        });

        backend.on_volume_changed({
            let state_ = state.clone();
            let timer = Timer::default();
            move |value| {
                let volume = ((value * (u8::MAX as i32)) / 100) as u8;
                Device::lock().dac.set_volume(volume).unwrap();
                // Start the timer to hide the bar.
                let state_ = state_.clone();
                timer.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(1000),
                    move || {
                        kvs::keys::VOLUME.set(&volume);
                        Backend::get(&state_.borrow().root.unwrap()).set_volume_visible(false);
                    },
                )
            }
        });

        backend.on_brightness_changed({
            let state_ = state.clone();
            let timer = Timer::default();
            move |value| {
                let brightness = (value as f32) / 100.0;
                Device::lock().set_brightness(brightness);
                // Start the timer to hide the bar.
                let state_ = state_.clone();
                timer.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(1000),
                    move || {
                        kvs::keys::BRIGHTNESS.set(&brightness);
                        Backend::get(&state_.borrow().root.unwrap()).set_brightness_visible(false);
                    },
                )
            }
        });

        backend.on_power_off(|| {
            Device::lock().power_off();
        });

        backend.on_reboot(|| {
            Device::lock().reboot();
        });

        // Utility function: add datetime with delta
        backend.on_datetime_add(settings::settings_datetime_add);
    }
}
