//! Device screen frame: header (name, connection, profile, sync) + floating tabs + body.

use crate::app::{tabs_for, Modal, PopItem, Shell, Tab};
use crate::store::Status;
use crate::ui::{self, Btn};
use crate::{motion, theme};
use devhub_core::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

impl Shell {
    pub fn page_device(
        &mut self,
        id: &DeviceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(s) = self.hub.session(id) else {
            // It is in the list but has not been opened yet (e.g. right after automatic restore).
            self.hub.open(id, true);
            return div().into_any_element();
        };
        let status = s.status.clone();
        let header = self.device_header(id, cx);

        let body: AnyElement = match status {
            Status::Opening => self.loading_card().into_any_element(),
            Status::Failed(e) => self.failed_card(id, &e, cx).into_any_element(),
            Status::Ready if !self.hub.session(id).is_some_and(|s| s.ready()) => {
                self.loading_card().into_any_element()
            }
            Status::Ready => {
                let caps = self
                    .hub
                    .session(id)
                    .and_then(|s| s.caps.clone())
                    .unwrap_or_else(Capabilities::empty);
                let tabs = tabs_for(&caps);
                if !tabs.contains(&self.tab) {
                    self.tab = tabs[0];
                }
                let sel = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
                let tab_items: Vec<(String, &'static str)> = tabs
                    .iter()
                    .map(|t| (t.label().to_string(), t.icon()))
                    .collect();
                let tabs2 = tabs.clone();
                let bar = ui::tab_bar("dev", &tab_items, sel, cx, move |this, i, cx| {
                    this.tab = tabs2[i];
                    this.recording = None;
                    this.scroll.set_offset(point(px(0.), px(0.)));
                    if let Some(id) = this.device_id() {
                        this.hub.set_travel(&id, false);
                    }
                    cx.notify();
                });
                let content = match self.tab {
                    Tab::Dpi => self.page_dpi(window, cx).into_any_element(),
                    Tab::Buttons => self.page_buttons(cx).into_any_element(),
                    Tab::Performance => self.page_performance(window, cx).into_any_element(),
                    Tab::Keymap => self.page_keymap(window, cx).into_any_element(),
                    Tab::Trigger => self.page_trigger(window, cx).into_any_element(),
                    Tab::Advanced => self.page_advanced(cx).into_any_element(),
                    Tab::Lighting => self.page_lighting(window, cx).into_any_element(),
                    Tab::Live => self.page_live(window, cx).into_any_element(),
                    Tab::Macros => self.page_macros(window, cx).into_any_element(),
                    Tab::Audio => self.page_audio(window, cx).into_any_element(),
                    Tab::Test => self.page_test(window, cx).into_any_element(),
                    Tab::System => self.page_system(window, cx).into_any_element(),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(div().flex().child(bar))
                    .child(content)
                    .into_any_element()
            }
        };

        div()
            .id("device-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::PAGE_GAP))
                    .p(px(theme::PAGE_PAD))
                    .pt(px(theme::PAGE_TOP))
                    .pb(px(theme::PAGE_BOT))
                    .child(header)
                    .child(body),
            )
            .into_any_element()
    }

    fn device_header(&mut self, id: &DeviceId, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let s = self.hub.session(id).expect("session");
        let info = s.info.clone();
        let pending = s.pending;
        let running = s.running.clone();
        let progress = s.progress.clone();
        let caps = s.caps.clone();
        let profile = s.state.as_ref().map(|st| st.profile.clone());
        let name = self
            .prefs
            .nicknames
            .get(&id.0)
            .cloned()
            .unwrap_or_else(|| info.name.clone());
        let icon = match info.kind {
            DeviceKind::Mouse => "mouse",
            DeviceKind::Keyboard => "keyboard",
            DeviceKind::Other if info.driver == DriverKind::Walkplay => "headphones",
            DeviceKind::Other => "usb",
            DeviceKind::Audio => "headphones",
        };
        let mut meta = vec![info.vendor.clone()];
        meta.push(
            match info.transport {
                Transport::Wired => "Wired",
                Transport::Wireless24G => "2.4GHz",
                Transport::Bluetooth => "Bluetooth",
                Transport::Unknown => "Unknown connection",
            }
            .to_string(),
        );
        if let Some(fw) = &info.firmware {
            meta.push(format!("FW {fw}"));
        }
        if let Some(fw) = &info.dongle_firmware {
            meta.push(format!("Receiver {fw}"));
        }

        // Profile switch.
        let profile_ctl = match (caps.as_ref().and_then(|c| c.profiles.clone()), profile) {
            (Some(pc), Some(ps)) if pc.count > 1 => {
                let items: Vec<PopItem> = (0..pc.count)
                    .map(|i| {
                        let n = ps.names.get(i as usize).filter(|n| !n.is_empty()).cloned();
                        let mut it = PopItem::new(
                            n.unwrap_or_else(|| format!("Profile {}", i + 1)),
                            i == ps.active,
                        );
                        it.sub = Some(format!("P{}", i + 1));
                        it
                    })
                    .collect();
                let cur = ps
                    .names
                    .get(ps.active as usize)
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("Profile {}", ps.active + 1));
                let id2 = id.clone();
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(ui::faint("Profile"))
                        .child(ui::select(
                            "profile",
                            items,
                            &cur,
                            190.,
                            cx,
                            move |this, i, _, cx| {
                                this.apply(Change::Profile(i as u8), cx);
                                this.hub.reload(&id2);
                            },
                        )),
                )
            }
            _ => None,
        };

        let sync: AnyElement = if let Some((label, frac)) = progress {
            let p2 = theme::pal();
            if frac.is_none() {
                motion::keep_alive();
            }
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .w(px(180.))
                .child(ui::faint(match frac {
                    Some(f) => format!("{label} {:.0}%", f * 100.),
                    None => format!("{label}…"),
                }))
                .child(
                    div().h(px(5.)).rounded(px(3.)).bg(p2.track).child(
                        div()
                            .h_full()
                            .rounded(px(3.))
                            .w(relative(frac.unwrap_or(0.35)))
                            .bg(p2.accent),
                    ),
                )
                .into_any_element()
        } else if let Some(r) = running {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(ui::sync_dot(true))
                .child(ui::faint(format!("{r}…")))
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(ui::sync_dot(pending > 0))
                .child(ui::faint(if pending > 0 {
                    "Writing"
                } else {
                    "Synced"
                }))
                .into_any_element()
        };
        let id_rename = id.clone();
        let id_reload = id.clone();
        let orig_name = info.name.clone();
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                div()
                    .size(px(60.))
                    .rounded(px(19.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::accent_fill())
                    .child(ui::icon(icon, 30., p.on_accent)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(ui::title(name.clone()))
                            .child(ui::icon_button(
                                "rename",
                                "pencil",
                                cx,
                                move |this, w, cx| {
                                    let idr = id_rename.clone();
                                    let orig = orig_name.clone();
                                    this.modal = Some(Modal::Text {
                                        title: "Change display name".into(),
                                        on_ok: Rc::new(move |this, v, _, cx| {
                                            let v = v.trim().to_string();
                                            if v.is_empty() || v == orig {
                                                this.prefs.nicknames.remove(&idr.0);
                                            } else {
                                                this.prefs.nicknames.insert(idr.0.clone(), v);
                                            }
                                            this.prefs.save();
                                            cx.notify();
                                        }),
                                    });
                                    let cur = this
                                        .prefs
                                        .nicknames
                                        .get(&id_rename.0)
                                        .cloned()
                                        .unwrap_or_else(|| orig_name.clone());
                                    this.input("modal", "", "", w, cx);
                                    this.set_input("modal", &cur, w, cx);
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(ui::faint(meta.join(" · ")))
                            .when_some(
                                info.battery
                                    .and_then(|b| b.percent.map(|x| (x, b.charging))),
                                |d, (x, c)| d.child(ui::battery_pill(x, c)),
                            ),
                    ),
            )
            .when_some(profile_ctl, |d, c| d.child(c))
            .when_some(self.preset_quick(&info.model, cx), |d, e| d.child(e))
            .child(self.undo_redo(cx))
            .child(sync)
            .child(ui::icon_button(
                "reload",
                "refresh-cw",
                cx,
                move |this, _, cx| {
                    this.hub.reload(&id_reload);
                    this.toast("Re-reading from the device", false, cx);
                },
            ))
    }

    fn loading_card(&self) -> impl IntoElement {
        let p = theme::pal();
        motion::keep_alive();
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f32())
            .unwrap_or(0.);
        let mut dots = div().flex().gap(px(6.));
        for i in 0..3 {
            let a = ((t * 4. - i as f32 * 0.6).sin() * 0.5 + 0.5).powf(2.);
            dots = dots.child(
                div()
                    .size(px(8.))
                    .rounded(px(4.))
                    .bg(theme::alpha(p.accent, 0.3 + a * 0.7)),
            );
        }
        ui::card().child(
            div()
                .relative()
                .h(px(220.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .child(dots)
                .child(ui::sub("Reading settings from the device…")),
        )
    }

    fn failed_card(
        &mut self,
        id: &DeviceId,
        err: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = self.hub.session(id);
        let unsupported = s.is_some_and(|s| !s.info.supported);
        let info = s.map(|s| s.info.clone());
        let id2 = id.clone();
        let mut c = div().relative().p(px(20.)).flex().flex_col().items_center();
        if unsupported {
            let i = info.expect("info");
            c = c.child(ui::empty_state(
                "puzzle",
                "This device cannot be configured yet",
                &format!(
                    "{} ({:04X}:{:04X}) was detected, but its protocol driver is not implemented yet. You can add it by following docs/ADD_DEVICE.md.",
                    i.name, i.vid, i.pid
                ),
            ));
        } else {
            c = c
                .child(ui::empty_state("plug-zap", "Cannot communicate with the device", err))
                .child(ui::button(
                    "retry",
                    "Retry",
                    Some("refresh-cw"),
                    Btn::Primary,
                    cx,
                    move |this, _, cx| {
                        this.hub.reload(&id2);
                        cx.notify();
                    },
                ));
        }
        ui::card().child(c)
    }

    pub fn page_performance_legacy(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut cards = vec![];
        // Measured polling rate (arrival rate of input reports counted with Raw Input).
        // In the background Windows 11 throttles raw input, so the value cannot be trusted.
        // The big number is the "peak" (max over the moving segment — details on the Test tab).
        if let Some(st) = self.cur().and_then(|s| s.poll.as_ref()) {
            let bg = !self.last_window_active;
            let p = theme::pal();
            let big = st.peak_hz.or(st.seg_hz).unwrap_or(st.hz);
            cards.push(
                ui::card_titled("Measured polling rate", "gauge", None)
                    .child(
                        ui::body()
                            .pt(px(8.))
                            .pb(px(14.))
                            .flex()
                            .items_baseline()
                            .gap(px(10.))
                            .child(
                                ui::big_number(format!("{big:.0} Hz"), 28.)
                                    .when(bg, |d| d.text_color(p.faint)),
                            )
                            .child(ui::faint(if bg {
                                format!("Background: {:.0} reports/s (approximate)", st.hz)
                            } else if big < 1. {
                                "— Use the device to start measuring".into()
                            } else {
                                let mut s = format!("Peak · avg {:.0}", st.hz);
                                if let Some((med, _)) = st.gap_stats {
                                    s = format!("{s} · est. interval {med:.3}ms",);
                                }
                                s
                            })),
                    )
                    .into_any_element(),
            );
        }
        cards.extend(self.section_cards(Tab::Performance, window, cx));
        ui::grid2(cards)
    }
}

impl Shell {
    /// Undo / redo.
    fn undo_redo(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let (u, r) = (self.can_undo(), self.can_redo());
        let b = |id: &'static str, icon: &str, on: bool| {
            div()
                .id(id)
                .size(px(30.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .when(on, |d| d.cursor_pointer().hover(|s| s.bg(p.ctrl_hover)))
                .child(ui::icon(
                    icon,
                    15.,
                    if on {
                        p.sub
                    } else {
                        theme::alpha(p.faint, 0.35)
                    },
                ))
        };
        div()
            .flex()
            .gap(px(2.))
            .child(b("undo", "undo-2", u).on_click(cx.listener(|this, _, _, cx| this.undo(cx))))
            .child(b("redo", "redo-2", r).on_click(cx.listener(|this, _, _, cx| this.redo(cx))))
    }

    /// Quick preset switching (only when saved presets exist).
    fn preset_quick(&mut self, model: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let list = crate::presets::list(model);
        if list.is_empty() {
            return None;
        }
        let items: Vec<PopItem> = list
            .iter()
            .map(|p| {
                let mut it = PopItem::new(p.name.clone(), false);
                it.icon = Some("bookmark");
                it
            })
            .collect();
        Some(
            ui::select(
                "preset-quick",
                items,
                "Presets",
                150.,
                cx,
                move |this, i, _, cx| {
                    if let Some(pr) = list.get(i) {
                        match crate::presets::load(pr) {
                            Ok(st) => this.apply_preset(&pr.name, st, cx),
                            Err(e) => this.toast(format!("Cannot load: {e}"), true, cx),
                        }
                    }
                },
            )
            .into_any_element(),
        )
    }
}
