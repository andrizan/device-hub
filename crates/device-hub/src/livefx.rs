//! Live lighting (DirectDrive) engine — PC-driven key lighting that the official app does not have.
//!
//! Reads the **press depth (`live_travel`)** of a magnetic keyboard, computes a color per key, and
//! streams it directly to the real device's LEDs via DirectDrive (`DeviceDriver::direct_drive`).
//! The computation is pure functions in this module (called from the UI thread's 30fps ticker).
//! I/O is handled by `store::Hub` (travel read / dd_frame).
//!
//! Modes:
//! - `Depth` … color and brightness change with how deep the key is pressed (the analog feel, straight into light).
//! - `Heat`  … keystroke count heatmap for this session (the more a key is used, the redder).
//! - `Focus` … only the chosen keys (WASD etc.) are lit, the others stay dark. Keys light up when pressed.
//! - `Ripple`… ripples spread from the pressed key (stronger the deeper the press).

use devhub_core::model::{KeyboardCaps, LayoutKey};
use devhub_core::Rgb;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FxMode {
    #[default]
    Depth,
    Heat,
    Focus,
    Ripple,
    Status,
    Audio,
    AppFocus,
}

impl FxMode {
    pub const ALL: [FxMode; 7] = [
        FxMode::Depth,
        FxMode::Heat,
        FxMode::Focus,
        FxMode::Ripple,
        FxMode::Status,
        FxMode::Audio,
        FxMode::AppFocus,
    ];
    pub fn label(self) -> &'static str {
        match self {
            FxMode::Depth => "Press depth",
            FxMode::Heat => "Keystroke heatmap",
            FxMode::Focus => "Key highlight",
            FxMode::Ripple => "Ripple",
            FxMode::Status => "Status",
            FxMode::Audio => "Audio",
            FxMode::AppFocus => "App sync",
        }
    }
    pub fn desc(self) -> &'static str {
        match self {
            FxMode::Depth => {
                "Color and brightness change continuously with press depth. The color switches past the actuation point"
            }
            FxMode::Heat => "Counts keystrokes and maps them to color. The more a key is used, the redder (resettable)",
            FxMode::Focus => "Lights only the keys used in games, such as WASD. Keys light up when pressed",
            FxMode::Ripple => "Ripples spread from the pressed key. The deeper the press, the stronger and faster",
            FxMode::Status => "Overlays CapsLock and low battery on wireless devices onto the lighting (hybrid recommended)",
            FxMode::Audio => "Shows a spectrum on the keyboard in sync with the sound being played",
            FxMode::AppFocus => "Detects the foreground app and switches the highlighted keys automatically",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            FxMode::Depth => "arrow-down-to-line",
            FxMode::Heat => "flame",
            FxMode::Focus => "crosshair",
            FxMode::Ripple => "radio",
            FxMode::Status => "activity",
            FxMode::Audio => "music",
            FxMode::AppFocus => "app-window",
        }
    }
}

/// Settings (`livefx` in prefs.json).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FxPrefs {
    pub mode: FxMode,
    /// Hybrid: disabled cells are not sent to DD and are left to the keyboard's own effect
    /// (the firmware lighting stays as the background, and only presses/highlights are PC-driven — verified on real hardware).
    pub hybrid: bool,
    /// Background color and brightness (0..100). In hybrid it is only used to decide what a "disabled cell" is.
    pub base: Rgb,
    pub base_level: u8,
    /// Primary color (shallow / highlighted keys / ripples).
    pub a: Rgb,
    /// Secondary color (deep / after actuation / when pressed).
    pub b: Rgb,
    /// Sensitivity of the audio mode (%).
    pub audio_gain: u8,
    /// Overall brightness 0..100.
    pub brightness: u8,
    /// Afterglow (ms). 0 turns off immediately.
    pub decay_ms: u32,
    /// Keys lit by key highlight (`LayoutKey::label`).
    pub focus: Vec<String>,
    /// App sync: partial match of the exe name (lowercase) → preset name.
    /// If the foreground app matches, highlight with the key list of `focus_preset`.
    pub app_map: Vec<(String, String)>,
    /// Start automatically at launch (when this device is found).
    pub autostart: bool,
    /// Last device that was run (DeviceId).
    pub device: Option<String>,
}

