//! Automation — recording/applying settings profiles, and process-linked rules.
//!
//! - Profiles: record changes made on a device screen as a `Change` list, and replay them in order
//!   with one click or process monitoring.
//! - Process monitor: while the exe name is in the foreground, apply the `on_fg` profile,
//!   and apply `on_leave` when it leaves (procmon.rs evaluates about every second).

use crate::app::Shell;
use crate::prefs::SavedProfile;
use crate::ui::{self, Btn};
use crate::{procmon, theme};
use devhub_core::DeviceId;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

/// Selection state of the new profile / rule form.
pub struct AutoForm {
    /// Target devices of the new profile (indexes into the supported device list, multi-select).
    pub new_devs: std::collections::BTreeSet<usize>,
    /// Target device of the rule.
    pub rule_dev: usize,
    /// The rule's on_fg profile (usize::MAX = not selected).
    pub rule_fg: usize,
    /// The rule's on_leave profile (usize::MAX = none).
    pub rule_leave: usize,
    /// Rule being edited (if Some, the form is in update mode).
    pub edit_rule: Option<usize>,
}

impl Default for AutoForm {
    fn default() -> Self {
        Self {
            new_devs: std::collections::BTreeSet::new(),
            rule_dev: 0,
            rule_fg: usize::MAX,
            rule_leave: usize::MAX,
            edit_rule: None,
        }
    }
}

/// List of supported devices (same order and display names as the sidebar).
fn dev_list(shell: &Shell) -> Vec<(DeviceId, String)> {
    let mut devices = shell.hub.devices.clone();
    devices.sort_by_key(|d| {
        (
            !d.supported,
            d.kind != devhub_core::DeviceKind::Mouse,
            d.name.clone(),
        )
    });
    devices
        .into_iter()
        .filter(|d| d.supported)
        .map(|d| {
            let name = shell
                .prefs
                .nicknames
                .get(&d.id.0)
                .cloned()
                .unwrap_or_else(|| d.name.clone());
            (d.id, name)
        })
        .collect()
}

/// Row of selection chips (for choosing a device / profile).
fn chips_row(
    id_prefix: &str,
    items: &[String],
    sel: usize,
    cx: &mut Context<Shell>,
    on: Rc<dyn Fn(&mut Shell, usize, &mut Context<Shell>)>,
) -> Div {
    chips_row_impl(id_prefix, items, |i| i == sel, cx, on)
}

/// Row of multi-select chips (for the target devices of a new profile).
fn chips_row_multi(
    id_prefix: &str,
    items: &[String],
    sel: &std::collections::BTreeSet<usize>,
    cx: &mut Context<Shell>,
    on: Rc<dyn Fn(&mut Shell, usize, &mut Context<Shell>)>,
) -> Div {
    chips_row_impl(id_prefix, items, |i| sel.contains(&i), cx, on)
}

