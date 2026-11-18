//! Table mapping HID Keyboard/Keypad page (0x07) usages to names.
//!
//! Used for UI display and for recording macros/shortcuts (`KeyStroke::usage`, `MacroInput::Key`).
//! A keyboard driver may use `basic_group()` as the base when building its own
//! `KeyboardCaps::catalog` (if the code space is device-specific, convert the code before use).

use crate::model::{KeyChoice, KeyGroup, MediaFn, MouseFn};

/// (usage, list name, short name for keycaps)
pub const HID_KEYS: &[(u16, &str, &str)] = &[
    (0x04, "A", "A"),
    (0x05, "B", "B"),
    (0x06, "C", "C"),
    (0x07, "D", "D"),
    (0x08, "E", "E"),
    (0x09, "F", "F"),
    (0x0A, "G", "G"),
    (0x0B, "H", "H"),
    (0x0C, "I", "I"),
    (0x0D, "J", "J"),
    (0x0E, "K", "K"),
    (0x0F, "L", "L"),
    (0x10, "M", "M"),
    (0x11, "N", "N"),
    (0x12, "O", "O"),
    (0x13, "P", "P"),
    (0x14, "Q", "Q"),
    (0x15, "R", "R"),
    (0x16, "S", "S"),
    (0x17, "T", "T"),
    (0x18, "U", "U"),
    (0x19, "V", "V"),
    (0x1A, "W", "W"),
    (0x1B, "X", "X"),
    (0x1C, "Y", "Y"),
    (0x1D, "Z", "Z"),
    (0x1E, "1", "1"),
    (0x1F, "2", "2"),
    (0x20, "3", "3"),
    (0x21, "4", "4"),
    (0x22, "5", "5"),
    (0x23, "6", "6"),
    (0x24, "7", "7"),
    (0x25, "8", "8"),
    (0x26, "9", "9"),
    (0x27, "0", "0"),
    (0x28, "Enter", "Enter"),
    (0x29, "Esc", "Esc"),
    (0x2A, "Backspace", "BS"),
    (0x2B, "Tab", "Tab"),
    (0x2C, "Space", "Space"),
    (0x2D, "-", "-"),
    (0x2E, "=", "="),
    (0x2F, "[", "["),
    (0x30, "]", "]"),
    (0x31, "\\", "\\"),
    (0x33, ";", ";"),
    (0x34, "'", "'"),
    (0x35, "` (Hankaku/Zenkaku)", "`"),
    (0x36, ",", ","),
    (0x37, ".", "."),
    (0x38, "/", "/"),
    (0x39, "Caps Lock", "Caps"),
    (0x3A, "F1", "F1"),
    (0x3B, "F2", "F2"),
    (0x3C, "F3", "F3"),
    (0x3D, "F4", "F4"),
    (0x3E, "F5", "F5"),
    (0x3F, "F6", "F6"),
    (0x40, "F7", "F7"),
    (0x41, "F8", "F8"),
    (0x42, "F9", "F9"),
    (0x43, "F10", "F10"),
    (0x44, "F11", "F11"),
    (0x45, "F12", "F12"),
    (0x46, "Print Screen", "PrtSc"),
    (0x47, "Scroll Lock", "ScrLk"),
    (0x48, "Pause", "Pause"),
    (0x49, "Insert", "Ins"),
    (0x4A, "Home", "Home"),
    (0x4B, "Page Up", "PgUp"),
    (0x4C, "Delete", "Del"),
    (0x4D, "End", "End"),
    (0x4E, "Page Down", "PgDn"),
    (0x4F, "→", "→"),
    (0x50, "←", "←"),
    (0x51, "↓", "↓"),
    (0x52, "↑", "↑"),
    (0x53, "Num Lock", "NumLk"),
    (0x54, "Num /", "N/"),
    (0x55, "Num *", "N*"),
    (0x56, "Num -", "N-"),
    (0x57, "Num +", "N+"),
    (0x58, "Num Enter", "NEnt"),
    (0x59, "Num 1", "N1"),
    (0x5A, "Num 2", "N2"),
    (0x5B, "Num 3", "N3"),
    (0x5C, "Num 4", "N4"),
    (0x5D, "Num 5", "N5"),
    (0x5E, "Num 6", "N6"),
    (0x5F, "Num 7", "N7"),
    (0x60, "Num 8", "N8"),
    (0x61, "Num 9", "N9"),
    (0x62, "Num 0", "N0"),
    (0x63, "Num .", "N."),
    (0x64, "ISO \\", "ISO\\"),
    (0x65, "Menu", "Menu"),
    (0x68, "F13", "F13"),
    (0x69, "F14", "F14"),
    (0x6A, "F15", "F15"),
    (0x6B, "F16", "F16"),
    (0x6C, "F17", "F17"),
    (0x6D, "F18", "F18"),
    (0x6E, "F19", "F19"),
    (0x6F, "F20", "F20"),
    (0x70, "F21", "F21"),
    (0x71, "F22", "F22"),
    (0x72, "F23", "F23"),
    (0x73, "F24", "F24"),
    (0x87, "Ro (JIS \\_)", "Ro"),
    (0x88, "Kana", "Kana"),
    (0x89, "¥ (JIS)", "¥"),
    (0x8A, "Convert", "Conv"),
    (0x8B, "Non-convert", "NConv"),
    (0xE0, "Left Ctrl", "Ctrl"),
    (0xE1, "Left Shift", "Shift"),
    (0xE2, "Left Alt", "Alt"),
    (0xE3, "Left Win", "Win"),
    (0xE4, "Right Ctrl", "RCtrl"),
    (0xE5, "Right Shift", "RShift"),
    (0xE6, "Right Alt", "RAlt"),
    (0xE7, "Right Win", "RWin"),
];

