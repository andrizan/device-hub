//! Look and displayed keys of the OBS overlay, edited on the app (Device HUB) side.
//!
//! The "OBS overlay" screen of Device HUB writes `%APPDATA%\DeviceHub\obs_overlay.json`,
//! and the OBS plugin checks its modification time every second and re-reads it (applied without restarting OBS).
//! It takes effect as a **union** with the OBS-side properties (per-key checks, etc.).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Look theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    /// Translucent dark panel (default).
    #[default]
    Glass,
    /// Light panel.
    Light,
    /// No background (keys only; for overlaying directly on the stream).
    Clear,
    /// Black base + accent outline and glow.
    Neon,
}

impl Theme {
    pub const ALL: [Theme; 4] = [Theme::Glass, Theme::Light, Theme::Clear, Theme::Neon];
    pub fn label(self) -> &'static str {
        match self {
            Theme::Glass => "Glass",
            Theme::Light => "Light",
            Theme::Clear => "No background",
            Theme::Neon => "Neon",
        }
    }
}

/// How the mouse is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MouseStyle {
    /// Mouse shape + horizontal bars (default).
    #[default]
    Both,
    /// Mouse shape only.
    Shape,
    /// Horizontal bars only.
    Bars,
}

impl MouseStyle {
    pub const ALL: [MouseStyle; 3] = [MouseStyle::Both, MouseStyle::Shape, MouseStyle::Bars];
    pub fn label(self) -> &'static str {
        match self {
            MouseStyle::Both => "Shape + bars",
            MouseStyle::Shape => "Shape only",
            MouseStyle::Bars => "Bars only",
        }
    }
}

/// Direction of the press fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FillMode {
    /// Fills from the bottom (default).
    #[default]
    Bottom,
    /// Sinks from the top.
    Top,
    /// The whole key lights up according to depth.
    Fade,
}

impl FillMode {
    pub const ALL: [FillMode; 3] = [FillMode::Bottom, FillMode::Top, FillMode::Fade];
    pub fn label(self) -> &'static str {
        match self {
            FillMode::Bottom => "From bottom",
            FillMode::Top => "From top",
            FillMode::Fade => "Fade",
        }
    }
}

/// Layout of the combined source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Arrange {
    /// Keyboard left, mouse right (default).
    #[default]
    Side,
    /// Keyboard top, mouse bottom.
    Stack,
    /// Mouse left, keyboard right.
    Swap,
}

