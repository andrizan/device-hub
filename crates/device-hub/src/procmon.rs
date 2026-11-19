//! Process monitor — watches the foreground exe name and applies profiles automatically.
//!
//! For each rule in `prefs.proc_rules`, evaluates "is the exe in the foreground" about once per second,
//! and on the rising/falling transitions sends the `on_fg`/`on_leave` profiles in order
//! through `hub.apply`. It also runs while minimized, via the 33ms ticker.
//!
//! Transition evaluation rules:
//! - A tick where the foreground exe cannot be resolved makes no transition (so we do not restore by mistake on a high-privilege window).
//! - Evaluate only when the foreground is the same for 2 consecutive ticks (do not write for the intermediate state of a fast alt-tab).
//! - Apply all restores (leave) first and new entries (enter) after — so even when moving
//!   directly between managed apps, the new side always writes last.
//! - Rules with the same process+device are aggregated by key; when several match they are applied in order
//!   (lower rules take priority).
//! - Even if a rule is deleted/disabled/edited while applied, it is restored with the recorded on_leave.
//!
//! Changes made while the device cannot accept writes (loading, switching configuration, asleep)
//! are queued in `proc_pending` and drained once it can accept them — they do not vanish silently.

use crate::app::Shell;
use crate::prefs::ProcRule;
use devhub_core::{Change, DeviceId};
use gpui_kit::prelude::*;

/// Foreground window's exe name (lowercase, with extension).
#[cfg(windows)]
pub fn foreground_app() -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(h);
        if !ok || len == 0 {
            return None;
        }
        String::from_utf16_lossy(&buf[..len as usize])
            .rsplit(['\\', '/'])
            .next()
            .map(|s| s.to_lowercase())
    }
}

#[cfg(not(windows))]
pub fn foreground_app() -> Option<String> {
    None
}

/// List of processes that have a visible window: `(exe name, representative window title)`.
/// Only processes that can become foreground = only those for which a rule can actually fire.
/// The first is the current foreground process.
#[cfg(windows)]
pub fn running_apps() -> Vec<(String, String)> {
    use std::collections::BTreeMap;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };
    unsafe {
        // Collect (pid, title) of visible windows.
        struct Acc {
            v: Vec<(u32, String)>,
        }
        unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
            let acc = &mut *(l.0 as *mut Acc);
            if IsWindowVisible(h).as_bool() {
                let mut t = [0u16; 256];
                let n = GetWindowTextW(h, &mut t) as usize;
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, Some(&mut pid));
                if pid != 0 {
                    acc.v.push((
                        pid,
                        if n > 0 {
                            String::from_utf16_lossy(&t[..n])
                        } else {
                            String::new()
                        },
                    ));
                }
            }
            BOOL(1)
        }
        let mut acc = Acc { v: vec![] };
        let _ = EnumWindows(Some(cb), LPARAM(&mut acc as *mut _ as isize));
        // Look up pid → exe name and group by exe (the representative title is the first non-empty one).
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        for (pid, title) in acc.v {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                continue;
            };
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(h);
            if !ok || len == 0 {
                continue;
            }
            let exe = String::from_utf16_lossy(&buf[..len as usize])
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or_default()
                .to_lowercase();
            if exe.is_empty() {
                continue;
            }
            let e = map.entry(exe).or_default();
            if e.is_empty() && !title.is_empty() {
                *e = title;
            }
        }
        let fg = foreground_app().unwrap_or_default();
        let mut v: Vec<(String, String)> = map.into_iter().collect();
        v.sort_by(|a, b| (b.0 == fg).cmp(&(a.0 == fg)).then_with(|| a.0.cmp(&b.0)));
        v
    }
}

#[cfg(not(windows))]
pub fn running_apps() -> Vec<(String, String)> {
    vec![]
}

/// Whether the exe name matches the rule's process (lowercase, substring).
fn rule_matches(rule: &ProcRule, fg: &str) -> bool {
    let pat = rule.process.trim().to_lowercase();
    !pat.is_empty() && fg.contains(&pat)
}

/// Identification key of a rule (held by content, not index — robust against deletion/reordering).
fn rule_key(rule: &ProcRule) -> String {
    format!("{}|{}", rule.process.trim().to_lowercase(), rule.device)
}

/// Record for one rule being applied (information needed to restore on leaving).
#[derive(Clone, Debug, Default)]
pub struct ProcApplied {
    /// Target device of the rule (used as the filter for profile lookup).
    pub device: String,
    /// Profile name to apply on leaving (empty = do not restore).
    pub on_leave: String,
}

