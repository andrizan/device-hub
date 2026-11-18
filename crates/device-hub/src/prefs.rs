//! App settings (`%APPDATA%\DeviceHub\prefs.json`). Device settings live on the device,
//! so only appearance and UI state go here.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// "dark" | "light"
    pub theme: String,
    pub accent: u8,
    /// Font (an id in theme::FONT_OPTIONS).
    pub font: String,
    /// GPU glass (false = opaque surfaces).
    pub glass: bool,
    /// Animation.
    pub motion: bool,
    /// Wallpaper image path (None = aurora).
    pub wallpaper: Option<String>,
    /// Last opened device.
    pub last_device: Option<String>,
    /// Stop drawing when sent to the background (zero load even when open behind a game).
    pub idle_throttle: bool,
    /// Check for firmware updates at startup (at most once per 24 hours).
    pub fw_check: bool,
    /// Also show a Windows notification when an update is found (default OFF).
    pub fw_native_notify: bool,
    /// Device aliases (DeviceId → display name).
    pub nicknames: std::collections::BTreeMap<String, String>,
    /// EQ presets saved by the user (shared across USB DACs).
    pub eq_presets: Vec<EqPreset>,
    /// Live lighting (DirectDrive) settings.
    pub livefx: crate::livefx::FxPrefs,
    /// Saved settings profiles (a Change list per device).
    #[serde(default)]
    pub profiles: Vec<SavedProfile>,
    /// Process watch rules (apply a profile in step with the foreground app).
    #[serde(default)]
    pub proc_rules: Vec<ProcRule>,
}

/// Settings profile — applies by replaying the recorded `Change`s in order.
/// Multiple target devices can be selected, each with its own change list.
#[derive(Clone, Debug, Serialize)]
pub struct SavedProfile {
    pub name: String,
    /// Change list per target device.
    #[serde(default)]
    pub devices: Vec<ProfileTarget>,
}

/// One device's share within a profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileTarget {
    /// Target device (DeviceId.0).
    pub device: String,
    /// Changes collected by recording or manually (applied in this order).
    #[serde(default)]
    pub changes: Vec<devhub_core::Change>,
}

// Restore by hand so files in the old format (a single `device` + `changes`) can still be read.
impl<'de> Deserialize<'de> for SavedProfile {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Repr {
            name: String,
            #[serde(default)]
            devices: Vec<ProfileTarget>,
            // Old-format fields.
            #[serde(default)]
            device: Option<String>,
            #[serde(default)]
            changes: Vec<devhub_core::Change>,
        }
        let r = Repr::deserialize(de)?;
        let mut devices = r.devices;
        if let Some(d) = r.device {
            if !devices.iter().any(|t| t.device == d) {
                devices.push(ProfileTarget {
                    device: d,
                    changes: r.changes,
                });
            }
        }
        Ok(SavedProfile {
            name: r.name,
            devices,
        })
    }
}

/// Process watch rule: applies a profile while the exe name is in the foreground.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcRule {
    /// exe name to watch (lowercase, partial match. e.g. "valorant" / works without ".exe").
    pub process: String,
    /// Target device (DeviceId.0).
    pub device: String,
    /// Name of the profile to apply when it becomes the foreground app.
    pub on_fg: String,
    /// Name of the profile to apply when it leaves the foreground (empty = do nothing).
    #[serde(default)]
    pub on_leave: String,
    /// When disabled it is not evaluated (if turned OFF while applied, on_leave runs).
    #[serde(default = "proc_rule_enabled")]
    pub enabled: bool,
}

fn proc_rule_enabled() -> bool {
    true
}

/// Saved EQ preset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EqPreset {
    pub name: String,
    /// Preamp (EQ volume) dB. None = leave untouched.
    #[serde(default)]
    pub preamp: Option<f32>,
    pub bands: Vec<devhub_core::model::PeqBand>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            accent: 0,
            font: "modern".into(),
            glass: true,
            motion: true,
            wallpaper: None,
            last_device: None,
            idle_throttle: true,
            fw_check: true,
            fw_native_notify: false,
            nicknames: Default::default(),
            eq_presets: vec![],
            livefx: Default::default(),
            profiles: vec![],
            proc_rules: vec![],
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("DeviceHub").join("prefs.json"))
}

impl Prefs {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    pub fn light(&self) -> bool {
        self.theme == "light"
    }
}