impl Arrange {
    pub const ALL: [Arrange; 3] = [Arrange::Side, Arrange::Stack, Arrange::Swap];
    pub fn label(self) -> &'static str {
        match self {
            Arrange::Side => "Side by side",
            Arrange::Stack => "Stacked",
            Arrange::Swap => "Swapped",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayCfg {
    pub theme: Theme,
    /// Accent color (RGB). A source whose color was chosen on the OBS side takes priority.
    pub accent: [u8; 3],
    /// Resolution multiplier (1.0 / 1.5 / 2.0). Larger values stay sharp when scaled up.
    pub scale: f32,
    pub title: bool,
    pub labels: bool,
    /// Show the press amount (mm / %) as numbers.
    pub numbers: bool,
    pub mouse_style: MouseStyle,
    /// Keys to show (`LayoutKey::id`). **Empty = show all**. Only the selected keys appear.
    pub show_keys: Vec<u16>,
    /// Mouse buttons to show (display names). Empty = show all.
    pub show_buttons: Vec<String>,
    /// Keys to hide (`LayoutKey::id`). Legacy field - kept for compatibility with saved settings.
    pub hide_keys: Vec<u16>,
    /// Mouse buttons to hide (display names).
    pub hide_buttons: Vec<String>,

    // ---- Layout ----
    /// Key size factor (0.7–1.4). Changes only the key density, separately from scale.
    #[serde(default = "one")]
    pub key_size: f32,
    /// Corner radius factor (0–2). 1 = standard roundness.
    #[serde(default = "one")]
    pub radius: f32,
    /// Panel padding factor (0.5–2).
    #[serde(default = "one")]
    pub pad: f32,
    /// Layout of the combined source.
    pub arrange: Arrange,

    // ---- Color ----
    /// Background panel opacity % (0–100). Multiplied onto the theme color.
    #[serde(default = "hundred")]
    pub panel_alpha: u8,
    /// Press color (RGB). None = same as the accent.
    pub pressed: Option<[u8; 3]>,

    // ---- Display elements ----
    /// Glow intensity % when pressed (0–200).
    #[serde(default = "hundred")]
    pub glow: u8,
    /// Direction of the press fill.
    pub fill_mode: FillMode,
    /// Actuation point line (the horizontal line inside the key).
    #[serde(default = "yes")]
    pub trigger_line: bool,
    /// AP/FT/R text inside the key.
    #[serde(default = "yes")]
    pub badges: bool,
    /// Show numbers in % instead of mm.
    pub num_pct: bool,
    /// Title text (empty = device name).
    #[serde(default)]
    pub title_text: String,
    /// Shadows of the keys and panel.
    #[serde(default = "yes")]
    pub shadow: bool,
}

fn one() -> f32 {
    1.0
}
fn hundred() -> u8 {
    100
}
fn yes() -> bool {
    true
}

impl Default for OverlayCfg {
    fn default() -> Self {
        Self {
            theme: Theme::Glass,
            accent: [60, 190, 255],
            scale: 1.0,
            title: true,
            labels: true,
            numbers: false,
            mouse_style: MouseStyle::Both,
            show_keys: vec![],
            show_buttons: vec![],
            hide_keys: vec![],
            hide_buttons: vec![],
            key_size: 1.0,
            radius: 1.0,
            pad: 1.0,
            arrange: Arrange::Side,
            panel_alpha: 100,
            pressed: None,
            glow: 100,
            fill_mode: FillMode::Bottom,
            trigger_line: true,
            badges: true,
            num_pct: false,
            title_text: String::new(),
            shadow: true,
        }
    }
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("DeviceHub").join("obs_overlay.json"))
}

pub fn load() -> OverlayCfg {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &OverlayCfg) -> std::io::Result<()> {
    let Some(p) = path() else { return Ok(()) };
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(cfg).unwrap_or_default())?;
    std::fs::rename(&tmp, &p)
}

struct Cache {
    cfg: OverlayCfg,
    mtime: Option<SystemTime>,
    checked: Option<Instant>,
    gen: u64,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Current settings and generation number (+1 when they change). The file is checked at most once per second.
pub fn current() -> (OverlayCfg, u64) {
    let mut g = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let c = g.get_or_insert_with(|| Cache {
        cfg: OverlayCfg::default(),
        mtime: None,
        checked: None,
        gen: 0,
    });
    if c.checked
        .is_none_or(|t| t.elapsed() > Duration::from_secs(1))
    {
        c.checked = Some(Instant::now());
        let m = path()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        if m != c.mtime || c.gen == 0 {
            c.mtime = m;
            let n = load();
            if n != c.cfg || c.gen == 0 {
                c.cfg = n;
                c.gen += 1;
            }
        }
    }
    (c.cfg.clone(), c.gen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_file_without_new_fields_loads_with_defaults() {
        // An old format without the new fields loads with the defaults.
        let c: OverlayCfg = serde_json::from_str(r#"{"theme":"Neon","accent":[1,2,3]}"#).unwrap();
        assert_eq!(c.theme, Theme::Neon);
        assert_eq!(c.key_size, 1.0);
        assert_eq!(c.radius, 1.0);
        assert_eq!(c.pad, 1.0);
        assert_eq!(c.arrange, Arrange::Side);
        assert_eq!(c.panel_alpha, 100);
        assert_eq!(c.pressed, None);
        assert_eq!(c.glow, 100);
        assert_eq!(c.fill_mode, FillMode::Bottom);
        assert!(c.trigger_line && c.badges && c.shadow);
        assert!(!c.num_pct);
        assert!(c.title_text.is_empty());
    }

    #[test]
    fn new_fields_roundtrip() {
        let c = OverlayCfg {
            key_size: 0.85,
            radius: 1.6,
            pad: 1.5,
            arrange: Arrange::Stack,
            panel_alpha: 40,
            pressed: Some([255, 150, 60]),
            glow: 180,
            fill_mode: FillMode::Top,
            trigger_line: false,
            badges: false,
            num_pct: true,
            title_text: "TEST".into(),
            shadow: false,
            ..Default::default()
        };
        let j = serde_json::to_string(&c).unwrap();
        let d: OverlayCfg = serde_json::from_str(&j).unwrap();
        assert_eq!(d, c);
    }
}