/// Evaluation result of one tick. The apply order is fixed: leaves (restore) → enters (new) —
/// so that when switching directly from a managed app A to a managed B, A's restore
/// does not overwrite B's apply.
#[derive(Default)]
struct ProcEval {
    /// Currently active keys → record used at the next leave.
    now: std::collections::HashMap<String, ProcApplied>,
    /// (device, profile name) — fire in rule order.
    leaves: Vec<(String, String)>,
    /// (device, list of on_fg profile names) — fire in rule order.
    enters: Vec<(String, Vec<String>)>,
}

/// Decides this tick's transitions from the rule set and the foreground exe (pure function — no side effects).
///
/// Rules are aggregated per key (process|device), so even several rules with the same process+device
/// count as one transition. `states` is the record of keys that were active last time;
/// even if a rule is deleted or edited and nothing is left, it can be restored from the recorded value.
fn eval_rules(
    rules: &[ProcRule],
    fg: &str,
    states: &std::collections::HashMap<String, ProcApplied>,
) -> ProcEval {
    // Aggregate enabled & matching rules by key (keeping order of appearance).
    let mut order: Vec<String> = vec![];
    let mut contributors: std::collections::HashMap<String, Vec<&ProcRule>> =
        std::collections::HashMap::new();
    for r in rules {
        if r.enabled && rule_matches(r, fg) {
            let k = rule_key(r);
            contributors.entry(k.clone()).or_default().push(r);
            if !order.contains(&k) {
                order.push(k);
            }
        }
    }

    let mut ev = ProcEval::default();
    let mut emitted: std::collections::HashSet<String> = Default::default();

    // Leave (was active before but has no contributors now).
    // Keys whose rule still exists fire in rule order, and on_leave prefers
    // the latest definition of the same key (the last non-empty one) — otherwise the recorded value.
    for r in rules {
        let k = rule_key(r);
        if !states.contains_key(&k) || contributors.contains_key(&k) || !emitted.insert(k.clone()) {
            continue;
        }
        let name = rules
            .iter()
            .rev()
            .find(|r2| rule_key(r2) == k && !r2.on_leave.is_empty())
            .map(|r2| r2.on_leave.clone())
            .unwrap_or_else(|| states[&k].on_leave.clone());
        if !name.is_empty() {
            ev.leaves.push((states[&k].device.clone(), name));
        }
    }
    // Keys whose rule is gone (rule deleted) come last — restored with the recorded value.
    let mut orphans: Vec<&String> = states
        .keys()
        .filter(|k| !emitted.contains(*k) && !contributors.contains_key(*k))
        .collect();
    orphans.sort();
    for k in orphans {
        if !states[k].on_leave.is_empty() {
            ev.leaves
                .push((states[k].device.clone(), states[k].on_leave.clone()));
        }
    }

    // Enter/continue. The on_leave record is the last non-empty one among the contributing rules.
    for k in order {
        let rs = &contributors[&k];
        let leave = rs
            .iter()
            .rev()
            .find(|r| !r.on_leave.is_empty())
            .map(|r| r.on_leave.clone())
            .unwrap_or_default();
        let device = rs[0].device.clone();
        ev.now.insert(
            k.clone(),
            ProcApplied {
                device,
                on_leave: leave,
            },
        );
        if states.contains_key(&k) {
            continue; // continue — do not re-apply.
        }
        // New: apply on_fg of the contributing rules in order (consecutive duplicates are merged).
        let mut fgs: Vec<String> = vec![];
        for r in rs {
            if !r.on_fg.is_empty() && fgs.last() != Some(&r.on_fg) {
                fgs.push(r.on_fg.clone());
            }
        }
        if !fgs.is_empty() {
            ev.enters.push((rs[0].device.clone(), fgs));
        }
    }
    ev
}

impl Shell {
    /// Whether the device can accept writes now (loaded, not switching, awake).
    fn device_applyable(&self, id: &DeviceId) -> bool {
        let Some(s) = self.hub.session(id) else {
            return false;
        };
        if !s.ready() || s.switching_profile {
            return false;
        }
        // A sleeping real device times out on writes (mf8 family) → wait for it to wake.
        !self.hub.devices.iter().any(|d| d.id == *id && d.asleep)
    }

