//! GPUI key names (`Keystroke::key`) → HID usage. For macro recording and shortcut capture.

use devhub_core::mods;
use gpui_kit::Modifiers;

pub fn usage_of(key: &str) -> Option<u16> {
    let k = key.to_ascii_lowercase();
    if k.len() == 1 {
        let c = k.as_bytes()[0];
        return match c {
            b'a'..=b'z' => Some(0x04 + (c - b'a') as u16),
            b'1'..=b'9' => Some(0x1E + (c - b'1') as u16),
            b'0' => Some(0x27),
            b'-' => Some(0x2D),
            b'=' => Some(0x2E),
            b'[' => Some(0x2F),
            b']' => Some(0x30),
            b'\\' => Some(0x31),
            b';' => Some(0x33),
            b'\'' => Some(0x34),
            b'`' => Some(0x35),
            b',' => Some(0x36),
            b'.' => Some(0x37),
            b'/' => Some(0x38),
            b' ' => Some(0x2C),
            _ => None,
        };
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        return match n {
            1..=12 => Some(0x3A + n - 1),
            13..=24 => Some(0x68 + n - 13),
            _ => None,
        };
    }
    Some(match k.as_str() {
        "enter" => 0x28,
        "escape" => 0x29,
        "backspace" => 0x2A,
        "tab" => 0x2B,
        "space" => 0x2C,
        "capslock" | "caps_lock" | "caps-lock" => 0x39,
        "printscreen" | "print" => 0x46,
        "scrolllock" => 0x47,
        "pause" => 0x48,
        "insert" => 0x49,
        "home" => 0x4A,
        "pageup" => 0x4B,
        "delete" => 0x4C,
        "end" => 0x4D,
        "pagedown" => 0x4E,
        "right" => 0x4F,
        "left" => 0x50,
        "down" => 0x51,
        "up" => 0x52,
        "numlock" => 0x53,
        "menu" | "contextmenu" => 0x65,
        "control" | "ctrl" => 0xE0,
        "shift" => 0xE1,
        "alt" => 0xE2,
        "platform" | "cmd" | "super" | "win" => 0xE3,
        _ => return None,
    })
}

pub fn mods_of(m: &Modifiers) -> u8 {
    let mut b = 0;
    if m.control {
        b |= mods::LCTRL;
    }
    if m.shift {
        b |= mods::LSHIFT;
    }
    if m.alt {
        b |= mods::LALT;
    }
    if m.platform {
        b |= mods::LGUI;
    }
    b
}

/// Display of a shortcut ("Ctrl+Shift+A").
pub fn stroke_label(s: devhub_core::KeyStroke) -> String {
    let mut out = devhub_core::keys::mods_label(s.mods);
    if s.usage != 0 {
        out.push_str(&devhub_core::keys::hid_name(s.usage));
    } else if out.ends_with('+') {
        out.pop();
    }
    if out.is_empty() {
        "(unset)".into()
    } else {
        out
    }
}
