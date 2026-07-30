use std::path::PathBuf;

use super::KvsKey;

/// The device hardware revision
pub static DEVICE_REVISION: KvsKey<u32> = KvsKey::new_ro("revision");

/// Serial number
pub static DEVICE_SERIAL: KvsKey<String> = KvsKey::new_ro("serial");

/// The full path of the last selected ROM.
pub static LAST_ROM_PATH: KvsKey<PathBuf> = KvsKey::new("last-rom-path");

/// The last volume level.
pub static VOLUME: KvsKey<u8> = KvsKey::new_with_default("volume", 128);

/// The last brightness level.
pub static BRIGHTNESS: KvsKey<f32> = KvsKey::new_with_default("brightness", 0.50);

/// Whether dark mode is enabled.
pub static DARK_MODE: KvsKey<bool> = KvsKey::new_with_default("dark-mode", false);

/// Whether to use DMG mode (instead of CGB mode)
pub static GB_IS_DMG: KvsKey<bool> = KvsKey::new_with_default("gb-is-dmg", false);

/// Whether to skip DMG/CGB boot animation
pub static GB_SKIP_BOOT_ANIM: KvsKey<bool> = KvsKey::new_with_default("gb-no-anim", false);

/// Whether to skip GBA boot animation.
pub static GBA_SKIP_BOOT_ANIM: KvsKey<bool> = KvsKey::new_with_default("gba-no-anim", false);

/// Whether to enable Game Boy Player functiona.lity
pub static GBA_ENABLE_GBP: KvsKey<bool> = KvsKey::new_with_default("gba-enable-gbp", false);

/// Rumble strength level
pub static RUMBLE_LEVEL: KvsKey<i32> = KvsKey::new_with_default("rumble-level", 0);

pub fn flush_all() {
    LAST_ROM_PATH.flush();
    VOLUME.flush();
    BRIGHTNESS.flush();
    DARK_MODE.flush();
    GB_IS_DMG.flush();
    GB_SKIP_BOOT_ANIM.flush();
    GBA_SKIP_BOOT_ANIM.flush();
    GBA_ENABLE_GBP.flush();
    RUMBLE_LEVEL.flush()
}