impl Default for FxPrefs {
    fn default() -> Self {
        Self {
            mode: FxMode::Depth,
            hybrid: false,
            base: Rgb::hex(0x4cb4ff),
            base_level: 8,
            a: Rgb::hex(0x34d6a8),
            b: Rgb::hex(0xff5c8a),
            audio_gain: 150,
            brightness: 90,
            decay_ms: 350,
            focus: focus_preset("FPS"),
            app_map: vec![
                ("valorant".into(), "FPS".into()),
                ("cs2".into(), "FPS".into()),
                ("apex".into(), "FPS".into()),
                ("dota".into(), "MOBA".into()),
                ("league".into(), "MOBA".into()),
            ],
            autostart: false,
            device: None,
        }
    }
}

/// Key highlight presets (key legend labels).
pub const FOCUS_PRESETS: [&str; 4] = ["FPS", "WASD", "MOBA", "Number row"];

/// The Japanese name this preset was saved under before the UI became English
/// (`prefs.json` stores the preset name). Written as escapes so no Japanese
/// text stays in the source.
const LEGACY_NUMBER_ROW: &str = "\u{6570}\u{5B57}\u{5217}";

pub fn focus_preset(name: &str) -> Vec<String> {
    let v: &[&str] = match name {
        "WASD" => &["W", "A", "S", "D"],
        "MOBA" => &[
            "Q", "W", "E", "R", "D", "F", "1", "2", "3", "4", "5", "6", "B", "Tab",
        ],
        "Number row" | LEGACY_NUMBER_ROW => &["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"],
        _ => &[
            "W", "A", "S", "D", "Q", "E", "R", "F", "C", "X", "G", "Z", "1", "2", "3", "4",
            "Shift", "LShift", "Ctrl", "LCtrl", "Space", "Tab", "Esc",
        ],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// One ripple.
#[derive(Clone, Copy, Debug)]
struct Ripple {
    x: f32,
    y: f32,
    age: f32,
    power: f32,
}

/// External inputs used for frame computation (the Shell collects and passes them every 30fps tick).
#[derive(Clone, Default)]
pub struct FxEnv {
    /// Whether Caps Lock is on.
    pub caps_lock: bool,
    /// Lowest wireless battery including other devices (percent, charging). None if there is none.
    pub battery: Option<(u8, bool)>,
    /// Audio bands 0..1 (low → high). For `FxMode::Audio`.
    pub audio: Vec<f32>,
    /// Highlight keys resolved by app sync (falls back to `cfg.focus` when empty).
    pub app_focus: Vec<String>,
    /// Foreground app's exe name (for display).
    pub fg_app: Option<String>,
}

/// Runtime state (session only).
#[derive(Default)]
pub struct Engine {
    /// Afterglow 0..1 (layout index).
    glow: Vec<f32>,
    /// Whether the key is actuated (for rising-edge detection).
    down: Vec<bool>,
    /// Keystroke counts.
    pub heat: Vec<u32>,
    ripples: Vec<Ripple>,
    /// Last frame produced (shared with the UI preview).
    pub last: Vec<Rgb>,
    /// Total keystrokes.
    pub presses: u64,
    /// Whether DirectDrive has ever been enabled (to detect an external stop).
    pub armed: bool,
    /// Time of the last attempt to resume live measurement.
    pub rearm: Option<std::time::Instant>,
    /// Elapsed engine time (for blinking and pulses).
    t: f32,
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0., 1.);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b))
}

fn scale(c: Rgb, k: f32) -> Rgb {
    let k = k.clamp(0., 1.);
    Rgb::new(
        (c.r as f32 * k).round() as u8,
        (c.g as f32 * k).round() as u8,
        (c.b as f32 * k).round() as u8,
    )
}

