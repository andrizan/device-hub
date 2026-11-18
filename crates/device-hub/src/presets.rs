//! Presets saved on the PC (per-game settings etc.). Replaces the Web version's cloud configs, and
//! any number can be saved without being tied to the on-board slots (4 etc.).
//!
//! Location: `%APPDATA%\DeviceHub\presets\<model>\<name>.json` (contents are a `DeviceState`).
//! Applying writes only the diff via `diff_changes`, so it is fast.

use devhub_core::DeviceState;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
}

fn dir(model: &str) -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let safe: String = model
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(base.join("DeviceHub").join("presets").join(safe))
}

fn file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{}.json", s.trim())
}

pub fn list(model: &str) -> Vec<Preset> {
    let Some(d) = dir(model) else { return vec![] };
    let mut v: Vec<Preset> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| Preset {
            name: e
                .path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            modified: e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH),
            path: e.path(),
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn save(model: &str, name: &str, st: &DeviceState) -> Result<(), String> {
    let d = dir(model).ok_or("Save location not found")?;
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let s = serde_json::to_string_pretty(st).map_err(|e| e.to_string())?;
    std::fs::write(d.join(file_name(name)), s).map_err(|e| e.to_string())
}

pub fn load(p: &Preset) -> Result<DeviceState, String> {
    let s = std::fs::read_to_string(&p.path).map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

pub fn delete(p: &Preset) -> Result<(), String> {
    std::fs::remove_file(&p.path).map_err(|e| e.to_string())
}

pub fn rename(p: &Preset, new_name: &str) -> Result<(), String> {
    let to = p.path.with_file_name(file_name(new_name));
    std::fs::rename(&p.path, to).map_err(|e| e.to_string())
}
