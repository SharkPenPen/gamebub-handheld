use std::{cell::RefCell, ops::DerefMut, rc::Rc, time::Duration};

use super::super::slint::Backend;
use slint::{ComponentHandle, Timer};

use crate::{
    bitstream::{self, Bitstream, CurrentBitstream},
    device::Device,
    worker,
};

use super::UiState;

impl UiState {
    /// Set up the "Game" screen.
    pub(super) fn setup_game(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_game_set_paused(move |paused| {
            let needs_persist = match bitstream::current().deref_mut() {
                CurrentBitstream::None => false,
                CurrentBitstream::Gameboy(x) => {
                    x.set_paused(paused).unwrap();
                    x.needs_save_persist()
                }
                CurrentBitstream::Gba(x) => {
                    x.set_paused(paused).unwrap();
                    x.needs_save_persist()
                }
            };
            if paused && needs_persist {
                let state = state_.borrow_mut();
                let root = state.root.unwrap();
                let backend = root.global::<Backend>();
                backend.set_status_is_saving(true);
                worker::send(worker::Message::SaveGame);
            }
        });

        backend.on_game_reset(move || match bitstream::current().deref_mut() {
            CurrentBitstream::None => {}
            CurrentBitstream::Gameboy(x) => x.reset().unwrap(),
            CurrentBitstream::Gba(x) => x.reset().unwrap(),
        });
    }

    pub fn game_on_saved(&mut self) {
        // Ensure that the save icon is shown for a visible amount of time.
        let window = self.root.upgrade().unwrap();
        Timer::single_shot(Duration::from_millis(1000), move || {
            window.global::<Backend>().set_status_is_saving(false);
        });
    }
}