/// Display color of a "disabled cell". Each `frame` mode outputs
/// `scale(base, brightness/100)` for idle cells, so in hybrid mode
/// cells that match this color are not sent to DD and are left to the firmware effect.
/// (flag=0 = "don't touch" is verified on real hardware — cells that are not pinned
/// are simply driven by the firmware)
pub fn off_color(cfg: &FxPrefs) -> Rgb {
    scale(
        scale(cfg.base, cfg.base_level as f32 / 100.),
        cfg.brightness as f32 / 100.,
    )
}

/// Saturating add.
fn add(a: Rgb, b: Rgb) -> Rgb {
    Rgb::new(
        a.r.saturating_add(b.r),
        a.g.saturating_add(b.g),
        a.b.saturating_add(b.b),
    )
}

/// Heatmap palette (0 = blue → green → yellow → red).
fn heat_color(t: f32) -> Rgb {
    const STOPS: [(f32, u32); 5] = [
        (0.0, 0x1e3a8a),
        (0.25, 0x0ea5e9),
        (0.5, 0x22c55e),
        (0.75, 0xf5c24c),
        (1.0, 0xff3b3b),
    ];
    let t = t.clamp(0., 1.);
    for w in STOPS.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            return mix(Rgb::hex(c0), Rgb::hex(c1), (t - t0) / (t1 - t0));
        }
    }
    Rgb::hex(0xff3b3b)
}

fn center(k: &LayoutKey) -> (f32, f32) {
    (k.x + k.w / 2., k.y + k.h / 2.)
}

/// Number keys ("1".."0") → gauge position 0..9.
fn digit_pos(label: &str) -> Option<usize> {
    match label {
        "1" => Some(0),
        "2" => Some(1),
        "3" => Some(2),
        "4" => Some(3),
        "5" => Some(4),
        "6" => Some(5),
        "7" => Some(6),
        "8" => Some(7),
        "9" => Some(8),
        "0" => Some(9),
        _ => None,
    }
}

impl Engine {
    pub fn reset_heat(&mut self) {
        self.heat.iter_mut().for_each(|h| *h = 0);
        self.presses = 0;
    }

