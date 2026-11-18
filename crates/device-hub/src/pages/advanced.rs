//! Advanced keys (DKS / MPT / MT / TGL / END / SOCD / RS).

use crate::app::{PopItem, Shell};
use crate::pages::keyboard::{action_label, kb_parts, Cap};
use crate::theme;
use crate::ui::{self, Btn};
use devhub_core::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum PickTarget {
    #[default]
    Key,
    Other,
}

/// The advanced key being edited.
#[derive(Clone, Default)]
pub struct AdvDraft {
    pub kind: Option<AdvancedKind>,
    pub key: Option<usize>,
    pub layer: u8,
    pub binding: Option<Advanced>,
    pub pick: PickTarget,
}

pub fn kind_meta(k: AdvancedKind) -> (&'static str, &'static str, &'static str) {
    match k {
        AdvancedKind::Dks => ("DKS", "Dynamic Keystroke — output up to 4 keys across 4 stages: press start, bottom out, release start, full release", "layers"),
        AdvancedKind::Mpt => ("MPT", "Multi-point trigger — output a different key at each of 3 press depths", "chart-column-increasing"),
        AdvancedKind::Mt => ("MT", "Multi-tap — one key on a short press, another on a long press", "timer"),
        AdvancedKind::Tgl => ("TGL", "Toggle — press once to hold the key down, press again to release", "toggle-right"),
        AdvancedKind::End => ("END", "Release trigger — output the key at a set position on the way back up (osu! etc.)", "corner-left-up"),
        AdvancedKind::Socd => ("SOCD", "Resolve simultaneous presses of opposing keys (last input priority, etc.)", "arrow-left-right"),
        AdvancedKind::Rs => ("RS", "Rapid Snap — output only the deeper-pressed of two keys (counter-strafing)", "zap"),
    }
}

fn default_binding(k: AdvancedKind, kc: &KeyboardCaps, key: usize, other: usize) -> Advanced {
    let code_of = |i: usize| match kc.layout.get(i).map(|l| l.label.as_str()) {
        Some(l) => kc
            .catalog
            .first()
            .and_then(|g| {
                g.keys
                    .iter()
                    .find(|c| c.short.as_deref() == Some(l) || c.label == l)
            })
            .map(|c| c.code)
            .unwrap_or(0x04),
        None => 0x04,
    };
    let c = code_of(key);
    match k {
        AdvancedKind::Dks => Advanced::Dks {
            rows: vec![DksRow {
                code: c,
                phases: [DksPhase::Hold, DksPhase::Off, DksPhase::Off, DksPhase::Off],
            }],
            bottom: 3.0,
        },
        AdvancedKind::Mpt => Advanced::Mpt {
            depths: [0.8, 2.0, 3.2],
            codes: [c, c, c],
        },
        AdvancedKind::Mt => Advanced::Mt {
            tap: c,
            hold: 0xE1,
            hold_ms: 200,
        },
        AdvancedKind::Tgl => Advanced::Tgl {
            code: c,
            delay_ms: 20,
        },
        AdvancedKind::End => Advanced::End {
            code: c,
            code_release: c,
            delay_ms: 20,
        },
        AdvancedKind::Socd => Advanced::Socd {
            other,
            code_a: c,
            code_b: code_of(other),
            mode: SocdMode::Last,
            delay_ms: 0,
        },
        AdvancedKind::Rs => Advanced::Rs {
            other,
            code_a: c,
            code_b: code_of(other),
            delay_ms: 0,
        },
    }
}

fn partner(b: &Advanced) -> Option<usize> {
    match b {
        Advanced::Socd { other, .. } | Advanced::Rs { other, .. } => Some(*other),
        _ => None,
    }
}

