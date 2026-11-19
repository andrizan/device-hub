//! Magnetic switch calibration wizard.
//!
//! Start → run the driver's `calibrate`, turn the stroke test (`live_travel`) ON →
//! record the maximum press depth of each key and turn keys pressed near the bottom (85% of the full stroke) green →
//! on finish, run `calibrate_finish` (if it is in caps). The web version cannot show whether a key was pressed,
//! but here keys that were missed are visible at a glance.

use crate::app::{Modal, Shell};
use crate::pages::keyboard::{kb_parts, Cap};
use crate::theme;
use crate::ui::{self, Btn};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

const DONE_RATIO: f32 = 0.85;

impl Shell {
    pub fn modal_calibrate(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = theme::pal();
        let Some((kc, _)) = kb_parts(self) else {
            return div().into_any_element();
        };
        let live = self.cur().map(|s| s.travel.clone()).unwrap_or_default();
        // Completion is judged by a ratio of the observed maximum depth (the nominal full stroke is deeper than the real bottom).
        let gmax = self.cur().map(|s| s.travel_max).unwrap_or(0.).max(1.5);
        let done_mm = (gmax * DONE_RATIO).max(2.0);
        let n = kc.layout.len();
        let started = matches!(self.modal, Some(Modal::Calibrate { started: true, .. }));
        // Update the maximum depth.
        if let Some(Modal::Calibrate {
            maxes,
            started: true,
        }) = self.modal.as_mut()
        {
            if maxes.len() != n {
                *maxes = vec![0.; n];
            }
            for (m, d) in maxes.iter_mut().zip(live.iter()) {
                *m = m.max(*d);
            }
        }
        let maxes = match &self.modal {
            Some(Modal::Calibrate { maxes, .. }) => maxes.clone(),
            _ => vec![],
        };
        let done = maxes.iter().filter(|m| **m >= done_mm).count();
        let caps: Vec<Cap> = kc
            .layout
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let m = maxes.get(i).copied().unwrap_or(0.);
                let d = live.get(i).copied().unwrap_or(0.);
                let ok = m >= done_mm;
                Cap {
                    top: k.label.clone(),
                    sub: (started && m > 0.05).then(|| format!("{m:.2}")),
                    tint: Some(if ok {
                        theme::alpha(p.ok, 0.45)
                    } else if m > 0.2 {
                        theme::alpha(p.warn, 0.35)
                    } else {
                        p.key
                    }),
                    depth: started.then_some(d / gmax),
                    ..Default::default()
                }
            })
            .collect();
        let view = self.keyboard_view("calib", &kc, caps, cx, Rc::new(|_, _, _, _| {}));
        let has_finish = self
            .cur()
            .and_then(|s| s.caps.as_ref())
            .is_some_and(|c| c.actions.iter().any(|a| a.id == "calibrate_finish"));

        let footer = if !started {
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(ui::button(
                    "cal-cancel",
                    "Close",
                    None,
                    Btn::Ghost,
                    cx,
                    |this, _, cx| {
                        this.modal = None;
                        cx.notify();
                    },
                ))
                .child(ui::button(
                    "cal-start",
                    "Start",
                    Some("play"),
                    Btn::Primary,
                    cx,
                    |this, _, cx| {
                        if let Some(id) = this.device_id() {
                            this.hub
                                .run_action(&id, "calibrate", "Start calibration", None);
                            this.hub.set_travel(&id, true);
                        }
                        this.modal = Some(Modal::Calibrate {
                            started: true,
                            maxes: vec![],
                        });
                        cx.notify();
                    },
                ))
        } else {
            let all = done == n && n > 0;
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(ui::faint(format!("{done} / {n} keys done")))
                .child(div().flex_1())
                .child(ui::button(
                    "cal-abort",
                    "Cancel",
                    None,
                    Btn::Ghost,
                    cx,
                    |this, _, cx| {
                        if let Some(id) = this.device_id() {
                            this.hub.set_travel(&id, false);
                        }
                        this.modal = None;
                        cx.notify();
                    },
                ))
                .child(ui::button(
                    "cal-finish",
                    if all { "Finish and save" } else { "Save as is" },
                    Some("check"),
                    if all { Btn::Primary } else { Btn::Secondary },
                    cx,
                    move |this, _, cx| {
                        if let Some(id) = this.device_id() {
                            this.hub.set_travel(&id, false);
                            if has_finish {
                                this.hub.run_action(
                                    &id,
                                    "calibrate_finish",
                                    "Save calibration",
                                    None,
                                );
                            }
                        }
                        this.modal = None;
                        this.toast(format!("Calibration: {done}/{n} keys"), done < n, cx);
                    },
                ))
        };
        let frac = if n > 0 { done as f32 / n as f32 } else { 0. };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(ui::sub(if started {
                "Press every key slowly down to the bottom once, then release. Keys that reach the bottom turn green."
            } else {
                "Re-measures the reference for key press amount. Once started, press every key down to the bottom once. Do not unplug the keyboard midway."
            }))
            .when(started, |d| {
                d.child(
                    div()
                        .h(px(6.))
                        .rounded(px(3.))
                        .bg(p.track)
                        .child(div().h_full().w(relative(frac)).rounded(px(3.)).bg(p.ok)),
                )
            })
            .child(view)
            .child(footer)
            .into_any_element()
    }
}