    /// Sends `changes` from the head with `hub.apply` as far as they are accepted, and puts the rest in `proc_pending`.
    /// The return value is (number sent, number held).
    ///
    /// Even if `Change::Profile` (configuration switch) is included midway, everything after it is held as soon as
    /// the switch starts, so the order is preserved.
    pub fn apply_changes(&mut self, id: &DeviceId, changes: Vec<Change>) -> (usize, usize) {
        let n = changes.len();
        let mut i = 0;
        while i < n && self.device_applyable(id) {
            self.hub.apply(id, changes[i].clone());
            i += 1;
        }
        if i < n {
            // If the same device already has a backlog, append to the tail of its queue (keeps order, and
            // the entry count does not grow even if transitions repeat while it stays disconnected).
            let rest = changes[i..].to_vec();
            if let Some((_, q)) = self.proc_pending.iter_mut().find(|(d, _)| *d == id.0) {
                q.extend(rest);
            } else {
                self.proc_pending.push((id.0.clone(), rest));
            }
        }
        (i, n - i)
    }

    /// Flushes held changes to devices that can now accept them (called every proc_tick).
    fn proc_drain(&mut self, cx: &mut Context<Self>) {
        if self.proc_pending.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.proc_pending);
        let mut progress = false;
        for (did, q) in pending.iter_mut() {
            let id = DeviceId(did.clone());
            // A device that was not connected was plugged in later → open a session and start loading.
            if self.hub.session(&id).is_none()
                && self.hub.devices.iter().any(|d| d.id == id && d.supported)
            {
                self.hub.open(&id, false);
            }
            // Sessions that failed to open (Failed) are reopened every 10 seconds.
            if let Some(s) = self.hub.session(&id) {
                if matches!(s.status, crate::store::Status::Failed(_)) {
                    let due = self
                        .proc_retry
                        .get(did)
                        .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(10));
                    if due {
                        self.proc_retry
                            .insert(did.clone(), std::time::Instant::now());
                        self.hub.reload(&id);
                    }
                }
            }
            let had = q.len();
            while !q.is_empty() && self.device_applyable(&id) {
                let c = q.remove(0);
                self.hub.apply(&id, c);
                progress = true;
            }
            if had > 0 && q.is_empty() {
                self.toast(
                    format!("Applied the queued profile changes ({did})"),
                    false,
                    cx,
                );
            }
        }
        pending.retain(|(_, q)| !q.is_empty());
        self.proc_pending = pending;
        if progress {
            cx.notify();
        }
    }

    /// Applies by profile name (filters by the rule's device and sends the whole bundle).
    /// Returns (sent, held, total). None if not found.
    fn apply_profile_named(&mut self, device: &str, name: &str) -> Option<(usize, usize, usize)> {
        let profile = self
            .prefs
            .profiles
            .iter()
            .find(|p| p.name == name && p.devices.iter().any(|t| t.device == device))
            .cloned()?;
        let (mut sent, mut held, mut n) = (0usize, 0usize, 0usize);
        for t in &profile.devices {
            let id = DeviceId(t.device.clone());
            // Open the session if possible. Even if not connected / not ready, do not give up; the changes
            // are put in the held queue by apply_changes (drained after connection, wake-up and loading).
            if self.hub.session(&id).is_none()
                && self.hub.devices.iter().any(|d| d.id == id && d.supported)
            {
                self.hub.open(&id, false);
            }
            n += t.changes.len();
            let (s, h) = self.apply_changes(&id, t.changes.clone());
            sent += s;
            held += h;
        }
        Some((sent, held, n))
    }

    /// Called about every second. Applies profiles on foreground transitions.
    ///
    /// - Ticks where resolving the foreground exe fails are not evaluated for transitions (prevents all rules being
    ///   misjudged as "left" and restores being written when it becomes None, e.g. on a high-privilege window).
    /// - Evaluate only when the foreground is the same for 2 consecutive ticks (no EEPROM writes for the
    ///   intermediate state of fast alt-tab back and forth = flash protection).
    /// - Transitions go restore → new, so even when moving directly between managed apps the new side wins.
    pub fn proc_tick(&mut self, cx: &mut Context<Self>) {
        self.proc_drain(cx);
        if self.prefs.proc_rules.is_empty() {
            // All rules deleted = no longer managed. Whatever was applied is restored with the recorded value.
            let states = std::mem::take(&mut self.proc_states);
            if !states.is_empty() {
                let mut restored = 0;
                for a in states.values() {
                    if !a.on_leave.is_empty()
                        && self.apply_profile_named(&a.device, &a.on_leave).is_some()
                    {
                        restored += 1;
                    }
                }
                if restored > 0 {
                    self.toast(
                        format!("Restored: applied {restored} profile(s) because the rules were removed"),
                        false,
                        cx,
                    );
                    cx.notify();
                }
            }
            self.proc_fg = None;
            return;
        }
        let fg = foreground_app().filter(|s| !s.is_empty());
        if fg != self.proc_fg {
            self.proc_fg = fg;
            return;
        }
        let Some(fg) = fg else { return };
        let rules = self.prefs.proc_rules.clone();
        let ev = eval_rules(&rules, &fg, &self.proc_states);
        let changed = !ev.leaves.is_empty() || !ev.enters.is_empty();
        let mut msgs: Vec<String> = vec![];
        let detail = |res: Option<(usize, usize, usize)>| match res {
            None => "not found".to_string(),
            Some((sent, held, n)) => {
                if held > 0 {
                    if sent > 0 {
                        format!("{sent}/{n} changes, rest queued")
                    } else {
                        format!("{n} changes queued")
                    }
                } else {
                    format!("{n} changes")
                }
            }
        };
        for (device, name) in &ev.leaves {
            let res = self.apply_profile_named(device, name);
            msgs.push(format!("Restored \"{name}\": {}", detail(res)));
        }
        for (device, names) in &ev.enters {
            for name in names {
                let res = self.apply_profile_named(device, name);
                msgs.push(format!("Applied \"{name}\": {}", detail(res)));
            }
        }
        self.proc_states = ev.now;
        if !msgs.is_empty() {
            self.toast(format!("{} — {fg}", msgs.join(" → ")), false, cx);
        }
        if changed {
            cx.notify();
        }
    }

    /// Whether it currently matches the rule (for status display in the UI).
    pub fn proc_rule_active(&self, idx: usize) -> bool {
        self.prefs
            .proc_rules
            .get(idx)
            .is_some_and(|r| self.proc_states.contains_key(&rule_key(r)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::{Prefs, ProcRule, SavedProfile};

    fn rule(process: &str) -> ProcRule {
        ProcRule {
            process: process.into(),
            device: "dev".into(),
            on_fg: "fps".into(),
            on_leave: String::new(),
            enabled: true,
        }
    }

    #[test]
    fn rule_match_is_case_insensitive_substring() {
        assert!(rule_matches(
            &rule("valorant"),
            "valorant-win64-shipping.exe"
        ));
        assert!(rule_matches(&rule("VALORANT"), "valorant.exe"));
        assert!(rule_matches(&rule("notepad"), "notepad.exe"));
        assert!(!rule_matches(&rule("valorant"), "chrome.exe"));
        assert!(!rule_matches(&rule("  "), "anything.exe"));
        assert!(!rule_matches(&rule(""), ""));
    }

    #[test]
    fn prefs_profiles_roundtrip() {
        let mut p = Prefs::default();
        p.profiles.push(SavedProfile {
            name: "fps".into(),
            devices: vec![
                crate::prefs::ProfileTarget {
                    device: "dev0".into(),
                    changes: vec![devhub_core::Change::Profile(1)],
                },
                crate::prefs::ProfileTarget {
                    device: "dev1".into(),
                    changes: vec![],
                },
            ],
        });
        p.proc_rules.push(rule("game.exe"));
        let s = serde_json::to_string(&p).unwrap();
        let q: Prefs = serde_json::from_str(&s).unwrap();
        assert_eq!(q.profiles.len(), 1);
        assert_eq!(q.profiles[0].name, "fps");
        assert_eq!(q.profiles[0].devices.len(), 2);
        assert_eq!(q.profiles[0].devices[0].changes.len(), 1);
        assert_eq!(q.proc_rules[0].process, "game.exe");
        assert_eq!(q.proc_rules[0].on_leave, "");
    }

    #[test]
    fn prefs_old_single_device_profile_migrates() {
        // Profiles in the old format (device + changes written directly) are migrated to devices.
        let q: Prefs = serde_json::from_str(
            r#"{"profiles":[{"name":"fps","device":"dev0","changes":[{"Profile":1}]}]}"#,
        )
        .unwrap();
        assert_eq!(q.profiles.len(), 1);
        assert_eq!(q.profiles[0].devices.len(), 1);
        assert_eq!(q.profiles[0].devices[0].device, "dev0");
        assert_eq!(q.profiles[0].devices[0].changes.len(), 1);
    }

    #[test]
    fn prefs_old_file_without_new_fields_loads() {
        // An old prefs.json (without profiles/proc_rules) can be loaded.
        let q: Prefs = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert!(q.profiles.is_empty());
        assert!(q.proc_rules.is_empty());
    }

    // ---------------- eval_rules ----------------

    fn states_of(entries: &[(&str, &str, &str)]) -> std::collections::HashMap<String, ProcApplied> {
        entries
            .iter()
            .map(|(k, d, l)| {
                (
                    (*k).to_string(),
                    ProcApplied {
                        device: (*d).into(),
                        on_leave: (*l).into(),
                    },
                )
            })
            .collect()
    }

    fn rule_full(process: &str, device: &str, on_fg: &str, on_leave: &str) -> ProcRule {
        ProcRule {
            process: process.into(),
            device: device.into(),
            on_fg: on_fg.into(),
            on_leave: on_leave.into(),
            enabled: true,
        }
    }

    #[test]
    fn eval_enter_then_leave() {
        let rules = vec![rule_full("valorant", "mouse", "fps", "desk")];
        // Enter: states empty → on_fg goes to enters, on_leave is recorded.
        let ev = eval_rules(&rules, "valorant.exe", &Default::default());
        assert_eq!(ev.enters, vec![("mouse".into(), vec!["fps".into()])]);
        assert!(ev.leaves.is_empty());
        assert_eq!(ev.now["valorant|mouse"].on_leave, "desk");
        // Continue: nothing happens in the same state.
        let ev = eval_rules(&rules, "valorant.exe", &ev.now);
        assert!(ev.enters.is_empty() && ev.leaves.is_empty());
        // Leave: when fg changes, on_leave goes to leaves.
        let ev = eval_rules(&rules, "chrome.exe", &ev.now);
        assert_eq!(ev.leaves, vec![("mouse".into(), "desk".into())]);
        assert!(ev.now.is_empty());
    }

    #[test]
    fn eval_leave_before_enter_new_app_wins() {
        // valorant→mouse(A) and apex→mouse(B). Direct switch from valorant to apex.
        let rules = vec![
            rule_full("apex", "mouse", "b_prof", "desk_b"),
            rule_full("valorant", "mouse", "a_prof", "desk_a"),
        ];
        let states = states_of(&[("valorant|mouse", "mouse", "desk_a")]);
        let ev = eval_rules(&rules, "apex.exe", &states);
        // leaves come first, enters after → apex (B) writes last and wins.
        assert_eq!(ev.leaves, vec![("mouse".into(), "desk_a".into())]);
        assert_eq!(ev.enters, vec![("mouse".into(), vec!["b_prof".into()])]);
        assert_eq!(ev.now["apex|mouse"].on_leave, "desk_b");
    }

    #[test]
    fn eval_deleted_rule_restores_stored_leave() {
        // Even if a rule is deleted, what was applied is restored from the recorded value.
        let rules = vec![rule_full("other", "mouse", "x", "")];
        let states = states_of(&[("valorant|mouse", "mouse", "desk")]);
        let ev = eval_rules(&rules, "other.exe", &states);
        assert_eq!(ev.leaves, vec![("mouse".into(), "desk".into())]);
    }

    #[test]
    fn eval_disabled_rule_leaves_with_live_on_leave() {
        // Disabling also counts as leaving. If on_leave was edited, use the latest value.
        let mut r = rule_full("valorant", "mouse", "fps", "desk_new");
        r.enabled = false;
        let states = states_of(&[("valorant|mouse", "mouse", "desk_old")]);
        let ev = eval_rules(&[r], "valorant.exe", &states);
        assert_eq!(ev.leaves, vec![("mouse".into(), "desk_new".into())]);
    }

    #[test]
    fn eval_same_key_rules_aggregate() {
        // Two rules with the same process+device → aggregated into one transition, on_fg applied in order.
        let rules = vec![
            rule_full("valorant", "mouse", "a", "desk"),
            rule_full("valorant", "mouse", "b", "desk2"),
        ];
        let ev = eval_rules(&rules, "valorant.exe", &Default::default());
        assert_eq!(ev.enters.len(), 1);
        assert_eq!(ev.enters[0].1, vec!["a".to_string(), "b".to_string()]);
        // Even if one is disabled, the key is still alive → no wrong restore.
        let mut r2 = rules.clone();
        r2[0].enabled = false;
        let ev = eval_rules(&r2, "valorant.exe", &ev.now);
        assert!(ev.leaves.is_empty());
        assert!(ev.enters.is_empty());
        // Both disabled → leave. on_leave is the last non-empty "desk2".
        r2[1].enabled = false;
        let ev = eval_rules(&r2, "valorant.exe", &ev.now);
        assert_eq!(ev.leaves, vec![("mouse".into(), "desk2".into())]);
    }

    #[test]
    fn eval_empty_on_leave_no_restore() {
        // A rule with empty on_leave emits nothing on leaving.
        let rules = vec![rule_full("valorant", "mouse", "fps", "")];
        let ev = eval_rules(&rules, "valorant.exe", &Default::default());
        let ev = eval_rules(&rules, "chrome.exe", &ev.now);
        assert!(ev.leaves.is_empty());
    }
}
