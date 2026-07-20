use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::super::slint::Backend;
use crate::device::Device;
use crate::kvs;
use crate::ui::state::{SettingDatetime, SettingEntry, SettingType, SettingValue};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker};
use time::OffsetDateTime;

use super::UiState;

impl UiState {
    /// Set up the "Settings" screen.
    pub(super) fn setup_settings(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_setting_changed(move |i, value| {
            state_
                .borrow_mut()
                .settings_model
                .changed(i as usize, value);
        });
    }

    pub(super) fn on_settings_enter(&mut self) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        self.settings_model.refresh();
        backend.set_settings(ModelRc::from(self.settings_model.clone()));
    }
}

pub struct SettingsModel {
    notify: ModelNotify,
    datetime: Cell<OffsetDateTime>,
}

impl Model for SettingsModel {
    type Data = SettingEntry;

    fn row_count(&self) -> usize {
        7
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        match row {
            0 => Some(SettingEntry {
                name: "Dark mode".into(),
                r#type: SettingType::Checkbox,
                value: SettingValue {
                    bool_value: kvs::keys::DARK_MODE.get().unwrap(),
                    ..SettingValue::default()
                },
                ..Default::default()
            }),
            1 => Some(SettingEntry {
                name: "Date and Time (UTC)".into(),
                r#type: SettingType::Datetime,
                value: SettingValue {
                    datetime_value: {
                        let dt = self.datetime.get();
                        SettingDatetime {
                            year: dt.year(),
                            month: dt.month() as i32,
                            day: dt.day() as i32,
                            hour: dt.hour() as i32,
                            min: dt.minute() as i32,
                            sec: dt.second() as i32,
                        }
                    },
                    ..SettingValue::default()
                },
                ..Default::default()
            }),
            2 => Some(SettingEntry {
                name: "Rumble Strength".into(),
                r#type: SettingType::List,
                value: SettingValue {
                    int_value: kvs::keys::RUMBLE_LEVEL.get().unwrap(),
                    ..SettingValue::default()
                },
                choices: ["Off".into(), "Low".into(), "Medium".into(), "High".into()].into(),
            }),
            3 => Some(SettingEntry {
                name: "GB: Enable DMG mode".into(),
                r#type: SettingType::Checkbox,
                value: SettingValue {
                    bool_value: kvs::keys::GB_IS_DMG.get().unwrap(),
                    ..SettingValue::default()
                },
                ..Default::default()
            }),
            4 => Some(SettingEntry {
                name: "GB: Skip Boot Animation".into(),
                r#type: SettingType::Checkbox,
                value: SettingValue {
                    bool_value: kvs::keys::GB_SKIP_BOOT_ANIM.get().unwrap(),
                    ..SettingValue::default()
                },
                ..Default::default()
            }),
            5 => Some(SettingEntry {
                name: "GBA: Skip Boot Animation".into(),
                r#type: SettingType::Checkbox,
                value: SettingValue {
                    bool_value: kvs::keys::GBA_SKIP_BOOT_ANIM.get().unwrap(),
                    ..SettingValue::default()
                },
                ..Default::default()
            }),
            6 => Some(SettingEntry {
                name: "GBA: Enable Game Boy Player".into(),
                r#type: SettingType::Checkbox,
                value: SettingValue {
                    bool_value: kvs::keys::GBA_ENABLE_GBP.get().unwrap(),
                    ..SettingValue::default()
                },
                ..Default::default()
            }),
            _ => None,
        }
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn core::any::Any {
        // a typical implementation just return `self`
        self
    }
}

impl SettingsModel {
    pub fn new(device: &mut Device) -> Self {
        SettingsModel {
            notify: ModelNotify::default(),
            datetime: Cell::new(device.get_datetime()),
        }
    }

    pub fn refresh(&self) {
        self.datetime.set(Device::lock().get_datetime());
    }

    pub fn changed(&self, index: usize, value: SettingValue) {
        match index {
            0 => kvs::keys::DARK_MODE.set(&value.bool_value),
            1 => {
                let dt = convert_settings_datetime(&value.datetime_value).unwrap();
                let dt = dt.replace_second(0).unwrap();
                let dt = dt.assume_utc();
                Device::lock().set_datetime(dt);
                self.datetime.set(dt);
            }
            2 => kvs::keys::RUMBLE_LEVEL.set(&value.int_value),
            3 => kvs::keys::GB_IS_DMG.set(&value.bool_value),
            4 => kvs::keys::GB_SKIP_BOOT_ANIM.set(&value.bool_value),
            5 => kvs::keys::GBA_SKIP_BOOT_ANIM.set(&value.bool_value),
            6 => kvs::keys::GBA_ENABLE_GBP.set(&value.bool_value),
            _ => {
                log::info!("Unknown setting changed: {} -> {:?}", index, value);
                return;
            }
        }
        self.notify.row_changed(index);
    }
}

/// Helper function used by the UI to be able to correctly modify individual datetime components.
pub fn settings_datetime_add(source: SettingDatetime, delta: SettingDatetime) -> SettingDatetime {
    fn inner(
        source: &SettingDatetime,
        delta: SettingDatetime,
    ) -> time::Result<time::PrimitiveDateTime> {
        let mut dt = convert_settings_datetime(source)?;
        dt = dt.replace_day(1)?; // The day will be re-added later.
        dt = dt.replace_year(((dt.year() as i32) + delta.year).min(2100).max(2000))?;
        if delta.month < 0 {
            dt = dt.replace_month(dt.month().nth_prev((-delta.month) as u8))?;
        } else {
            dt = dt.replace_month(dt.month().nth_next(delta.month as u8))?;
        }
        dt = dt.replace_hour(((dt.hour() as i32) + delta.hour).rem_euclid(24) as u8)?;
        dt = dt.replace_minute(((dt.minute() as i32) + delta.min).rem_euclid(60) as u8)?;
        dt = dt.replace_second(((dt.second() as i32) + delta.sec).rem_euclid(60) as u8)?;
        let day_max = time::util::days_in_year_month(dt.year(), dt.month()) as i32;
        if delta.day == 0 {
            // If we aren't changing the day, clamp it to the maximum days in the month.
            dt = dt.replace_day(source.day.min(day_max) as u8)?;
        } else {
            dt = dt.replace_day((source.day + delta.day - 1).rem_euclid(day_max) as u8 + 1)?;
        }
        Ok(dt)
    }
    match inner(&source, delta) {
        Ok(dt) => SettingDatetime {
            year: dt.year(),
            month: dt.month() as i32,
            day: dt.day() as i32,
            hour: dt.hour() as i32,
            min: dt.minute() as i32,
            sec: dt.second() as i32,
        },
        Err(_) => {
            log::warn!("Invalid date");
            source
        }
    }
}

fn convert_settings_datetime(source: &SettingDatetime) -> time::Result<time::PrimitiveDateTime> {
    let date = time::Date::from_calendar_date(
        source.year,
        (source.month as u8).try_into()?,
        source.day as u8,
    )?;
    let time = time::Time::from_hms(source.hour as u8, source.min as u8, source.sec as u8)?;
    Ok(time::PrimitiveDateTime::new(date, time))
}
