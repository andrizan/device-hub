//! Persistent state of firmware update notifications (`%APPDATA%\DeviceHub\firmware.json`).
//! The key is `DeviceInfo.model` (one entry per model — shared when several of the same model are connected).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Interval between automatic checks.
pub const CHECK_EVERY: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FwEntry {
    /// Fingerprint of the distribution last checked (baseline).
    pub fingerprint: String,
    /// Time of the last check attempt (UNIX seconds). Recorded even on failure so we do not retry too often.
    pub last_check: u64,
    /// Fingerprint the user dismissed — the same update is not notified again.
    pub dismissed: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FwState {
    pub devices: BTreeMap<String, FwEntry>,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("DeviceHub").join("firmware.json"))
}

impl FwState {
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

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Whether the interval for an automatic check has elapsed (always true if force).
    pub fn due(&self, key: &str, force: bool) -> bool {
        if force {
            return true;
        }
        let last = self.devices.get(key).map(|e| e.last_check).unwrap_or(0);
        Self::now().saturating_sub(last) >= CHECK_EVERY
    }

    /// Stamp just before starting a check (no retry for 24h even on failure).
    pub fn touch(&mut self, key: &str) {
        self.devices.entry(key.to_string()).or_default().last_check = Self::now();
    }
}
