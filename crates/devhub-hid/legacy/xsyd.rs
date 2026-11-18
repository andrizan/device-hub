//! XSYD protocol (hub.xsyd.top) — Hall effect keyboards such as the SU66 Max.
//!
//! Analyzed on a real device (1ca6:3009, usage_page 0xFFB0 / usage 1):
//! - 64B output report + 64B input report, no report id
//! - Frame: `[group][sub][args...]` → response `[group][sub][data...]`
//! - group: Device=1, Global=2, LayoutAndKey=3, Performance=4, Lighting=5,
//!   HigherKey=6, Macro=7, CustomCommand=10, Displayer=12, ThreeMode=13
//! - A response with sub==0xFF means "unsupported command" (confirmed on a real device)

use devhub_core::backend::{DeviceDriver, HubError, HubResult};
use devhub_core::model::*;
use devhub_core::registry;
use hidapi::HidDevice;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const READY: bool = true;

const POLL_MS: i32 = 60;
const DEADLINE_MS: u64 = 700;

#[allow(dead_code)]
mod grp {
    pub const DEVICE: u8 = 1;
    pub const LAYOUT_KEY: u8 = 3;
    pub const PERFORMANCE: u8 = 4;
    pub const LIGHTING: u8 = 5;
    pub const THREE_MODE: u8 = 13;
}
mod sub {
    pub const READ: u8 = 1;
    pub const SETTING: u8 = 2;
    pub const DEVICE_INFO: u8 = 2;
    pub const GET_BASIC_INFO: u8 = 1;
    /// device feature query — response r[2]=layer count (2 on the real device).
    pub const FEATURE: u8 = 3;
    pub const GET_KEY_LAYOUT: u8 = 1;
}

pub struct XsydDriver {
    info: DeviceInfo,
    dev: Mutex<HidDevice>,
    /// Layer count from r[2] of the device feature (1,3) response. 2 if the query fails.
    layers: u8,
}

pub fn open(info: DeviceInfo, path: &std::ffi::CString) -> HubResult<XsydDriver> {
    let api = hidapi::HidApi::new().map_err(|e| HubError::Io(e.to_string()))?;
    let dev = api
        .open_path(path)
        .map_err(|e| HubError::Io(format!("open {}: {e}", info.name)))?;
    dev.set_blocking_mode(false).ok();
    let drv = XsydDriver {
        info,
        dev: Mutex::new(dev),
        layers: 2,
    };
    let layers = drv
        .cmd(grp::DEVICE, sub::FEATURE, &[])
        .map(|r| r[2].max(1))
        .unwrap_or(2);
    Ok(XsydDriver { layers, ..drv })
}

impl XsydDriver {
    /// group/sub frame → 64B response. sub==0xFF (unsupported) becomes Unsupported.
    fn cmd(&self, group: u8, sub: u8, args: &[u8]) -> HubResult<[u8; 64]> {
        let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
        let mut w = [0u8; 65];
        w[1] = group;
        w[2] = sub;
        let n = args.len().min(61);
        w[3..3 + n].copy_from_slice(&args[..n]);
        dev.write(&w)
            .map_err(|e| HubError::Io(format!("xsyd write: {e}")))?;

        let deadline = Instant::now() + Duration::from_millis(DEADLINE_MS);
        let mut buf = [0u8; 65];
        while Instant::now() < deadline {
            match dev.read_timeout(&mut buf, POLL_MS) {
                Ok(m) if m > 0 && buf[0] == group => {
                    let mut out = [0u8; 64];
                    out.copy_from_slice(&buf[..64]);
                    if out[1] == 0xFF {
                        return Err(HubError::Unsupported("command"));
                    }
                    return Ok(out);
                }
                Ok(_) => {}
                Err(e) => return Err(HubError::Io(format!("xsyd read: {e}"))),
            }
        }
        Err(HubError::Protocol("xsyd: response timeout".into()))
    }
}

/// Firmware string: picks the trailing ASCII chunk of the device_info response
/// (real device: "... 49142 .. 2026101216:41:03" — the tail is the build time).
fn parse_fw(r: &[u8; 64]) -> Option<String> {
    let tail: String = r[2..]
        .iter()
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
        .collect();
    tail.split_whitespace().last().map(|s| s.to_string())
}