/// usage → list name. `0xNN` if unknown.
pub fn hid_name(usage: u16) -> String {
    HID_KEYS
        .iter()
        .find(|k| k.0 == usage)
        .map(|k| k.1.to_string())
        .unwrap_or_else(|| format!("0x{usage:02X}"))
}

/// usage → short name for keycaps.
pub fn hid_short(usage: u16) -> String {
    HID_KEYS
        .iter()
        .find(|k| k.0 == usage)
        .map(|k| k.2.to_string())
        .unwrap_or_else(|| format!("{usage:02X}"))
}

/// Display of modifier bits ("Ctrl+Shift+").
pub fn mods_label(m: u8) -> String {
    use crate::model::mods::*;
    let mut s = String::new();
    if m & (LCTRL | RCTRL) != 0 {
        s.push_str("Ctrl+");
    }
    if m & (LSHIFT | RSHIFT) != 0 {
        s.push_str("Shift+");
    }
    if m & (LALT | RALT) != 0 {
        s.push_str("Alt+");
    }
    if m & (LGUI | RGUI) != 0 {
        s.push_str("Win+");
    }
    s
}

/// Modifier usage (0xE0..=0xE7) → modifier bit.
pub fn mod_bit(usage: u16) -> Option<u8> {
    if (0xE0..=0xE7).contains(&usage) {
        Some(1 << (usage - 0xE0))
    } else {
        None
    }
}

pub fn mouse_fn_name(m: MouseFn) -> &'static str {
    match m {
        MouseFn::Left => "Left click",
        MouseFn::Right => "Right click",
        MouseFn::Middle => "Middle click",
        MouseFn::Back => "Back",
        MouseFn::Forward => "Forward",
        MouseFn::ScrollUp => "Scroll up",
        MouseFn::ScrollDown => "Scroll down",
        MouseFn::ScrollLeft => "Scroll left",
        MouseFn::ScrollRight => "Scroll right",
        MouseFn::DoubleClick => "Double click",
    }
}

pub const MOUSE_FNS: &[MouseFn] = &[
    MouseFn::Left,
    MouseFn::Right,
    MouseFn::Middle,
    MouseFn::Back,
    MouseFn::Forward,
    MouseFn::ScrollUp,
    MouseFn::ScrollDown,
    MouseFn::ScrollLeft,
    MouseFn::ScrollRight,
    MouseFn::DoubleClick,
];

pub fn media_fn_name(m: MediaFn) -> &'static str {
    match m {
        MediaFn::PlayPause => "Play/Pause",
        MediaFn::Stop => "Stop",
        MediaFn::Next => "Next track",
        MediaFn::Prev => "Previous track",
        MediaFn::VolumeUp => "Volume +",
        MediaFn::VolumeDown => "Volume −",
        MediaFn::Mute => "Mute",
        MediaFn::Calculator => "Calculator",
        MediaFn::MyComputer => "My Computer",
        MediaFn::Browser => "Browser",
        MediaFn::Mail => "Mail",
        MediaFn::BrightnessUp => "Screen brightness +",
        MediaFn::BrightnessDown => "Screen brightness −",
        MediaFn::Refresh => "Refresh",
    }
}

pub const MEDIA_FNS: &[MediaFn] = &[
    MediaFn::PlayPause,
    MediaFn::Stop,
    MediaFn::Next,
    MediaFn::Prev,
    MediaFn::VolumeUp,
    MediaFn::VolumeDown,
    MediaFn::Mute,
    MediaFn::Calculator,
    MediaFn::MyComputer,
    MediaFn::Browser,
    MediaFn::Mail,
    MediaFn::BrightnessUp,
    MediaFn::BrightnessDown,
    MediaFn::Refresh,
];

/// Group of basic keys (code = HID usage). If a conversion to device-specific codes is needed,
/// the driver rewrites `code` before using it.
pub fn basic_group() -> KeyGroup {
    let mut seen = std::collections::BTreeSet::new();
    KeyGroup {
        name: "Basic keys".into(),
        keys: HID_KEYS
            .iter()
            .filter(|k| seen.insert(k.0))
            .map(|k| KeyChoice {
                code: k.0 as u32,
                label: k.1.into(),
                short: Some(k.2.into()),
            })
            .collect(),
    }
}
