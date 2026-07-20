use super::Device;
use crate::ui::buttons::{Button, ButtonMap};
use enum_map::enum_map;

impl Device<'_> {
    /// Get the current state of the buttons.
    pub fn read_button_state(&mut self, io_expander: [bool; 16]) -> Result<ButtonMap, ()> {
        Ok(enum_map! {
         Button::A => !io_expander[3],
         Button::B => !io_expander[4],
         Button::X => !io_expander[1],
         Button::Y => !io_expander[2],
         Button::Up => !io_expander[10],
         Button::Down => !io_expander[13],
         Button::Left => !io_expander[12],
         Button::Right => !io_expander[11],
         Button::Start => !io_expander[15],
         Button::Select => !io_expander[14],
         Button::L => !io_expander[9],
         Button::R => !io_expander[0],
         Button::Home => self.button_home.is_low(),
         Button::VolUp => self.button_vol_up.is_low(),
         Button::VolDown => self.button_vol_down.is_low(),
         Button::Power => self.button_power.is_low(),
        })
    }

    /// Get whether an HDMI cable is plugged in based on IO expander state
    pub(super) fn parse_hdmi_detect(&mut self, io_expander: [bool; 16]) -> Result<bool, ()> {
        cfg_if::cfg_if! {
            if #[cfg(feature = "rev1")] {
                // Rev 1: HDMI hot plug detect is active-low.
                Ok(!io_expander[5])
            } else if #[cfg(feature = "rev2")] {
                // Rev 2: HDMI hot plug detect is not in I/O expander.
                let _ = io_expander;
                Ok(false)
            }
        }
    }

    pub fn read_hdmi_detect(&mut self) -> Result<bool, ()> {
        let io_expander = self.io_expander.get_pins().map_err(|_| ())?;
        self.parse_hdmi_detect(io_expander)
    }
}
