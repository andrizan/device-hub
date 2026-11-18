//! "General" tab: device info, profiles, backup, other settings and maintenance.

use crate::app::{Modal, Shell, Tab};
use crate::theme;
use crate::ui::{self, Btn};
use devhub_core::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

impl Shell {
    pub fn page_system(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let Some(s) = self.cur() else {
            return div().into_any_element();
        };
        let info = s.info.clone();
        let caps = s.caps.clone().unwrap_or_else(Capabilities::empty);
        let state = s.state.clone();
        let did = s.id.clone();

        // ---- Info
        let kv = |k: &str, v: String| {
            div()
                .flex()
                .py(px(7.))
                .border_t_1()
                .border_color(p.hair)
                .child(
                    div()
                        .w(px(150.))
                        .text_size(px(12.5))
                        .text_color(p.sub)
                        .child(k.to_string()),
                )
                .child(div().flex_1().text_size(px(12.5)).text_color(p.fg).child(v))
        };
        let info_card = ui::card_titled("Device info", "info", None).child(
            ui::body()
                .pb(px(14.))
                .child(kv("Product", format!("{} {}", info.vendor, info.name)))
                .child(kv("VID:PID", format!("{:04X}:{:04X}", info.vid, info.pid)))
                .child(kv(
                    "Firmware",
                    info.firmware.clone().unwrap_or_else(|| "-".into()),
                ))
                .when_some(info.dongle_firmware.clone(), |d, f| {
                    d.child(kv("Receiver", f))
                })
                .when_some(info.serial.clone(), |d, f| d.child(kv("Serial", f)))
                .child(kv(
                    "Driver",
                    match info.driver {
                        DriverKind::Xsyd => "XSYD (hub.xsyd.top compatible)".into(),
                        DriverKind::Mousefit => "mouse.fit compatible".into(),
                        DriverKind::Walkplay => "WalkPlay (peq.szwalkplay.com compatible)".into(),
                        DriverKind::Atk => "ATK HUB compatible".into(),
                        DriverKind::Lamzu => "LAMZU Aurora compatible".into(),
                        DriverKind::Ry => "RongYuan (VID 3151) compatible".into(),
                        DriverKind::Earfun => "EarFun (GAIA / Bluetooth)".into(),
                        DriverKind::Unsupported => "Unsupported".into(),
                    },
                ))
                .child(kv("ID", info.id.0.clone())),
        );

        let model = info.model.clone();
        let presets = self.presets_card(&model, cx);
        let mut cards: Vec<AnyElement> = vec![info_card.into_any_element(), presets];

        // ---- Firmware update (at the top if there is one)
        if let Some(up) = self
            .hub
            .fw_status
            .get(&did)
            .filter(|u| u.new_available)
            .cloned()
        {
            let mut line = "A new firmware has been released".to_string();
            if let Some(v) = &up.version {
                line = format!("{line} ({v})");
            }
            let ago = fmt_ago(up.checked_at);
            let mut body = ui::body()
                .pb(px(14.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(p.fg)
                        .pb(px(6.))
                        .child(line),
                )
                .child(ui::faint(format!("Last checked: {ago}")));
            let mut row = div().flex().items_center().gap(px(8.)).pt(px(10.));
            if let Some(url) = up.url.clone() {
                row = row.child(ui::button(
                    "fw-open",
                    "Open the official page",
                    Some("download"),
                    Btn::Primary,
                    cx,
                    move |_, _, cx| cx.open_url(&url),
                ));
            }
            row = row.child(ui::button(
                "fw-dismiss",
                "Close",
                Some("x"),
                Btn::Glass,
                cx,
                move |this, _, cx| {
                    this.hub.fw_dismiss(&did);
                    cx.notify();
                },
            ));
            body = body.child(row);
            cards.insert(
                0,
                ui::card_titled("Firmware update", "info", None)
                    .child(body)
                    .into_any_element(),
            );
        }

        // ---- Profiles
        if let (Some(pc), Some(st)) = (caps.profiles.clone(), state.as_ref()) {
            if pc.count > 1 {
                let mut body = ui::body().pb(px(14.));
                for i in 0..pc.count {
                    let active = st.profile.active == i;
                    let name = st
                        .profile
                        .names
                        .get(i as usize)
                        .filter(|n| !n.is_empty())
                        .cloned();
                    let shown = name.clone().unwrap_or_else(|| format!("Profile {}", i + 1));
                    let mut row = div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .py(px(8.))
                        .when(i > 0, |d| d.border_t_1().border_color(p.hair))
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(px(9.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(if active { p.accent } else { p.ctrl })
                                .text_size(px(11.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(if active { p.on_accent } else { p.sub })
                                .child(format!("P{}", i + 1)),
                        )
                        .child(div().flex_1().text_size(px(13.)).child(shown.clone()))
                        .when(active, |d| d.child(ui::chip("In use", p.ok)));
                    if !active {
                        row = row.child(ui::button(
                            format!("prof-use-{i}"),
                            "Switch",
                            None,
                            Btn::Glass,
                            cx,
                            move |this, _, cx| {
                                this.apply(Change::Profile(i), cx);
                                if let Some(id) = this.device_id() {
                                    this.hub.reload(&id);
                                }
                            },
                        ));
                    }
                    if pc.renamable {
                        row = row.child(ui::icon_button(
                            format!("prof-ren-{i}"),
                            "pencil",
                            cx,
                            move |this, w, cx| {
                                this.modal = Some(Modal::Text {
                                    title: format!("P{} name", i + 1),
                                    on_ok: Rc::new(move |this, v, _, cx| {
                                        this.apply(
                                            Change::ProfileName {
                                                index: i,
                                                name: v.trim().to_string(),
                                            },
                                            cx,
                                        )
                                    }),
                                });
                                this.input("modal", "", "", w, cx);
                                let cur = name.clone().unwrap_or_default();
                                this.set_input("modal", &cur, w, cx);
                                cx.notify();
                            },
                        ));
                    }
                    body = body.child(row);
                }
                cards.push(
                    ui::card_titled("Onboard profiles", "layers", None)
                        .child(body)
                        .into_any_element(),
                );
            }
        }

        // ---- Backup
        let name = info.name.clone();
        let backup = ui::card_titled("Backup", "hard-drive-download", None).child(
            ui::body()
                .pb(px(12.))
                .child(ui::row_first(
                    "Export settings",
                    Some("Save all current settings to a JSON file"),
                    ui::button(
                        "export",
                        "Export",
                        Some("download"),
                        Btn::Glass,
                        cx,
                        move |this, _, cx| {
                            let Some(st) = this.cur().and_then(|s| s.state.clone()) else {
                                return;
                            };
                            let file = rfd::FileDialog::new()
                                .set_file_name(format!("{}.devhub.json", name.replace(' ', "_")))
                                .add_filter("Device Hub", &["json"])
                                .save_file();
                            if let Some(f) = file {
                                match serde_json::to_string_pretty(&st)
                                    .map(|s| std::fs::write(&f, s))
                                {
                                    Ok(Ok(())) => this.toast(
                                        format!("Saved: {}", f.display()),
                                        false,
                                        cx,
                                    ),
                                    Ok(Err(e)) => this.toast(format!("Save failed: {e}"), true, cx),
                                    Err(e) => this.toast(format!("Save failed: {e}"), true, cx),
                                }
                            }
                        },
                    ),
                ))
                .child(ui::row(
                    "Import settings",
                    Some("Write an exported JSON to this device"),
                    ui::button(
                        "import",
                        "Import",
                        Some("upload"),
                        Btn::Glass,
                        cx,
                        |this, _, cx| {
                            let Some(f) = rfd::FileDialog::new()
                                .add_filter("Device Hub", &["json"])
                                .pick_file()
                            else {
                                return;
                            };
                            let parsed = std::fs::read_to_string(&f)
                                .map_err(|e| e.to_string())
                                .and_then(|s| {
                                    serde_json::from_str::<DeviceState>(&s)
                                        .map_err(|e| e.to_string())
                                });
                            match parsed {
                                Ok(mut st) => {
                                    // Files for a different model or unsupported ones are rejected here.
                                    let compat = this
                                        .device_id()
                                        .and_then(|id| this.hub.session(&id))
                                        .and_then(|s| {
                                            s.caps.clone().zip(s.state.clone())
                                        });
                                    let Some((caps, cur)) = compat else {
                                        this.toast(
                                            "The device's settings have not been read yet",
                                            true,
                                            cx,
                                        );
                                        return;
                                    };
                                    match crate::store::state_compatible(&caps, &st) {
                                        Err(e) => {
                                            this.toast(
                                                format!("Cannot import: {e}"),
                                                true,
                                                cx,
                                            );
                                        }
                                        Ok(warn) => {
                                            // The file is applied only to the currently selected configuration.
                                            st.profile.active = cur.profile.active;
                                            let body = match &warn {
                                                Some(w) => format!(
                                                    "This overwrites the current configuration with the file's contents ({w})."
                                                ),
                                                None => "This overwrites the current configuration with the file's contents."
                                                    .to_string(),
                                            };
                                            this.confirm(
                                                "Import settings",
                                                &body,
                                                "Write",
                                                true,
                                                Rc::new(move |this, _, cx| {
                                                    match this.apply_state(&st, true, cx) {
                                                        crate::app::RestoreOutcome::Applied(
                                                            n,
                                                        ) => this.toast(
                                                            format!("{n} items written"),
                                                            false,
                                                            cx,
                                                        ),
                                                        crate::app::RestoreOutcome::Switching => {
                                                            this.toast(
                                                                "Switching configuration and applying…",
                                                                false,
                                                                cx,
                                                            )
                                                        }
                                                        crate::app::RestoreOutcome::Nothing => this
                                                            .toast(
                                                                "Could not apply",
                                                                true,
                                                                cx,
                                                            ),
                                                    }
                                                }),
                                                cx,
                                            );
                                        }
                                    }
                                }
                                Err(e) => this.toast(format!("Cannot import: {e}"), true, cx),
                            }
                        },
                    ),
                )),
        );
        cards.push(backup.into_any_element());

        cards.extend(self.section_cards(Tab::System, window, cx));
        ui::grid2(cards).into_any_element()
    }

    /// Card for presets saved on the PC.
    fn presets_card(&mut self, model: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = theme::pal();
        let list = crate::presets::list(model);
        let mut body = ui::body().pb(px(12.));
        if list.is_empty() {
            body = body.child(ui::faint("None yet. Save per-game settings and the like, and you can switch between them with one click.").py(px(8.)));
        } else {
            body = body.child(
                ui::faint("A preset is applied only to the currently selected configuration.")
                    .py(px(6.)),
            );
        }
        for (i, pr) in list.iter().enumerate() {
            let (a, b, c) = (pr.clone(), pr.clone(), pr.clone());
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .py(px(7.))
                    .when(i > 0, |d| d.border_t_1().border_color(p.hair))
                    .child(ui::icon("bookmark", 15., p.accent))
                    .child(div().flex_1().text_size(px(13.)).child(pr.name.clone()))
                    .child(ui::button(
                        format!("pre-apply-{i}"),
                        "Apply",
                        Some("check"),
                        Btn::Glass,
                        cx,
                        move |this, _, cx| match crate::presets::load(&a) {
                            Ok(st) => this.apply_preset(&a.name, st, cx),
                            Err(e) => this.toast(format!("Cannot load: {e}"), true, cx),
                        },
                    ))
                    .child(ui::icon_button(
                        format!("pre-ren-{i}"),
                        "pencil",
                        cx,
                        move |this, w, cx| {
                            let b = b.clone();
                            this.modal = Some(Modal::Text {
                                title: "Rename preset".into(),
                                on_ok: Rc::new(move |this, v, _, cx| {
                                    if let Err(e) = crate::presets::rename(&b, v.trim()) {
                                        this.toast(e, true, cx);
                                    }
                                    cx.notify();
                                }),
                            });
                            this.input("modal", "", "", w, cx);
                            this.set_input("modal", &c.name, w, cx);
                            cx.notify();
                        },
                    ))
                    .child(ui::icon_button(format!("pre-del-{i}"), "trash", cx, {
                        let d = pr.clone();
                        move |this, _, cx| {
                            let d = d.clone();
                            this.confirm(
                                "Delete preset",
                                &format!(
                                    "Delete \"{}\" (the device's settings will not change).",
                                    d.name
                                ),
                                "Delete",
                                true,
                                Rc::new(move |_this, _, cx| {
                                    let _ = crate::presets::delete(&d);
                                    cx.notify();
                                }),
                                cx,
                            );
                        }
                    })),
            );
        }
        let model2 = model.to_string();
        let save = ui::button(
            "pre-save",
            "Save current settings",
            Some("plus"),
            Btn::Primary,
            cx,
            move |this, w, cx| {
                let model = model2.clone();
                this.modal = Some(Modal::Text {
                    title: "Save as preset".into(),
                    on_ok: Rc::new(move |this, v, _, cx| {
                        let name = v.trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        let Some(st) = this.cur().and_then(|s| s.state.clone()) else {
                            return;
                        };
                        match crate::presets::save(&model, &name, &st) {
                            Ok(()) => this.toast(format!("Saved \"{name}\""), false, cx),
                            Err(e) => this.toast(format!("Cannot save: {e}"), true, cx),
                        }
                    }),
                });
                this.input("modal", "", "", w, cx);
                this.set_input("modal", "", w, cx);
                cx.notify();
            },
        );
        ui::card_titled(
            "Presets (saved on this PC)",
            "bookmark",
            Some(save.into_any_element()),
        )
        .child(body)
        .into_any_element()
    }
}

/// Relative time for "Last checked".
fn fmt_ago(ts: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let d = now.saturating_sub(ts);
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{} min ago", d / 60)
    } else if d < 86400 {
        format!("{} h ago", d / 3600)
    } else {
        format!("{} d ago", d / 86400)
    }
}