impl Shell {
    pub fn page_advanced(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let Some((kc, ks)) = kb_parts(self) else {
            return div().into_any_element();
        };

        // ---- List
        let mut list = ui::body().pt(px(4.));
        if ks.advanced.is_empty() {
            list = list.child(ui::faint("Nothing configured yet").py(px(10.)));
        }
        for (i, a) in ks.advanced.iter().enumerate() {
            let (name, _, icon) = kind_meta(a.binding.kind());
            let label = kc
                .layout
                .get(a.key)
                .map(|k| k.label.clone())
                .unwrap_or_default();
            let other = partner(&a.binding)
                .and_then(|o| kc.layout.get(o))
                .map(|k| format!(" + {}", k.label))
                .unwrap_or_default();
            let layer = kc.layers.get(a.layer as usize).cloned().unwrap_or_default();
            let a_edit = a.clone();
            let (key, lay) = (a.key, a.layer);
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .py(px(9.))
                    .when(i > 0, |d| d.border_t_1().border_color(p.hair))
                    .child(
                        div()
                            .size(px(32.))
                            .rounded(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(p.accent_soft)
                            .child(ui::icon(icon, 15., p.accent)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{label}{other}")),
                            )
                            .child(ui::faint(format!(
                                "{name} · {layer} · {}",
                                summary(&kc, &a.binding)
                            ))),
                    )
                    .child(ui::icon_button(
                        format!("adv-edit-{i}"),
                        "pencil",
                        cx,
                        move |this, _, cx| {
                            this.adv = AdvDraft {
                                kind: Some(a_edit.binding.kind()),
                                key: Some(a_edit.key),
                                layer: a_edit.layer,
                                binding: Some(a_edit.binding.clone()),
                                pick: PickTarget::Key,
                            };
                            cx.notify();
                        },
                    ))
                    .child(ui::icon_button(
                        format!("adv-del-{i}"),
                        "trash",
                        cx,
                        move |this, _, cx| {
                            this.apply(
                                Change::Advanced {
                                    key,
                                    layer: lay,
                                    binding: None,
                                },
                                cx,
                            );
                        },
                    )),
            );
        }
        let list_card = ui::card_titled("Configured advanced keys", "sparkles", None).child(list);

        // ---- Choose a type
        let mut kinds = div().flex().flex_wrap().gap(px(10.));
        for k in kc.advanced.iter().copied() {
            let (name, desc, icon) = kind_meta(k);
            let on = self.adv.kind == Some(k);
            kinds = kinds.child(
                div()
                    .id(SharedString::from(format!("advk-{k:?}")))
                    .w(px(228.))
                    .p(px(12.))
                    .flex()
                    .gap(px(10.))
                    .rounded(px(16.))
                    .cursor_pointer()
                    .border_1()
                    .when(on, |d| {
                        d.bg(p.accent_soft)
                            .border_color(theme::alpha(p.accent, 0.55))
                    })
                    .when(!on, |d| {
                        d.bg(p.ctrl)
                            .border_color(p.hair)
                            .hover(|s| s.bg(p.ctrl_hover))
                    })
                    .child(ui::icon(icon, 18., if on { p.accent } else { p.sub }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(15.))
                                    .text_color(p.sub)
                                    .child(desc),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let layer = this.kb_layer;
                        this.adv = AdvDraft {
                            kind: Some(k),
                            key: None,
                            layer,
                            binding: None,
                            pick: PickTarget::Key,
                        };
                        cx.notify();
                    })),
            );
        }
        let new_card = ui::card_titled("Add new", "plus", None)
            .child(ui::body().pt(px(8.)).pb(px(16.)).child(kinds));

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP))
            .child(list_card)
            .child(new_card);
        if self.adv.kind.is_some() {
            col = col.child(self.adv_editor(&kc, &ks, cx));
        }
        col.into_any_element()
    }

    fn adv_editor(&mut self, kc: &KeyboardCaps, ks: &KeyboardState, cx: &mut Context<Self>) -> Div {
        let p = theme::pal();
        let d = self.adv.clone();
        let kind = d.kind.expect("kind");
        let (name, desc, icon) = kind_meta(kind);
        let needs_other = matches!(kind, AdvancedKind::Socd | AdvancedKind::Rs);
        let other = d.binding.as_ref().and_then(partner);

        // Keyboard diagram (pick the key and its partner).
        let caps: Vec<Cap> = kc
            .layout
            .iter()
            .enumerate()
            .map(|(i, k)| Cap {
                top: k.label.clone(),
                selected: d.key == Some(i),
                link: other == Some(i),
                badge: ks
                    .advanced
                    .iter()
                    .any(|a| a.key == i && a.layer == d.layer)
                    .then(|| "★".into()),
                ..Default::default()
            })
            .collect();
        let kc2 = kc.clone();
        let view = self.keyboard_view(
            "adv",
            kc,
            caps,
            cx,
            Rc::new(move |this, i, _, _| {
                let kind = this.adv.kind.expect("kind");
                match this.adv.pick {
                    PickTarget::Key => {
                        this.adv.key = Some(i);
                        let other = this.adv.binding.as_ref().and_then(partner).unwrap_or(i);
                        this.adv.binding = Some(default_binding(kind, &kc2, i, other));
                        if matches!(kind, AdvancedKind::Socd | AdvancedKind::Rs) {
                            this.adv.pick = PickTarget::Other;
                        }
                    }
                    PickTarget::Other => {
                        if let Some(key) = this.adv.key {
                            if key != i {
                                this.adv.binding = Some(default_binding(kind, &kc2, key, i));
                                this.adv.pick = PickTarget::Key;
                            }
                        }
                    }
                }
            }),
        );
        let key_name = |i: usize| {
            kc.layout
                .get(i)
                .map(|l| l.label.clone())
                .unwrap_or_else(|| "?".into())
        };
        let hint = match (d.key, d.pick, needs_other) {
            (None, _, _) => "Click the key to configure".to_string(),
            (Some(k), PickTarget::Other, true) => {
                format!("Click the partner key to pair with \"{}\"", key_name(k))
            }
            (Some(k), _, _) => format!(
                "Pair: {} ⇄ {}",
                key_name(k),
                other.map(key_name).unwrap_or_else(|| "not selected".into())
            ),
        };

        let params: AnyElement = match d.binding.clone() {
            None => div().into_any_element(),
            Some(b) => self.adv_params(kc, b, cx).into_any_element(),
        };
        let can_save = d.key.is_some()
            && d.binding.is_some()
            && (!needs_other || other.is_some_and(|o| Some(o) != d.key));
        let footer = div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .child(ui::button(
                "adv-cancel",
                "Close",
                None,
                Btn::Ghost,
                cx,
                |this, _, cx| {
                    this.adv = AdvDraft::default();
                    cx.notify();
                },
            ))
            .when(can_save, |f| {
                f.child(ui::button(
                    "adv-save",
                    "Save",
                    Some("check"),
                    Btn::Primary,
                    cx,
                    |this, _, cx| {
                        let d = this.adv.clone();
                        if let (Some(key), Some(b)) = (d.key, d.binding) {
                            // MPT depths must be strictly increasing.
                            if let Advanced::Mpt { depths, .. } = &b {
                                if !(depths[0] < depths[1] && depths[1] < depths[2]) {
                                    this.toast(
                                        "The 3 depths must increase from shallow to deep",
                                        true,
                                        cx,
                                    );
                                    return;
                                }
                            }
                            this.apply(
                                Change::Advanced {
                                    key,
                                    layer: d.layer,
                                    binding: Some(b),
                                },
                                cx,
                            );
                            this.adv = AdvDraft::default();
                            this.toast("Advanced key saved", false, cx);
                        }
                    },
                ))
            });
        ui::card_titled(
            &format!("Configure {name}"),
            icon,
            Some(ui::chip(hint, p.accent).into_any_element()),
        )
        .child(
            ui::body()
                .gap(px(12.))
                .pt(px(4.))
                .pb(px(16.))
                .child(ui::faint(desc))
                .child(view)
                .child(params)
                .child(footer),
        )
    }

    fn code_select(
        &mut self,
        id: &str,
        kc: &KeyboardCaps,
        code: u32,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Shell, u32) + 'static,
    ) -> Stateful<Div> {
        let all: Vec<(u32, String)> = kc
            .catalog
            .iter()
            .flat_map(|g| {
                g.keys
                    .iter()
                    .map(move |k| (k.code, format!("{} — {}", k.label, g.name)))
            })
            .collect();
        let items: Vec<PopItem> = all
            .iter()
            .map(|(c, l)| PopItem::new(l.clone(), *c == code))
            .collect();
        let cur = action_label(kc, &KeyAction::Code(code)).1;
        let f = Rc::new(f);
        ui::select(id, items, &cur, 190., cx, move |this, i, _, cx| {
            f(this, all[i].0);
            cx.notify();
        })
    }

    fn adv_params(&mut self, kc: &KeyboardCaps, b: Advanced, cx: &mut Context<Self>) -> Div {
        let p = theme::pal();
        let travel = kc.perf.as_ref().map(|pc| pc.travel).unwrap_or(4.0);
        let step = kc.perf.as_ref().map(|pc| pc.step).unwrap_or(0.01).max(0.01);
        let upd = |f: Rc<dyn Fn(&mut Advanced)>| {
            move |this: &mut Shell| {
                if let Some(b) = this.adv.binding.as_mut() {
                    f(b);
                }
            }
        };
        let mut body = div().flex().flex_col();
        match b {
            Advanced::Dks { rows, bottom } => {
                let phases = ["Press start", "Bottom out", "Release start", "Full release"];
                let mut head = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pb(px(6.))
                    .child(div().w(px(200.)).child(ui::faint("Key")));
                for ph in phases {
                    head = head.child(
                        div()
                            .w(px(84.))
                            .flex()
                            .justify_center()
                            .child(ui::faint(ph)),
                    );
                }
                body = body.child(head);
                for (ri, r) in rows.iter().enumerate() {
                    let mut line = div().flex().items_center().gap(px(8.)).py(px(4.));
                    line = line.child(
                        self.code_select(
                            &format!("dks-code-{ri}"),
                            kc,
                            r.code,
                            cx,
                            upd_rc(
                                move |b| {
                                    if let Advanced::Dks { rows, .. } = b {
                                        if let Some(r) = rows.get_mut(ri) {
                                            r.code = 0;
                                        }
                                    }
                                },
                                ri,
                            ),
                        )
                        .w(px(200.)),
                    );
                    for (pi, ph) in r.phases.iter().enumerate() {
                        let (label, c) = match ph {
                            DksPhase::Off => ("—", p.faint),
                            DksPhase::Tap => ("Tap", p.accent),
                            DksPhase::Hold => ("Hold", p.ok),
                        };
                        line = line.child(
                            div()
                                .id(SharedString::from(format!("dks-{ri}-{pi}")))
                                .w(px(84.))
                                .h(px(32.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(10.))
                                .cursor_pointer()
                                .bg(theme::alpha(c, 0.12))
                                .border_1()
                                .border_color(theme::alpha(c, 0.4))
                                .text_size(px(12.))
                                .text_color(c)
                                .child(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(Advanced::Dks { rows, .. }) =
                                        this.adv.binding.as_mut()
                                    {
                                        if let Some(r) = rows.get_mut(ri) {
                                            r.phases[pi] = match r.phases[pi] {
                                                DksPhase::Off => DksPhase::Tap,
                                                DksPhase::Tap => DksPhase::Hold,
                                                DksPhase::Hold => DksPhase::Off,
                                            };
                                        }
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    line = line.child(ui::icon_button(
                        format!("dks-del-{ri}"),
                        "x",
                        cx,
                        move |this, _, cx| {
                            if let Some(Advanced::Dks { rows, .. }) = this.adv.binding.as_mut() {
                                if rows.len() > 1 {
                                    rows.remove(ri);
                                }
                            }
                            cx.notify();
                        },
                    ));
                    body = body.child(line);
                }
                if rows.len() < 4 {
                    body = body.child(div().pt(px(4.)).child(ui::button(
                        "dks-add",
                        "Add key",
                        Some("plus"),
                        Btn::Ghost,
                        cx,
                        |this, _, cx| {
                            if let Some(Advanced::Dks { rows, .. }) = this.adv.binding.as_mut() {
                                rows.push(DksRow {
                                    code: 0x04,
                                    phases: [DksPhase::Off; 4],
                                });
                            }
                            cx.notify();
                        },
                    )));
                }
                body = body.child(ui::row(
                    "Bottom-out depth",
                    None,
                    ui::slider_value(
                        "dks-bottom",
                        bottom,
                        0.5,
                        travel,
                        step,
                        "mm",
                        cx,
                        move |this, v, cx| {
                            upd(Rc::new(move |b| {
                                if let Advanced::Dks { bottom, .. } = b {
                                    *bottom = v;
                                }
                            }))(this);
                            cx.notify();
                        },
                    ),
                ));
            }
            Advanced::Mpt { depths, codes } => {
                for i in 0..3 {
                    body = body.child(
                        ui::row(
                            &format!("Stage {}", i + 1),
                            None,
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(self.code_select(
                                    &format!("mpt-code-{i}"),
                                    kc,
                                    codes[i],
                                    cx,
                                    move |this, c| {
                                        if let Some(Advanced::Mpt { codes, .. }) =
                                            this.adv.binding.as_mut()
                                        {
                                            codes[i] = c;
                                        }
                                    },
                                ))
                                .child(ui::slider_value(
                                    &format!("mpt-d-{i}"),
                                    depths[i],
                                    0.1,
                                    travel,
                                    step,
                                    "mm",
                                    cx,
                                    move |this, v, cx| {
                                        if let Some(Advanced::Mpt { depths, .. }) =
                                            this.adv.binding.as_mut()
                                        {
                                            depths[i] = v;
                                        }
                                        cx.notify();
                                    },
                                )),
                        )
                        .when(i == 0, |d| d.border_t_0()),
                    );
                }
            }
            Advanced::Mt { tap, hold, hold_ms } => {
                body = body
                    .child(ui::row_first(
                        "Key on tap",
                        None,
                        self.code_select("mt-tap", kc, tap, cx, |this, c| {
                            if let Some(Advanced::Mt { tap, .. }) = this.adv.binding.as_mut() {
                                *tap = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        "Key on hold",
                        None,
                        self.code_select("mt-hold", kc, hold, cx, |this, c| {
                            if let Some(Advanced::Mt { hold, .. }) = this.adv.binding.as_mut() {
                                *hold = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        "Hold detection time",
                        None,
                        ui::slider_value(
                            "mt-ms",
                            hold_ms as f32,
                            50.,
                            1000.,
                            10.,
                            "ms",
                            cx,
                            |this, v, cx| {
                                if let Some(Advanced::Mt { hold_ms, .. }) =
                                    this.adv.binding.as_mut()
                                {
                                    *hold_ms = v as u16;
                                }
                                cx.notify();
                            },
                        ),
                    ));
            }
            Advanced::Tgl { code, delay_ms } => {
                body = body
                    .child(ui::row_first(
                        "Key to toggle",
                        None,
                        self.code_select("tgl-code", kc, code, cx, |this, c| {
                            if let Some(Advanced::Tgl { code, .. }) = this.adv.binding.as_mut() {
                                *code = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        "Press duration",
                        Some("Press duration of the toggle output"),
                        ui::slider_value(
                            "tgl-ms",
                            delay_ms as f32,
                            0.,
                            500.,
                            1.,
                            "ms",
                            cx,
                            |this, v, cx| {
                                if let Some(Advanced::Tgl { delay_ms, .. }) =
                                    this.adv.binding.as_mut()
                                {
                                    *delay_ms = v as u16;
                                }
                                cx.notify();
                            },
                        ),
                    ));
            }
            Advanced::End {
                code,
                code_release,
                delay_ms,
            } => {
                body = body
                    .child(ui::row_first(
                        "Key on press",
                        None,
                        self.code_select("end-code", kc, code, cx, |this, c| {
                            if let Some(Advanced::End { code, .. }) = this.adv.binding.as_mut() {
                                *code = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        "Key on release",
                        None,
                        self.code_select("end-code2", kc, code_release, cx, |this, c| {
                            if let Some(Advanced::End { code_release, .. }) =
                                this.adv.binding.as_mut()
                            {
                                *code_release = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        "Press duration",
                        None,
                        ui::slider_value(
                            "end-ms",
                            delay_ms as f32,
                            0.,
                            500.,
                            1.,
                            "ms",
                            cx,
                            |this, v, cx| {
                                if let Some(Advanced::End { delay_ms, .. }) =
                                    this.adv.binding.as_mut()
                                {
                                    *delay_ms = v as u16;
                                }
                                cx.notify();
                            },
                        ),
                    ));
            }
            Advanced::Socd {
                code_a,
                code_b,
                mode,
                delay_ms,
                ..
            } => {
                // Show the names of the physical keys picked on the board, not "A key / B key".
                let name = |i: Option<usize>| {
                    i.and_then(|i| kc.layout.get(i))
                        .map(|k| k.label.clone())
                        .unwrap_or_else(|| "?".into())
                };
                let la = name(self.adv.key);
                let lb = name(partner(&b));
                let modes = [
                    "Last input".to_string(),
                    "First input".to_string(),
                    format!("{la} priority"),
                    format!("{lb} priority"),
                    "Neutral".to_string(),
                ];
                let cur = match mode {
                    SocdMode::Last => 0,
                    SocdMode::First => 1,
                    SocdMode::PriorityA => 2,
                    SocdMode::PriorityB => 3,
                    SocdMode::Neutral => 4,
                };
                body = body
                    .child(ui::row_first(
                        &format!("Key sent by \"{la}\""),
                        Some(&format!("Key output when {la} is pressed")),
                        self.code_select("socd-a", kc, code_a, cx, |this, c| {
                            if let Some(Advanced::Socd { code_a, .. }) = this.adv.binding.as_mut() {
                                *code_a = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        &format!("Key sent by \"{lb}\""),
                        Some(&format!("Key output when {lb} is pressed")),
                        self.code_select("socd-b", kc, code_b, cx, |this, c| {
                            if let Some(Advanced::Socd { code_b, .. }) = this.adv.binding.as_mut() {
                                *code_b = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        "On simultaneous press",
                        None,
                        ui::segmented(
                            "socd-mode",
                            &modes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                            cur,
                            cx,
                            |this, i, _| {
                                if let Some(Advanced::Socd { mode, .. }) = this.adv.binding.as_mut()
                                {
                                    *mode = [
                                        SocdMode::Last,
                                        SocdMode::First,
                                        SocdMode::PriorityA,
                                        SocdMode::PriorityB,
                                        SocdMode::Neutral,
                                    ][i];
                                }
                            },
                        )
                        .w(px(460.)),
                    ))
                    .child(ui::row(
                        "Reversal delay",
                        Some("Time until the simultaneous-press decision switches"),
                        ui::slider_value(
                            "socd-ms",
                            delay_ms as f32,
                            0.,
                            500.,
                            1.,
                            "ms",
                            cx,
                            |this, v, cx| {
                                if let Some(Advanced::Socd { delay_ms, .. }) =
                                    this.adv.binding.as_mut()
                                {
                                    *delay_ms = v as u16;
                                }
                                cx.notify();
                            },
                        ),
                    ));
            }
            Advanced::Rs {
                code_a,
                code_b,
                delay_ms,
                ..
            } => {
                let name = |i: Option<usize>| {
                    i.and_then(|i| kc.layout.get(i))
                        .map(|k| k.label.clone())
                        .unwrap_or_else(|| "?".into())
                };
                let la = name(self.adv.key);
                let lb = name(partner(&b));
                body = body
                    .child(ui::row_first(
                        &format!("Key sent by \"{la}\""),
                        Some(&format!("Key output when {la} is pressed")),
                        self.code_select("rs-a", kc, code_a, cx, |this, c| {
                            if let Some(Advanced::Rs { code_a, .. }) = this.adv.binding.as_mut() {
                                *code_a = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        &format!("Key sent by \"{lb}\""),
                        Some(&format!("Key output when {lb} is pressed")),
                        self.code_select("rs-b", kc, code_b, cx, |this, c| {
                            if let Some(Advanced::Rs { code_b, .. }) = this.adv.binding.as_mut() {
                                *code_b = c;
                            }
                        }),
                    ))
                    .child(ui::row(
                        "Decision delay",
                        Some("Time until the switch to the deeper key is confirmed"),
                        ui::slider_value(
                            "rs-ms",
                            delay_ms as f32,
                            0.,
                            500.,
                            1.,
                            "ms",
                            cx,
                            |this, v, cx| {
                                if let Some(Advanced::Rs { delay_ms, .. }) =
                                    this.adv.binding.as_mut()
                                {
                                    *delay_ms = v as u16;
                                }
                                cx.notify();
                            },
                        ),
                    ));
            }
        }
        body
    }
}

/// For picking the key of a DKS row (closure that rewrites code).
fn upd_rc(
    _unused: impl Fn(&mut Advanced) + 'static,
    ri: usize,
) -> impl Fn(&mut Shell, u32) + 'static {
    move |this: &mut Shell, c: u32| {
        if let Some(Advanced::Dks { rows, .. }) = this.adv.binding.as_mut() {
            if let Some(r) = rows.get_mut(ri) {
                r.code = c;
            }
        }
    }
}

/// One-line summary shown in the list.
fn summary(kc: &KeyboardCaps, b: &Advanced) -> String {
    let n = |c: u32| action_label(kc, &KeyAction::Code(c)).0;
    match b {
        Advanced::Dks { rows, .. } => rows
            .iter()
            .map(|r| n(r.code))
            .collect::<Vec<_>>()
            .join(" / "),
        Advanced::Mpt { codes, depths } => format!(
            "{}@{:.1} {}@{:.1} {}@{:.1}",
            n(codes[0]),
            depths[0],
            n(codes[1]),
            depths[1],
            n(codes[2]),
            depths[2]
        ),
        Advanced::Mt { tap, hold, hold_ms } => {
            format!("Tap {} / Hold {} ({hold_ms}ms)", n(*tap), n(*hold))
        }
        Advanced::Tgl { code, .. } => n(*code),
        Advanced::End {
            code, code_release, ..
        } => format!("{} ⇅ {}", n(*code), n(*code_release)),
        Advanced::Socd { mode, .. } => format!("{mode:?}"),
        Advanced::Rs { code_a, code_b, .. } => format!("{} ↔ {}", n(*code_a), n(*code_b)),
    }
}
