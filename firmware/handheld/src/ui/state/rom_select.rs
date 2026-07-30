use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use super::super::slint::Backend;
use super::super::slint::FileIcon;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{device::Device, kvs, worker};

use super::UiState;

pub const BASE_DIR: &str = "/sdcard/";

impl UiState {
    /// Set up the "Rom Select" screen.
    pub(super) fn setup_rom_select(&mut self, state: &Rc<RefCell<UiState>>, _device: &mut Device) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();

        let state_ = state.clone();
        backend.on_rom_select_selected(move |index| {
            let mut state = state_.borrow_mut();
            let root = state.root.unwrap();
            let backend = root.global::<Backend>();
            let list = backend.get_rom_select_list();
            if let Some(data) = list.row_data(index as usize) {
                let path = state.rom_select_directory.join(data.name.as_str());
                state.rom_select_handle_select(path, data.name.as_str())
            } else {
                false
            }
        });

        let state_ = state.clone();
        backend.on_rom_select_up(move || {
            let mut state = state_.borrow_mut();
            state.rom_select_handle_select(PathBuf::new(), "..")
        });

        let state_ = state.clone();
        backend.on_rom_select_focused(move |index| {
            let state = state_.borrow_mut();
            let root = state.root.unwrap();
            let backend = root.global::<Backend>();
            let list = backend.get_rom_select_list();
            if let Some(data) = list.row_data(index as usize) {
                let path = state.rom_select_directory.join(data.name.as_str());
                kvs::keys::LAST_ROM_PATH.set(&path);
            }
        });
    }

    pub fn rom_select_update_list(&mut self, mut files: Vec<(String, bool)>) {
        let path = &self.rom_select_directory;
        if path != Path::new(BASE_DIR) {
            files.insert(0, ("..".to_string(), true));
        }

        // Determine initial selected file.
        let last_path = kvs::keys::LAST_ROM_PATH.get();
        let selected = last_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|f| f.to_str())
            .and_then(|filename| files.iter().position(|(f, _)| f == filename))
            .unwrap_or(0);
        let files = ModelRc::from(Rc::new(VecModel::from(
            files
                .into_iter()
                .map(|(name, is_dir)| crate::ui::slint::FileListEntry {
                    name: name.into(),
                    icon: if is_dir {
                        FileIcon::Folder
                    } else {
                        FileIcon::Blank
                    },
                })
                .collect::<Vec<_>>(),
        )));

        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_rom_select_list(files);
        backend.set_rom_select_index(selected as i32);
        backend.set_rom_select_is_loading(false);
        self.rom_select_update_path();
    }

    pub fn rom_select_update_path(&self) {
        // Remove base directory from name before displaying.
        let path = &self.rom_select_directory;
        let mut directory = path
            .strip_prefix(BASE_DIR)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        if !directory.starts_with("/") {
            directory.insert_str(0, "/");
        }

        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_rom_select_path(directory.into());
    }

    /// Handle selection. Returns whether a loading screen should be displayed.
    fn rom_select_handle_select(&mut self, path: PathBuf, filename: &str) -> bool {
        if filename == ".." {
            if self.rom_select_directory == Path::new(BASE_DIR) {
                log::warn!("No parent directory");
                return false;
            } else {
                kvs::keys::LAST_ROM_PATH.set(&self.rom_select_directory);
                self.rom_select_directory.pop();
                worker::send(worker::Message::ListRoms(self.rom_select_directory.clone()));
            }
        } else if path.is_dir() {
            log::info!("Entering subdirectory {}", filename);
            self.rom_select_directory.push(filename);
            let last_path = self.rom_select_directory.join("..");
            kvs::keys::LAST_ROM_PATH.set(&last_path);
            worker::send(worker::Message::ListRoms(self.rom_select_directory.clone()));
        } else {
            log::info!("Selected ROM {}", path.display());
            kvs::keys::LAST_ROM_PATH.set(&path);
            worker::send(worker::Message::RunRomFile(path));
        }
        self.root
            .unwrap()
            .global::<Backend>()
            .set_rom_select_progress(0.0);
        true
    }

    pub fn rom_select_set_error(&mut self, error: String) {
        let root = self.root.unwrap();
        let backend = root.global::<Backend>();
        backend.set_rom_select_is_loading(false);
        backend.set_rom_select_error(error.into());
        self.rom_select_update_path();
    }
}