    /// Computes one frame.
    /// - `depths` … press depth in mm (layout order; 0 if missing).
    /// - `actuation` … per-key actuation point in mm (layout order; 1.2 if missing).
    /// - `full` … full stroke in mm used to normalize depth.
    /// - `dt` … seconds since the previous frame.
    /// - `env` … CapsLock / battery / audio / foreground app (for Status, Audio and AppFocus).
    pub fn frame(
        &mut self,
        cfg: &FxPrefs,
        kc: &KeyboardCaps,
        depths: &[f32],
        actuation: &[f32],
        full: f32,
        dt: f32,
        env: &FxEnv,
    ) -> &[Rgb] {
        let n = kc.layout.len();
        self.t += dt;
        self.glow.resize(n, 0.);
        self.down.resize(n, false);
        self.heat.resize(n, 0);
        self.last.resize(n, Rgb::new(0, 0, 0));
        let full = full.max(0.5);
        // Afterglow decay factor (1/e over decay_ms).
        let fade = if cfg.decay_ms == 0 {
            0.
        } else {
            (-dt * 1000. / cfg.decay_ms as f32).exp()
        };
        let base = scale(cfg.base, cfg.base_level as f32 / 100.);
        // Press detection (actuation point + hysteresis)
        for i in 0..n {
            let d = depths.get(i).copied().unwrap_or(0.).max(0.);
            let act = actuation
                .get(i)
                .copied()
                .filter(|a| *a > 0.05)
                .unwrap_or(1.2);
            let was = self.down[i];
            let now = if was {
                d >= (act - 0.15).max(0.05)
            } else {
                d >= act
            };
            if now && !was {
                self.heat[i] = self.heat[i].saturating_add(1);
                self.presses += 1;
                let (x, y) = center(&kc.layout[i]);
                if cfg.mode == FxMode::Ripple && self.ripples.len() < 48 {
                    self.ripples.push(Ripple {
                        x,
                        y,
                        age: 0.,
                        power: (d / full).clamp(0.4, 1.),
                    });
                }
            }
            self.down[i] = now;
            let lvl = (d / full).clamp(0., 1.);
            self.glow[i] = (self.glow[i] * fade).max(lvl);
        }
        // Update ripples
        for r in self.ripples.iter_mut() {
            r.age += dt * (0.8 + r.power);
        }
        self.ripples.retain(|r| r.age < 1.6);
        let max_heat = self.heat.iter().copied().max().unwrap_or(0).max(1) as f32;
        // Coordinate system for Audio mode (normalization of bottom y and left x).
        let xmax = kc
            .layout
            .iter()
            .map(|k| k.x + k.w)
            .fold(0f32, f32::max)
            .max(1.);
        let ymax = kc
            .layout
            .iter()
            .map(|k| k.y + k.h)
            .fold(0f32, f32::max)
            .max(1.);
        // Battery gauge blink phase (slow at 40% or below, fast at 15% or below).
        let batt_pulse = self.t
            * if env.battery.is_some_and(|(p, _)| p <= 15) {
                6.
            } else {
                2.5
            };
        for i in 0..n {
            let k = &kc.layout[i];
            let g = self.glow[i];
            let act = actuation
                .get(i)
                .copied()
                .filter(|a| *a > 0.05)
                .unwrap_or(1.2);
            let c = match cfg.mode {
                FxMode::Depth => {
                    // Gradient from shallow to deep. Past the actuation point, shift toward the secondary color.
                    let d = depths.get(i).copied().unwrap_or(0.);
                    let t = if self.down[i] || d >= act {
                        0.65 + 0.35 * g
                    } else {
                        0.5 * (d / act).clamp(0., 1.)
                    };
                    let col = mix(cfg.a, cfg.b, t);
                    add(scale(base, 1. - g), scale(col, (g * 1.25).min(1.)))
                }
                FxMode::Heat => {
                    let h = self.heat[i];
                    let under = if h == 0 {
                        base
                    } else {
                        heat_color((h as f32 / max_heat).sqrt())
                    };
                    add(
                        scale(under, 1. - 0.6 * g),
                        scale(Rgb::new(255, 255, 255), g * 0.6),
                    )
                }
                FxMode::Focus => {
                    let on = cfg.focus.iter().any(|l| l.eq_ignore_ascii_case(&k.label));
                    let under = if on { cfg.a } else { base };
                    mix(under, cfg.b, g)
                }
                FxMode::Ripple => {
                    let (x, y) = center(k);
                    let mut s = 0f32;
                    for r in &self.ripples {
                        let rad = r.age * 9.;
                        let dist = ((x - r.x).powi(2) + ((y - r.y) * 1.1).powi(2)).sqrt();
                        let band = (1. - ((dist - rad).abs() / 1.3)).max(0.);
                        s += band * r.power * (1. - r.age / 1.6);
                    }
                    let s = s.min(1.);
                    let ring = mix(cfg.a, cfg.b, (s * 1.4).min(1.));
                    add(scale(base, 1. - s), scale(ring, s.max(g)))
                }
                FxMode::Status => {
                    // Keep the background as the firmware/base color and overlay only keys that carry information.
                    let mut c = base;
                    // CapsLock → light the Caps key in the secondary color.
                    if env.caps_lock
                        && (k.label.eq_ignore_ascii_case("caps")
                            || k.label.eq_ignore_ascii_case("capslock"))
                    {
                        c = cfg.b;
                    }
                    // Wireless device battery at 40% or below → the number row becomes a gauge
                    // (1 key = 10%, color goes red → yellow → green with the charge, blinking draws attention).
                    if let Some((pct, chg)) = env.battery {
                        if pct <= 40 {
                            if let Some(pos) = digit_pos(&k.label) {
                                let lit = pos * 10 < pct.max(1) as usize;
                                if lit {
                                    let meter = mix(
                                        Rgb::hex(0xff3b3b),
                                        Rgb::hex(0x22c55e),
                                        pct as f32 / 100.,
                                    );
                                    let pulse = 0.55 + 0.45 * batt_pulse.sin().abs();
                                    c = if chg { meter } else { scale(meter, pulse) };
                                }
                            }
                        }
                    }
                    add(scale(c, 1. - g), scale(cfg.b, g * 0.4))
                }
                FxMode::Audio => {
                    // Spectrum made by converting each column's band level into a "height from the bottom".
                    // The higher up, the closer the color is to the secondary color.
                    let nb = env.audio.len();
                    if nb == 0 {
                        base
                    } else {
                        let bi = (((k.x + k.w / 2.) / xmax) * nb as f32) as usize;
                        let lvl =
                            (env.audio[bi.min(nb - 1)] * cfg.audio_gain as f32 / 100.).min(1.);
                        let h = lvl * ymax;
                        let depth = ymax - (k.y + k.h); // height from the bottom edge
                        if depth < h {
                            // The closer to the peak, the more secondary color and the brighter.
                            let top = (1. - (h - depth).abs() / 1.2).clamp(0., 1.);
                            let col = mix(cfg.a, cfg.b, (depth / ymax) * 0.8 + top * 0.4);
                            add(scale(base, 0.4), scale(col, 0.25 + 0.75 * top))
                        } else {
                            scale(base, 0.4)
                        }
                    }
                }
                FxMode::AppFocus => {
                    // Keys for the foreground app (manual list if unresolved).
                    let keys = if env.app_focus.is_empty() {
                        &cfg.focus
                    } else {
                        &env.app_focus
                    };
                    let on = keys.iter().any(|l| l.eq_ignore_ascii_case(&k.label));
                    let under = if on { cfg.a } else { base };
                    mix(under, cfg.b, g)
                }
            };
            self.last[i] = scale(c, cfg.brightness as f32 / 100.);
        }
        &self.last
    }