/// u16 key code → name. XSYD is HID usage based (provisional: 0x04..=0x38 etc. as base names).
fn keycode_name(kc: u16) -> String {
    const NAMES: &[(u16, &str)] = &[
        (0x04, "A"), (0x05, "B"), (0x06, "C"), (0x07, "D"), (0x08, "E"), (0x09, "F"),
        (0x0A, "G"), (0x0B, "H"), (0x0C, "I"), (0x0D, "J"), (0x0E, "K"), (0x0F, "L"),
        (0x10, "M"), (0x11, "N"), (0x12, "O"), (0x13, "P"), (0x14, "Q"), (0x15, "R"),
        (0x16, "S"), (0x17, "T"), (0x18, "U"), (0x19, "V"), (0x1A, "W"), (0x1B, "X"),
        (0x1C, "Y"), (0x1D, "Z"), (0x1E, "1"), (0x1F, "2"), (0x20, "3"), (0x21, "4"),
        (0x22, "5"), (0x23, "6"), (0x24, "7"), (0x25, "8"), (0x26, "9"), (0x27, "0"),
        (0x28, "Enter"), (0x29, "Esc"), (0x2A, "Backspace"), (0x2B, "Tab"), (0x2C, "Space"),
        (0x2D, "-"), (0x2E, "="), (0x2F, "["), (0x30, "]"), (0x31, "\\"), (0x33, ";"),
        (0x34, "'"), (0x35, "`"), (0x36, ","), (0x37, "."), (0x38, "/"), (0x39, "Caps"),
        (0x3A, "F1"), (0x3B, "F2"), (0x3C, "F3"), (0x3D, "F4"), (0x3E, "F5"), (0x3F, "F6"),
        (0x40, "F7"), (0x41, "F8"), (0x42, "F9"), (0x43, "F10"), (0x44, "F11"), (0x45, "F12"),
        (0x4C, "Del"), (0x4A, "Home"), (0x4D, "End"), (0x4E, "PgDn"), (0x4B, "PgUp"),
        (0x50, "←"), (0x51, "↓"), (0x52, "↑"), (0x4F, "→"),
        (0xE0, "LCtrl"), (0xE1, "LShift"), (0xE2, "LAlt"), (0xE3, "LWin"),
        (0xE4, "RCtrl"), (0xE5, "RShift"), (0xE6, "RAlt"), (0xE7, "RWin"),
        (0x65, "Menu"), (0x00, ""),
        // Media / consumer codes (from the real-device L1 dump)
        (0x2192, "PrtSc"), (0x20B6, "Rewind"), (0x20CD, "Play/Pause"), (0x20B5, "Next"),
        (0x20E9, "Vol+"), (0x20EA, "Vol-"), (0x20E2, "Mute"),
        // XSYD-specific keys (0xF1xx=Fn, 0xF2xx/F3xx=system/RGB)
        (0xF101, "Fn"), (0xF200, "KB-Lock"), (0xF201, "RGB-Tog"), (0xF202, "RGB-Mod"),
        (0xF205, "Calibrate"), (0xF206, "Config-Sw"), (0xF207, "Profile"), (0xF208, "Reset"),
        (0xF209, "BT-Pair"), (0xF20A, "Sleep-Tog"),
        (0xF300, "ScrSav-"), (0xF301, "ScrSav+"), (0xF302, "Brt+"), (0xF303, "Brt-"),
        (0xF304, "Spd-"), (0xF305, "Spd+"), (0xF307, "Mode-"), (0xF310, "Eff-"),
        (0xF311, "Eff+"), (0xF317, "Hue-"),
    ];
    NAMES
        .iter()
        .find(|(c, _)| *c == kc)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| format!("KC{kc:04X}"))
}