fn chips_row_impl(
    id_prefix: &str,
    items: &[String],
    is_sel: impl Fn(usize) -> bool,
    cx: &mut Context<Shell>,
    on: Rc<dyn Fn(&mut Shell, usize, &mut Context<Shell>)>,
) -> Div {
    let p = theme::pal();
    let mut row = div().flex().flex_wrap().gap(px(6.));
    for (i, label) in items.iter().enumerate() {
        let on_i = is_sel(i);
        let on2 = on.clone();
        row = row.child(
            div()
                .id(ElementId::Name(format!("{id_prefix}-{i}").into()))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(if on_i {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if on_i { p.on_accent } else { p.sub })
                .bg(if on_i {
                    theme::accent_fill()
                } else {
                    theme::alpha(p.faint, 0.08).into()
                })
                .border_1()
                .border_color(if on_i { p.accent } else { p.hair })
                .hover(|s| s.bg(theme::alpha(p.faint, 0.16)))
                .child(label.clone())
                .on_click(cx.listener(move |this, _, _, cx| on2(this, i, cx))),
        );
    }
    row
}

impl Shell {
    /// Apply a saved profile right now (what cannot be accepted goes to the pending queue).
    /// A bundle with several target devices is sent to all of them.
    pub fn apply_profile(&mut self, pi: usize, cx: &mut Context<Self>) {
        let Some(p) = self.prefs.profiles.get(pi).cloned() else {
            return;
        };
        if p.devices.is_empty() {
            self.toast("No target devices", true, cx);
            return;
        }
        let (mut sent, mut held, mut n) = (0usize, 0usize, 0usize);
        for t in &p.devices {
            let id = DeviceId(t.device.clone());
            if self.hub.session(&id).is_none()
                && self.hub.devices.iter().any(|d| d.id == id && d.supported)
            {
                self.hub.open(&id, false);
            }
            n += t.changes.len();
            // Items for devices that are not connected / not loaded are queued as pending (drained after reconnect).
            let (s, h) = self.apply_changes(&id, t.changes.clone());
            sent += s;
            held += h;
        }
        let msg = if held > 0 {
            format!("Profile \"{}\": {sent}/{n} applied, {held} queued", p.name)
        } else {
            format!("Applied profile \"{}\" ({n} changes)", p.name)
        };
        self.toast(msg, false, cx);
        cx.notify();
    }

    /// Record the device's current settings into the profile (snapshot).
    /// Put the current values of each loaded target device into its change list.
    /// Structural items such as key assignments / buttons / macros are not included (see snapshot_changes).
    pub fn capture_profile(&mut self, pi: usize, cx: &mut Context<Self>) {
        let Some(prof) = self.prefs.profiles.get(pi).cloned() else {
            return;
        };
        let (mut got, mut total, mut skipped) = (0usize, 0usize, 0usize);
        for t in &prof.devices {
            let Some(s) = self.hub.session(&DeviceId(t.device.clone())) else {
                skipped += 1;
                continue;
            };
            let (Some(caps), Some(state)) = (s.caps.as_ref(), s.state.as_ref()) else {
                skipped += 1;
                continue;
            };
            let changes = crate::store::snapshot_changes(caps, state);
            total += changes.len();
            got += 1;
            if let Some(p) = self.prefs.profiles.get_mut(pi) {
                if let Some(t2) = p.devices.iter_mut().find(|x| x.device == t.device) {
                    t2.changes = changes;
                }
            }
        }
        if got == 0 {
            self.toast(
                "Open the target device and let it finish loading first",
                true,
                cx,
            );
            return;
        }
        self.prefs.save();
        let msg = if skipped > 0 {
            format!(
                "Recorded current settings to \"{}\" ({got} devices, {total} changes, {skipped} not loaded and left as is)",
                prof.name
            )
        } else {
            format!(
                "Recorded current settings to \"{}\" ({total} changes)",
                prof.name
            )
        };
        self.toast(msg, false, cx);
        cx.notify();
    }

    /// For the rule form: profile names of the given device ("None" first).
    fn profile_names(
        &self,
        dev_idx: usize,
        devs: &[(DeviceId, String)],
        allow_none: bool,
    ) -> Vec<String> {
        let Some((id, _)) = devs.get(dev_idx) else {
            return vec![];
        };
        let mut v: Vec<String> = self
            .prefs
            .profiles
            .iter()
            .filter(|p| p.devices.iter().any(|t| t.device == id.0))
            .map(|p| p.name.clone())
            .collect();
        if allow_none {
            v.insert(0, "None".into());
        }
        v
    }

    pub fn page_auto(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let devs = dev_list(self);
        let fg_now = procmon::foreground_app().unwrap_or_else(|| "(unknown)".into());
        let running = procmon::running_apps();
        let running_set: std::collections::HashSet<String> =
            running.iter().map(|(e, _)| e.clone()).collect();
        let mut page = div().flex().flex_col().gap(px(14.)).w_full();

        // ================= Profiles =================
        let mut prof_body = ui::body();
        if self.prefs.profiles.is_empty() {
            prof_body = prof_body.child(ui::faint(
                "No profiles yet. Create one with the form below, or start \"Record\" on the device screen.",
            ));
        }
        for (i, prof) in self.prefs.profiles.clone().iter().enumerate() {
            let dev_name = prof
                .devices
                .iter()
                .map(|t| {
                    devs.iter()
                        .find(|(id, _)| id.0 == t.device)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| t.device.clone())
                })
                .collect::<Vec<_>>()
                .join(", ");
            let recording = self.rec_profile == Some(i);
            let mut btns = div().flex().items_center().gap(px(8.));
            if recording {
                btns = btns.child(ui::chip("Recording", p.warn));
            }
            let help_txt = if recording {
                "Recording: changes made on the target device screen are recorded automatically"
                    .to_string()
            } else {
                format!(
                    "{} changes",
                    prof.devices.iter().map(|t| t.changes.len()).sum::<usize>()
                )
            };
            let row = ui::row_first(
                format!("{} ({dev_name})", prof.name).as_str(),
                Some(help_txt.as_str()),
                btns.child(ui::button(
                    format!("prof-apply-{i}"),
                    "Apply",
                    Some("play"),
                    Btn::Secondary,
                    cx,
                    move |this, _, cx| this.apply_profile(i, cx),
                ))
                .child(ui::button(
                    format!("prof-cap-{i}"),
                    "Save current",
                    Some("save"),
                    Btn::Ghost,
                    cx,
                    move |this, _, cx| this.capture_profile(i, cx),
                ))
                .child(ui::button(
                    format!("prof-ren-{i}"),
                    "Rename",
                    Some("pencil"),
                    Btn::Ghost,
                    cx,
                    move |this, w, cx| {
                        let Some(old) = this
                            .prefs
                            .profiles
                            .get(i)
                            .map(|p| p.name.clone())
                        else {
                            return;
                        };
                        let old_name = old.clone();
                        this.modal = Some(crate::app::Modal::Text {
                            title: "Rename profile".into(),
                            on_ok: Rc::new(move |this, v, _, cx| {
                                let v = v.trim().to_string();
                                if v.is_empty() || v == old {
                                    return;
                                }
                                if this.prefs.profiles.iter().any(|p| p.name == v) {
                                    this.toast("A profile with that name already exists", true, cx);
                                    return;
                                }
                                // Rule references follow along (profile names are unique).
                                for r in this.prefs.proc_rules.iter_mut() {
                                    if r.on_fg == old {
                                        r.on_fg = v.clone();
                                    }
                                    if r.on_leave == old {
                                        r.on_leave = v.clone();
                                    }
                                }
                                // The restore target recorded by an active rule follows as well.
                                for a in this.proc_states.values_mut() {
                                    if a.on_leave == old {
                                        a.on_leave = v.clone();
                                    }
                                }
                                if let Some(p) = this.prefs.profiles.get_mut(i) {
                                    p.name = v;
                                }
                                this.prefs.save();
                                cx.notify();
                            }),
                        });
                        // Fill the input with the current value (if not created yet, create with the current value).
                        this.input("modal", &old_name, "", w, cx);
                        this.set_input("modal", &old_name, w, cx);
                        cx.notify();
                    },
                ))
                .child(ui::button(
                    format!("prof-rec-{i}"),
                    if recording { "Stop" } else { "Record" },
                    Some("circle"),
                    if recording { Btn::Danger } else { Btn::Ghost },
                    cx,
                    move |this, _, cx| {
                        if this.rec_profile == Some(i) {
                            this.rec_profile = None;
                            let n = this
                                .prefs
                                .profiles
                                .get(i)
                                .map(|p| p.devices.iter().map(|t| t.changes.len()).sum::<usize>())
                                .unwrap_or(0);
                            this.toast(format!("Recording stopped: {n} changes recorded"), false, cx);
                        } else {
                            this.rec_profile = Some(i);
                            let name = this
                                .prefs
                                .profiles
                                .get(i)
                                .map(|p| p.name.clone())
                                .unwrap_or_default();
                            this.toast(
                                format!("Recording started: \"{name}\" — changes on device screens are recorded"),
                                false,
                                cx,
                            );
                        }
                        cx.notify();
                    },
                ))
                .child(ui::button(
                    format!("prof-del-{i}"),
                    "Delete",
                    Some("trash-2"),
                    Btn::Ghost,
                    cx,
                    move |this, _, cx| {
                        this.prefs.profiles.remove(i);
                        if this.rec_profile == Some(i) {
                            this.rec_profile = None;
                        } else if let Some(r) = this.rec_profile {
                            if r > i {
                                this.rec_profile = Some(r - 1);
                            }
                        }
                        this.prefs.save();
                        cx.notify();
                    },
                )),
            );
            prof_body = prof_body.child(row);
        }
        // New profile form — several target devices can be picked, and the profile becomes a bundle
        // applied to all of them at once (recording sorts changes to each device in the targets).
        let dev_names: Vec<String> = devs.iter().map(|(_, n)| n.clone()).collect();
        let new_sel = self.auto_form.new_devs.clone();
        prof_body = prof_body.child(ui::row(
            "Target devices",
            Some("Pick several — applied to all of them together"),
            div().child(chips_row_multi(
                "prof-dev",
                &dev_names,
                &new_sel,
                cx,
                Rc::new(|this, i, cx| {
                    let s = &mut this.auto_form.new_devs;
                    if !s.remove(&i) {
                        s.insert(i);
                    }
                    cx.notify();
                }),
            )),
        ));
        let name_in = self.input(
            "auto-prof-name",
            "",
            "Profile name (e.g. FPS / video editing)",
            window,
            cx,
        );
        prof_body = prof_body.child(ui::row_first(
            "New profile",
            Some("Name it and create → collect changes with Record"),
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(220.))
                        .child(gpui_kit::component::input::Input::new(&name_in)),
                )
                .child(ui::button(
                    "prof-new",
                    "Create",
                    Some("plus"),
                    Btn::Primary,
                    cx,
                    |this, w, cx| {
                        let name = this.input_value("auto-prof-name", cx).unwrap_or_default();
                        let name = name.trim().to_string();
                        if name.is_empty() {
                            this.toast("Enter a profile name", true, cx);
                            return;
                        }
                        if this.prefs.profiles.iter().any(|p| p.name == name) {
                            this.toast("A profile with that name already exists", true, cx);
                            return;
                        }
                        let devs = dev_list(this);
                        let targets: Vec<crate::prefs::ProfileTarget> = this
                            .auto_form
                            .new_devs
                            .iter()
                            .filter_map(|&i| devs.get(i))
                            .map(|(id, _)| crate::prefs::ProfileTarget {
                                device: id.0.clone(),
                                changes: vec![],
                            })
                            .collect();
                        if targets.is_empty() {
                            this.toast("Select a target device", true, cx);
                            return;
                        }
                        this.prefs.profiles.push(SavedProfile {
                            name,
                            devices: targets,
                        });
                        this.prefs.save();
                        this.set_input("auto-prof-name", "", w, cx);
                        this.auto_form.new_devs.clear();
                        cx.notify();
                    },
                )),
        ));
        page = page.child(ui::card_titled("Settings profiles", "layers", None).child(prof_body));

        // ================= Process monitor =================
        let mut rule_body = ui::body().child(ui::row(
            "Current foreground app",
            Some("Partial match on exe name · with several matches, later rules take priority · switches once the foreground is stable (~2 s)"),
            div().child(ui::chip(fg_now.clone(), p.accent)),
        ));
        if self.prefs.proc_rules.is_empty() {
            rule_body = rule_body.child(ui::faint(
                "No rules yet. Register the exe name of a game etc., and the profile is applied automatically only while that app is in the foreground.",
            ));
        }
        for (i, rule) in self.prefs.proc_rules.clone().iter().enumerate() {
            let active = self.proc_rule_active(i);
            let is_running = running_set.contains(&rule.process)
                || running_set
                    .iter()
                    .any(|e| e.contains(rule.process.as_str()) && !rule.process.is_empty());
            let editing = self.auto_form.edit_rule == Some(i);
            let dev_name = devs
                .iter()
                .find(|(id, _)| id.0 == rule.device)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| rule.device.clone());
            let fg_name = self
                .prefs
                .profiles
                .iter()
                .find(|p| p.name == rule.on_fg && p.devices.iter().any(|t| t.device == rule.device))
                .map(|_| rule.on_fg.clone())
                .unwrap_or_else(|| format!("{} (not found)", rule.on_fg));
            let leave_name = if rule.on_leave.is_empty() {
                "None".to_string()
            } else {
                self.prefs
                    .profiles
                    .iter()
                    .find(|p| {
                        p.name == rule.on_leave && p.devices.iter().any(|t| t.device == rule.device)
                    })
                    .map(|_| rule.on_leave.clone())
                    .unwrap_or_else(|| format!("{} (not found)", rule.on_leave))
            };
            rule_body = rule_body.child(ui::row_first(
                format!("{} → {}", rule.process, dev_name).as_str(),
                Some(format!("On foreground: {fg_name} / On leave: {leave_name}").as_str()),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(if active {
                        ui::chip("Active", p.ok)
                    } else if !rule.enabled {
                        ui::chip("Disabled", p.faint)
                    } else if is_running {
                        ui::chip("Running", p.accent)
                    } else {
                        ui::chip("Waiting", p.faint)
                    })
                    .child(ui::toggle(
                        format!("rule-en-{i}"),
                        rule.enabled,
                        cx,
                        move |this, v, cx| {
                            if let Some(r) = this.prefs.proc_rules.get_mut(i) {
                                r.enabled = v;
                            }
                            this.prefs.save();
                            cx.notify();
                        },
                    ))
                    .child(ui::button(
                        format!("rule-edit-{i}"),
                        if editing { "Close" } else { "Edit" },
                        Some("pencil"),
                        if editing { Btn::Secondary } else { Btn::Ghost },
                        cx,
                        move |this, w, cx| {
                            if this.auto_form.edit_rule == Some(i) {
                                this.auto_form.edit_rule = None;
                                this.set_input("auto-proc-name", "", w, cx);
                                cx.notify();
                                return;
                            }
                            let Some(rule) = this.prefs.proc_rules.get(i).cloned() else {
                                return;
                            };
                            let devs = dev_list(this);
                            if let Some(di) = devs.iter().position(|(id, _)| id.0 == rule.device) {
                                this.auto_form.rule_dev = di;
                            }
                            this.auto_form.rule_fg = usize::MAX;
                            this.auto_form.rule_leave = usize::MAX;
                            // Resolve the profile selection from names (to match the form's index).
                            let dev_idx = this.auto_form.rule_dev.min(devs.len().saturating_sub(1));
                            let fg_items = this.profile_names(dev_idx, &devs, false);
                            if let Some(fi) = fg_items.iter().position(|n| *n == rule.on_fg) {
                                this.auto_form.rule_fg = fi;
                            }
                            let leave_items = this.profile_names(dev_idx, &devs, true);
                            if let Some(li) = leave_items.iter().position(|n| *n == rule.on_leave) {
                                this.auto_form.rule_leave = li;
                            }
                            this.input("auto-proc-name", &rule.process, "", w, cx);
                            this.set_input("auto-proc-name", &rule.process, w, cx);
                            this.auto_form.edit_rule = Some(i);
                            cx.notify();
                        },
                    ))
                    .child(ui::button(
                        format!("rule-del-{i}"),
                        "Delete",
                        Some("trash-2"),
                        Btn::Ghost,
                        cx,
                        move |this, _, cx| {
                            this.prefs.proc_rules.remove(i);
                            if this.auto_form.edit_rule == Some(i) {
                                this.auto_form.edit_rule = None;
                            } else if let Some(e) = this.auto_form.edit_rule {
                                if e > i {
                                    this.auto_form.edit_rule = Some(e - 1);
                                }
                            }
                            this.prefs.save();
                            cx.notify();
                        },
                    )),
            ));
        }
        // Rule add form.
        let rule_dev_sel = self
            .auto_form
            .rule_dev
            .min(dev_names.len().saturating_sub(1));
        let fg_items = self.profile_names(rule_dev_sel, &devs, false);
        let leave_items = self.profile_names(rule_dev_sel, &devs, true);
        // Default when not selected (usize::MAX): on_fg = first profile, on_leave = none (0).
        let fg_sel = if fg_items.is_empty() {
            usize::MAX
        } else if self.auto_form.rule_fg == usize::MAX {
            0
        } else {
            self.auto_form.rule_fg.min(fg_items.len() - 1)
        };
        let leave_sel = if leave_items.is_empty() || self.auto_form.rule_leave == usize::MAX {
            0
        } else {
            self.auto_form.rule_leave.min(leave_items.len() - 1)
        };
        let proc_in = self.input(
            "auto-proc-name",
            "",
            "exe name (e.g. valorant-win64-shipping / partial match OK)",
            window,
            cx,
        );
        // List of running apps → picking one in the dropdown fills the input.
        let proc_items: Vec<crate::app::PopItem> = running
            .iter()
            .map(|(exe, title)| {
                let mut it = crate::app::PopItem::new(exe.clone(), false);
                if !title.is_empty() {
                    it.sub = Some(title.clone());
                }
                it.icon = Some("app-window");
                it
            })
            .collect();
        let editing = self.auto_form.edit_rule;
        let exe_list: Vec<String> = running.iter().map(|(e, _)| e.clone()).collect();
        rule_body = rule_body
            .child(ui::row(
                "Process to watch",
                Some("Pick from running apps, or type the exe name directly (partial match)"),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(260.))
                            .child(gpui_kit::component::input::Input::new(&proc_in)),
                    )
                    .child(ui::select(
                        "auto-proc-pick",
                        proc_items,
                        "Running apps…",
                        220.,
                        cx,
                        move |this, i, w, cx| {
                            if let Some(exe) = exe_list.get(i).cloned() {
                                this.set_input("auto-proc-name", &exe, w, cx);
                            }
                        },
                    ))
                    .child(ui::button(
                        "proc-capture",
                        "Capture foreground",
                        Some("scan"),
                        Btn::Ghost,
                        cx,
                        move |this, w, cx| {
                            if let Some(exe) = procmon::foreground_app() {
                                this.set_input("auto-proc-name", &exe, w, cx);
                            } else {
                                this.toast("Cannot get the foreground app", true, cx);
                            }
                        },
                    )),
            ))
            .child(ui::row(
                "Target device",
                None,
                div().child(chips_row(
                    "rule-dev",
                    &dev_names,
                    rule_dev_sel,
                    cx,
                    Rc::new(|this, i, cx| {
                        this.auto_form.rule_dev = i;
                        // Reset the profile selection when the device changes.
                        this.auto_form.rule_fg = usize::MAX;
                        this.auto_form.rule_leave = usize::MAX;
                        cx.notify();
                    }),
                )),
            ))
            .child(ui::row(
                "Apply when in foreground",
                None,
                div().child(chips_row(
                    "rule-fg",
                    &fg_items,
                    fg_sel,
                    cx,
                    Rc::new(|this, i, cx| {
                        this.auto_form.rule_fg = i;
                        cx.notify();
                    }),
                )),
            ))
            .child(ui::row(
                "Apply when leaving the foreground",
                Some("If \"None\", nothing happens (restore manually)"),
                div().child(chips_row(
                    "rule-leave",
                    &leave_items,
                    leave_sel,
                    cx,
                    Rc::new(|this, i, cx| {
                        this.auto_form.rule_leave = i;
                        cx.notify();
                    }),
                )),
            ))
            .child(ui::row_first(
                if editing.is_some() {
                    "Update rule"
                } else {
                    "Add rule"
                },
                Some(if editing.is_some() {
                    "Overwrites with the form contents"
                } else {
                    "Checks the foreground app about once a second (also works while minimized)"
                }),
                {
                    let commit = Rc::new(
                        |this: &mut Shell, w: &mut Window, cx: &mut Context<Shell>| {
                            let proc = this.input_value("auto-proc-name", cx).unwrap_or_default();
                            let proc = proc.trim().to_lowercase();
                            if proc.is_empty() {
                                this.toast("Enter a process name", true, cx);
                                return;
                            }
                            let devs = dev_list(this);
                            let dev_idx = this.auto_form.rule_dev.min(devs.len().saturating_sub(1));
                            let Some((id, _)) = devs.get(dev_idx) else {
                                return;
                            };
                            let fg_items = this.profile_names(dev_idx, &devs, false);
                            if fg_items.is_empty() {
                                this.toast("Create a profile first", true, cx);
                                return;
                            }
                            let fg_sel = if this.auto_form.rule_fg == usize::MAX {
                                0
                            } else {
                                this.auto_form.rule_fg.min(fg_items.len() - 1)
                            };
                            let leave_items = this.profile_names(dev_idx, &devs, true);
                            let leave_sel = if this.auto_form.rule_leave == usize::MAX {
                                0
                            } else {
                                this.auto_form
                                    .rule_leave
                                    .min(leave_items.len().saturating_sub(1))
                            };
                            let on_fg = fg_items[fg_sel].clone();
                            let on_leave = if leave_items.is_empty() || leave_sel == 0 {
                                String::new()
                            } else {
                                leave_items[leave_sel].clone()
                            };
                            if let Some(ei) = this.auto_form.edit_rule {
                                if let Some(r) = this.prefs.proc_rules.get_mut(ei) {
                                    r.process = proc;
                                    r.device = id.0.clone();
                                    r.on_fg = on_fg;
                                    r.on_leave = on_leave;
                                }
                                this.auto_form.edit_rule = None;
                            } else {
                                this.prefs.proc_rules.push(crate::prefs::ProcRule {
                                    process: proc,
                                    device: id.0.clone(),
                                    on_fg,
                                    on_leave,
                                    enabled: true,
                                });
                            }
                            this.prefs.save();
                            this.set_input("auto-proc-name", "", w, cx);
                            cx.notify();
                        },
                    );
                    let mut b = div().flex().items_center().gap(px(8.)).child(ui::button(
                        if editing.is_some() {
                            "rule-update"
                        } else {
                            "rule-add"
                        },
                        if editing.is_some() { "Update" } else { "Add" },
                        Some(if editing.is_some() { "check" } else { "plus" }),
                        Btn::Primary,
                        cx,
                        move |this, w, cx| commit(this, w, cx),
                    ));
                    if editing.is_some() {
                        b = b.child(ui::button(
                            "rule-cancel",
                            "Cancel",
                            None,
                            Btn::Ghost,
                            cx,
                            |this, w, cx| {
                                this.auto_form.edit_rule = None;
                                this.set_input("auto-proc-name", "", w, cx);
                                cx.notify();
                            },
                        ));
                    }
                    b
                },
            ));
        let pending_n: usize = self.proc_pending.iter().map(|(_, q)| q.len()).sum();
        let proc_trailing = (pending_n > 0)
            .then(|| ui::chip(format!("{pending_n} pending"), p.warn).into_any_element());
        page =
            page.child(ui::card_titled("Process rules", "monitor", proc_trailing).child(rule_body));

        div()
            .id("auto-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::PAGE_GAP))
                    .p(px(theme::PAGE_PAD))
                    .pt(px(theme::PAGE_TOP))
                    .pb(px(theme::PAGE_BOT))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(ui::title("Automation"))
                            .child(ui::sub(
                                "A profile is a bundle of settings saved for a device. Process rules apply it only while the app is in the foreground, and apply the restore profile when the app leaves the foreground.",
                            )),
                    )
                    .child(page),
            )
    }
}