    /// In descending order of keystroke count (legend, count).
    pub fn top_keys(&self, kc: &KeyboardCaps, n: usize) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = kc
            .layout
            .iter()
            .zip(&self.heat)
            .filter(|(_, h)| **h > 0)
            .map(|(k, h)| (k.label.clone(), *h))
            .collect();
        v.sort_by_key(|b| std::cmp::Reverse(b.1));
        v.truncate(n);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devhub_core::model::LayoutKey;

    fn kc() -> KeyboardCaps {
        let mut kc = devhub_core::demo::keyboard_caps().keyboard.unwrap();
        kc.layout = vec![
            LayoutKey {
                id: 1,
                x: 0.,
                y: 0.,
                w: 1.,
                h: 1.,
                label: "W".into(),
            },
            LayoutKey {
                id: 2,
                x: 1.,
                y: 0.,
                w: 1.,
                h: 1.,
                label: "K".into(),
            },
        ];
        kc
    }

    #[::core::prelude::v1::test]
    fn counts_presses_once_with_hysteresis() {
        let k = kc();
        let cfg = FxPrefs::default();
        let mut e = Engine::default();
        for d in [0., 1.5, 1.2, 1.5, 0.2, 1.6] {
            e.frame(
                &cfg,
                &k,
                &[d, 0.],
                &[1.2, 1.2],
                4.,
                0.033,
                &FxEnv::default(),
            );
        }
        assert_eq!(e.heat[0], 2);
        assert_eq!(e.presses, 2);
    }

    #[::core::prelude::v1::test]
    fn hybrid_off_cells_match_off_color() {
        let k = kc();
        let mut e = Engine::default();
        // In any mode, an "idle key" matches off_color
        // (= the check for "don't send to DD, leave to the firmware" holds).
        for mode in FxMode::ALL {
            let mut cfg = FxPrefs {
                mode,
                hybrid: true,
                brightness: 100,
                ..Default::default()
            };
            if mode == FxMode::Focus {
                cfg.focus = vec!["W".into()];
            }
            let f = e
                .frame(&cfg, &k, &[0., 0.], &[], 4., 0.033, &FxEnv::default())
                .to_vec();
            assert_eq!(f[1], off_color(&cfg), "{mode:?} idle cell");
            if mode == FxMode::Focus {
                assert_eq!(f[0], cfg.a);
            }
        }
    }