impl DeviceDriver for XsydDriver {
    fn info(&self) -> DeviceInfo {
        let mut info = self.info.clone();
        if let Ok(r) = self.cmd(grp::DEVICE, sub::DEVICE_INFO, &[]) {
            if let Some(fw) = parse_fw(&r) {
                info.firmware = Some(fw);
            }
            info.supported = true;
        }
        info
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            mouse: None,
            keyboard: Some(KeyboardCaps {
                layout: registry::SU66MAX_LAYOUT,
                layers: self.layers,
                hall_effect: true,
                actuation_min: 0.1,
                actuation_max: 4.0,
                rapid_trigger: true,
                per_key_he: true,
                dks: true,
                mod_tap: true,
                socd_pairs: 4,
                macro_slots: 32,
                polling_rates: &[125, 250, 500, 1000, 2000, 4000, 8000],
                light_modes: &[
                    LightMode::Off,
                    LightMode::Static,
                    LightMode::Breathing,
                    LightMode::Wave,
                    LightMode::Spectrum,
                    LightMode::Reactive,
                    LightMode::Rain,
                    LightMode::Custom,
                ],
                per_key_rgb: true,
                onboard_profiles: 1,
            }),
        }
    }

    fn keyboard_config(&self) -> HubResult<KeyboardConfig> {
        let mut cfg = KeyboardConfig {
            active_layer: 0,
            layers: Vec::new(),
            he_global: HeParams::default(),
            he_overrides: Default::default(),
            lighting: LightingConfig {
                mode: LightMode::Static,
                color: Rgb::new(255, 255, 255),
                brightness: 80,
                speed: 50,
                per_key: Default::default(),
            },
            macros: Vec::new(),
            socd: Vec::new(),
            polling_hz: 1000,
        };

        // Lighting: [5,1] → [4]=mode raw, [5]=effect, [6]=bright, [7]=speed (confirmed on the real device)
        if let Ok(r) = self.cmd(grp::LIGHTING, sub::READ, &[]) {
            cfg.lighting.brightness = r[6].min(100);
            cfg.lighting.speed = r[7].min(100);
            cfg.lighting.mode = match r[4] {
                0 => LightMode::Off,
                1 => LightMode::Static,
                2 => LightMode::Breathing,
                3 => LightMode::Wave,
                4 => LightMode::Spectrum,
                5 => LightMode::Reactive,
                6 => LightMode::Rain,
                _ => LightMode::Static,
            };
        }

        // Layers: [3,1,layer,row] → [2]=layer [3]=row [4..64]=30×u16 key codes
        // Stream position = row*30+col. Real device: row1..=5, col0..=14 (SU66MAX_LAYOUT.index).
        for layer_id in 0..self.layers {
            // Read every row and store it in stream[row*30+col]
            let mut stream = vec![0u16; 8 * 30];
            let mut got_any = false;
            for row in 0..8u8 {
                match self.cmd(grp::LAYOUT_KEY, sub::GET_KEY_LAYOUT, &[layer_id, row]) {
                    Ok(r) if r[2] == layer_id && r[3] == row => {
                        for i in 0..30 {
                            stream[row as usize * 30 + i] =
                                u16::from_le_bytes([r[4 + i * 2], r[5 + i * 2]]);
                        }
                        got_any = true;
                    }
                    _ => break,
                }
            }
            if !got_any {
                break;
            }
            let keys: Vec<KeyAction> = registry::SU66MAX_LAYOUT
                .iter()
                .map(|k| {
                    let (row, col) = registry::su66max_matrix_pos(k.index);
                    let kc = stream.get(row as usize * 30 + col as usize).copied().unwrap_or(0);
                    match kc {
                        0x0000 | 0x0001 => KeyAction::None, // unassigned / transparent
                        _ => KeyAction::Standard(keycode_name(kc)),
                    }
                })
                .collect();
            cfg.layers.push(Layer {
                name: format!("Layer {layer_id}"),
                keys,
            });
        }
        if cfg.layers.is_empty() {
            cfg.layers.push(Layer {
                name: "Layer 0".into(),
                keys: vec![KeyAction::None; registry::SU66MAX_LAYOUT.len()],
            });
        }

        Ok(cfg)
    }

    fn apply_keyboard(&self, cfg: &KeyboardConfig) -> HubResult<()> {
        // Write lighting only (read direction confirmed on a real device. Writing is [5,2] + keeping the raw header)
        let cur = self.cmd(grp::LIGHTING, sub::READ, &[])?;
        let mode_raw = match cfg.lighting.mode {
            LightMode::Off => 0,
            LightMode::Static => 1,
            LightMode::Breathing => 2,
            LightMode::Wave => 3,
            LightMode::Spectrum => 4,
            LightMode::Reactive => 5,
            LightMode::Rain => 6,
            LightMode::Custom => 7,
        };
        let args = [
            cur[2], cur[3], mode_raw, cur[5], cfg.lighting.brightness, cfg.lighting.speed,
        ];
        self.cmd(grp::LIGHTING, sub::SETTING, &args)?;
        Ok(())
    }

    fn battery(&self) -> HubResult<Battery> {
        // [13,1] → [2]=mode [3]=charge [4]=percent. SU66 is wired → should come back as unsupported
        let r = self.cmd(grp::THREE_MODE, sub::GET_BASIC_INFO, &[])?;
        Ok(Battery {
            percent: Some(r[4]),
            charging: r[3] == 1,
            voltage_mv: None,
        })
    }
}
