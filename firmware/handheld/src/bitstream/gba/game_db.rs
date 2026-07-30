use super::{EmulatedCartridgeConfig, SaveType};

macro_rules! config {
    ($save_type:ident $(, $field:ident)*) => {
        {
            #[allow(unused_mut)]
            let mut config = EmulatedCartridgeConfig::from_save_type(SaveType::$save_type);
            $(
                config.$field = true;
            )*
            config
        }
    }
}

/// The game database.
static DATABASE: &[(&'static [u8; 4], EmulatedCartridgeConfig)] = &[
    (b"ALUE", config!(Flash128K, has_rtc)), // Pokemon - Ruby Version (USA, Europe)
    (b"AXPE", config!(Flash128K, has_rtc)), // Pokemon - Sapphire Version (USA, Europe)
    (b"BPEE", config!(Flash128K, has_rtc)), // Pokemon - Emerald Version (USA, Europe)
    (b"V49E", config!(Sram, has_rumble)),   // Drill Dozer (USA)
    (b"RZWE", config!(Sram, has_rumble, has_gyro)), // Wario Ware Twisted
    (b"KYGE", config!(EepromAuto, has_accel)), // Yoshi Topsy-Turvy
    (b"KHPJ", config!(EepromAuto, has_accel)), // Korokoro Puzzle - Happy Panecchu!
    (b"FSME", config!(Eeprom512)),          // Classic NES: Super Mario Bros.
    (b"2ATE", config!(Sram, has_rumble)),   // Apotris
    (b"2GBP", config!(Sram, has_rumble)),   // Goodboy Galaxy
];

pub fn lookup(key: &[u8; 4]) -> Option<EmulatedCartridgeConfig> {
    DATABASE
        .iter()
        .find(|(&code, _)| *key == code)
        .map(|(_, config)| config.clone())
}