    #[::core::prelude::v1::test]
    fn focus_lights_only_selected() {
        let k = kc();
        let cfg = FxPrefs {
            mode: FxMode::Focus,
            base_level: 0,
            brightness: 100,
            ..Default::default()
        };
        let mut e = Engine::default();
        let f = e
            .frame(&cfg, &k, &[0., 0.], &[], 4., 0.033, &FxEnv::default())
            .to_vec();
        assert_eq!(f[0], cfg.a);
        assert_eq!(f[1], Rgb::new(0, 0, 0));
    }

    #[::core::prelude::v1::test]
    fn status_caps_and_battery() {
        // Verified with a layout of "Caps"(y=0) and "1"(number key).
        let mut k = kc();
        k.layout[0].label = "Caps".into();
        k.layout[1].label = "1".into();
        let cfg = FxPrefs {
            mode: FxMode::Status,
            base_level: 0,
            brightness: 100,
            ..Default::default()
        };
        let mut e = Engine::default();
        // Caps ON + battery 25% (charging) → Caps = secondary color, "1"(pos0) is lit within 25%.
        let env = FxEnv {
            caps_lock: true,
            battery: Some((25, true)),
            ..Default::default()
        };
        let f = e.frame(&cfg, &k, &[0., 0.], &[], 4., 0.033, &env).to_vec();
        assert_eq!(f[0], cfg.b, "caps on");
        assert_ne!(f[1], Rgb::new(0, 0, 0), "battery gauge lit");
        // Battery at 50% → gauge off. Caps is OFF too.
        let env2 = FxEnv {
            caps_lock: false,
            battery: Some((50, false)),
            ..Default::default()
        };
        let f2 = e.frame(&cfg, &k, &[0., 0.], &[], 4., 0.033, &env2).to_vec();
        assert_eq!(f2[0], Rgb::new(0, 0, 0));
        assert_eq!(f2[1], Rgb::new(0, 0, 0));
    }

    #[::core::prelude::v1::test]
    fn audio_column_height() {
        // Keys in 2 columns × y0 (top). Only the low band has sound → only the low-band column lights.
        let mut k = kc();
        k.layout[0].x = 0.;
        k.layout[1].x = 10.; // a different band column
        let cfg = FxPrefs {
            mode: FxMode::Audio,
            base_level: 0,
            brightness: 100,
            ..Default::default()
        };
        let mut audio = vec![0f32; crate::audio::NBANDS];
        audio[0] = 1.;
        let env = FxEnv {
            audio,
            ..Default::default()
        };
        let mut e = Engine::default();
        let f = e.frame(&cfg, &k, &[0., 0.], &[], 4., 0.033, &env).to_vec();
        assert_ne!(f[0], Rgb::new(0, 0, 0), "loud band lit");
        assert_eq!(f[1], Rgb::new(0, 0, 0), "quiet band dark");
    }

    #[::core::prelude::v1::test]
    fn appfocus_uses_resolved_keys() {
        let k = kc(); // "W", "K"
        let cfg = FxPrefs {
            mode: FxMode::AppFocus,
            base_level: 0,
            brightness: 100,
            focus: vec!["K".into()],
            ..Default::default()
        };
        let mut e = Engine::default();
        let env = FxEnv {
            app_focus: vec!["W".into()],
            ..Default::default()
        };
        let f = e.frame(&cfg, &k, &[0., 0.], &[], 4., 0.033, &env).to_vec();
        assert_eq!(f[0], cfg.a, "resolved W lit");
        assert_eq!(f[1], Rgb::new(0, 0, 0), "K not in app list");
    }
}
