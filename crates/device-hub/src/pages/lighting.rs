//! Lighting (per-area effects + per-key colors).

use crate::app::{Shell, Tab};
use crate::pages::keyboard::Cap;
use crate::ui::{self, Btn};
use crate::{overlay, theme};
use devhub_core::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

impl Shell {
    pub fn page_lighting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = theme::pal();
        let Some(s) = self.cur() else {
            return div().into_any_element();
        };
        let (Some(caps), Some(state)) = (s.caps.clone(), s.state.clone()) else {
            return div().into_any_element();
        };
        let mut col = div().flex().flex_col().gap(px(theme::GAP));
        self.light_anim = false;
        if !caps.lighting.is_empty() {
            let ai = caps
                .lighting
                .iter()
                .position(|a| a.id == self.light_area)
                .unwrap_or(0);
            let area = caps.lighting[ai].clone();
            self.light_area = area.id;
            let ls = state
                .lighting
                .get(&area.id)
                .cloned()
                .unwrap_or(LightingState {
                    on: true,
                    effect: area.effects.first().map(|e| e.id).unwrap_or(0),
                    color: Rgb::hex(0xffffff),
                    brightness: area.brightness_max,
                    speed: area.speed_max / 2,
                    direction: 0,
                    per_key: BTreeMap::new(),
                    zone_on: vec![true; area.zones.len()],
                    palette: vec![Rgb::hex(0xffffff)],
                    palette_sel: 0,
                    color_correction: None,
                });
            let eff = area.effects.iter().find(|e| e.id == ls.effect).cloned();
            let aid = area.id;
            let ls_send = ls.clone();
            let send = move |f: Rc<dyn Fn(&mut LightingState)>| {
                let base = ls_send.clone();
                move |this: &mut Shell, cx: &mut Context<Shell>| {
                    let mut n = base.clone();
                    f(&mut n);
                    this.apply(
                        Change::Lighting {
                            area: aid,
                            state: n,
                        },
                        cx,
                    );
                }
            };

            let area_tabs: Option<AnyElement> = (caps.lighting.len() > 1).then(|| {
                let names: Vec<String> = caps.lighting.iter().map(|a| a.name.clone()).collect();
                let ids: Vec<u8> = caps.lighting.iter().map(|a| a.id).collect();
                ui::segmented("light-area", &names, ai, cx, move |this, i, _| {
                    this.light_area = ids[i]
                })
                .w(px((names.len() as f32 * 150.).min(440.)))
                .into_any_element()
            });

            // Effect list.
            let mut effects = div().flex().flex_wrap().gap(px(8.));
            for e in &area.effects {
                let on = e.id == ls.effect;
                let id = e.id;
                // isCustom cells are an overlay that is kept across mode switches, as in the official app.
                // Clearing is done explicitly from the paint side's clear.
                let s2 = send(Rc::new(move |l| l.effect = id));
                effects = effects.child(
                    div()
                        .id(SharedString::from(format!("eff-{}", e.id)))
                        .h(px(34.))
                        .px(px(14.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(px(11.))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .border_1()
                        .when(on, |d| {
                            d.bg(p.accent_soft)
                                .border_color(theme::alpha(p.accent, 0.55))
                                .text_color(p.fg)
                        })
                        .when(!on, |d| {
                            d.bg(p.ctrl)
                                .border_color(p.hair)
                                .text_color(p.sub)
                                .hover(|s| s.bg(p.ctrl_hover))
                        })
                        .when(e.per_key, |d| d.child(ui::icon("brush", 13., p.accent)))
                        .child(e.name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| s2(this, cx))),
                );
            }

            let mut body = ui::body().pt(px(6.));
            if area.switchable {
                // Master toggle: on models with zones, opens/closes all zones together.
                let s2 = send(Rc::new(|l| {
                    l.on = !l.on;
                    l.zone_on.fill(l.on);
                }));
                body = body.child(ui::row_first(
                    "On",
                    None,
                    ui::toggle(format!("light-on-{aid}"), ls.on, cx, move |this, _, cx| {
                        s2(this, cx)
                    }),
                ));
                // DoubleLighting: the same upper/lower light switches as the official app (split into the jo enum and sent).
                for (zi, zname) in area.zones.iter().enumerate() {
                    let cur = ls.zone_on.get(zi).copied().unwrap_or(false);
                    let link = self.light_link;
                    let s2 = send(Rc::new(move |l| {
                        if l.zone_on.len() <= zi {
                            l.zone_on.resize(zi + 1, false);
                        }
                        l.zone_on[zi] = !l.zone_on[zi];
                        // When link is ON, keep both zones the same (same as the official link toggle).
                        if link {
                            let v = l.zone_on[zi];
                            l.zone_on.fill(v);
                        }
                        l.on = l.zone_on.iter().any(|&z| z);
                    }));
                    body = body.child(ui::row(
                        &format!("{zname} switch"),
                        None,
                        ui::toggle(
                            format!("light-zone-{aid}-{zi}"),
                            cur,
                            cx,
                            move |this, _, cx| s2(this, cx),
                        ),
                    ));
                }
                // Lighting link (a UI-local toggle, same as the official link/link-unlink).
                if area.zones.len() > 1 {
                    let link = self.light_link;
                    body = body.child(ui::row(
                        "Lighting link",
                        Some("While ON, the upper/lower switches move together"),
                        ui::toggle(format!("light-link-{aid}"), link, cx, |this, _, _| {
                            this.light_link = !this.light_link;
                        }),
                    ));
                }
            }
            body = body.child(
                div()
                    .py(px(12.))
                    .when(area.switchable, |d| d.border_t_1().border_color(p.hair))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(ui::faint("Effect"))
                    .child(effects),
            );
            if eff.as_ref().is_some_and(|e| e.color) {
                let cur = ls.color;
                let base = ls.clone();
                body = body.child(ui::row(
                    "Color",
                    None,
                    div()
                        .id("light-color")
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .cursor_pointer()
                        .child(ui::faint(format!("#{:06X}", cur.to_hex())))
                        .child(ui::swatch(theme::rgb_of(cur), 28., false))
                        .on_click(cx.listener(move |this, _, w, cx| {
                            let base = base.clone();
                            overlay::pick_color(
                                this,
                                "Lighting color",
                                cur,
                                w,
                                cx,
                                move |this, c, _, cx| {
                                    let mut n = base.clone();
                                    n.color = c;
                                    let sel = n.palette_sel as usize;
                                    if sel < n.palette.len() {
                                        n.palette[sel] = c;
                                    }
                                    this.apply(
                                        Change::Lighting {
                                            area: aid,
                                            state: n,
                                        },
                                        cx,
                                    );
                                },
                            );
                        })),
                ));
            }
            // Palette: the official 8 color dots. Click to select a slot
            // (that slot's color becomes the current color).
            if !ls.palette.is_empty() {
                let mut dots = div().flex().items_center().gap(px(8.));
                for (pi, pc) in ls.palette.iter().enumerate() {
                    let selected = pi == ls.palette_sel as usize;
                    let c = *pc;
                    let s2 = send(Rc::new(move |l| {
                        l.palette_sel = pi as u8;
                        l.color = c;
                    }));
                    // Slot 0 is fixed to the firmware's "multicolor / rainbow" (equivalent to the official more icon).
                    let dot = if pi == 0 {
                        let mut strips = div()
                            .size(px(22.))
                            .rounded(px(11.))
                            .overflow_hidden()
                            .flex()
                            .border_2()
                            .border_color(if selected {
                                p.fg
                            } else {
                                theme::alpha(p.fg, 0.12)
                            });
                        for hc in [0xff5040u32, 0xffd040, 0x40d060, 0x40b0ff, 0xa050ff] {
                            strips = strips
                                .child(div().h_full().flex_1().bg(theme::rgb_of(Rgb::hex(hc))));
                        }
                        strips.into_any_element()
                    } else {
                        ui::swatch(theme::rgb_of(c), 22., selected).into_any_element()
                    };
                    dots = dots.child(
                        div()
                            .id(SharedString::from(format!("pal-{aid}-{pi}")))
                            .cursor_pointer()
                            .child(dot)
                            .on_click(cx.listener(move |this, _, _, cx| s2(this, cx))),
                    );
                }
                body = body.child(ui::row("Palette", None, dots));
            }
            {
                let base = ls.clone();
                let bmax = area.brightness_max as f32;
                let f = Rc::new(move |this: &mut Shell, v: f32, cx: &mut Context<Shell>| {
                    let mut n = base.clone();
                    if n.brightness == v as u8 {
                        return;
                    }
                    n.brightness = v as u8;
                    this.apply(
                        Change::Lighting {
                            area: aid,
                            state: n,
                        },
                        cx,
                    );
                }) as Rc<dyn Fn(&mut Shell, f32, &mut Context<Shell>)>;
                let inp = self.num_field(
                    "light-br",
                    ls.brightness as f32,
                    0.,
                    bmax,
                    1.,
                    "",
                    window,
                    cx,
                    f.clone(),
                );
                body = body.child(ui::row(
                    "Brightness",
                    None,
                    ui::slider_value_edit(
                        "light-br",
                        ls.brightness as f32,
                        0.,
                        bmax,
                        1.,
                        &inp,
                        cx,
                        move |t, v, c| f(t, v, c),
                    ),
                ));
            }
            if eff.as_ref().is_some_and(|e| e.speed) {
                let base = ls.clone();
                let smax = area.speed_max as f32;
                let f = Rc::new(move |this: &mut Shell, v: f32, cx: &mut Context<Shell>| {
                    let mut n = base.clone();
                    if n.speed == v as u8 {
                        return;
                    }
                    n.speed = v as u8;
                    this.apply(
                        Change::Lighting {
                            area: aid,
                            state: n,
                        },
                        cx,
                    );
                }) as Rc<dyn Fn(&mut Shell, f32, &mut Context<Shell>)>;
                let inp = self.num_field(
                    "light-sp",
                    ls.speed as f32,
                    0.,
                    smax,
                    1.,
                    "",
                    window,
                    cx,
                    f.clone(),
                );
                body = body.child(ui::row(
                    "Speed",
                    None,
                    ui::slider_value_edit(
                        "light-sp",
                        ls.speed as f32,
                        0.,
                        smax,
                        1.,
                        &inp,
                        cx,
                        move |t, v, c| f(t, v, c),
                    ),
                ));
            }
            if eff.as_ref().is_some_and(|e| e.direction) && !area.directions.is_empty() {
                let base = ls.clone();
                body = body.child(ui::row(
                    "Direction",
                    None,
                    ui::segmented(
                        "light-dir",
                        &area.directions,
                        ls.direction as usize,
                        cx,
                        move |this, i, cx| {
                            let mut n = base.clone();
                            n.direction = i as u8;
                            this.apply(
                                Change::Lighting {
                                    area: aid,
                                    state: n,
                                },
                                cx,
                            );
                        },
                    )
                    .w(px((area.directions.len() as f32 * 76.).min(380.))),
                ));
            }
            // Saturation (official LightingSaturation): 3 sliders for R/G/B 0-100 →
            // [5,2,area,2,R,G,B] ColorCorrection.
            if let Some(cc) = ls.color_correction {
                body = body.child(
                    div()
                        .pt(px(12.))
                        .border_t_1()
                        .border_color(p.hair)
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(ui::faint("Saturation")),
                );
                for (ch_i, ch_name, ch_v) in [(0usize, "R", cc.r), (1, "G", cc.g), (2, "B", cc.b)] {
                    let base = ls.clone();
                    let f = Rc::new(move |this: &mut Shell, v: f32, cx: &mut Context<Shell>| {
                        let mut n = base.clone();
                        let mut cc2 = n.color_correction.unwrap_or(Rgb::new(100, 100, 100));
                        match ch_i {
                            0 => cc2.r = v as u8,
                            1 => cc2.g = v as u8,
                            _ => cc2.b = v as u8,
                        }
                        n.color_correction = Some(cc2);
                        this.apply(
                            Change::Lighting {
                                area: aid,
                                state: n,
                            },
                            cx,
                        );
                    })
                        as Rc<dyn Fn(&mut Shell, f32, &mut Context<Shell>)>;
                    let inp = self.num_field(
                        &format!("light-cc{ch_i}"),
                        ch_v as f32,
                        0.,
                        100.,
                        1.,
                        "",
                        window,
                        cx,
                        f.clone(),
                    );
                    body = body.child(ui::row(
                        ch_name,
                        None,
                        ui::slider_value_edit(
                            &format!("light-cc{ch_i}"),
                            ch_v as f32,
                            0.,
                            100.,
                            1.,
                            &inp,
                            cx,
                            move |t, v, c| f(t, v, c),
                        ),
                    ));
                }
            }
            col = col.child(ui::card_titled(&area.name, "lightbulb", area_tabs).child(body));

            // Lighting preview — same mechanism as the official HUB:
            // shows the **device's current lit colors** that the worker read back with [5,3] as they are
            // (session live_rgb, updated at ~6Hz). It is not a simulation, so
            // pattern, motion and color always match the real device. Only while it cannot be obtained (unsupported driver /
            // before the first read) does it fall back to a local approximation.
            if let Some(kc) = caps.keyboard.clone() {
                let eff2 = eff.clone();
                let live: Option<std::collections::BTreeMap<usize, Rgb>> = self
                    .device_id()
                    .and_then(|id| self.hub.session(&id).map(|s| s.live_rgb.clone()))
                    .filter(|m| !m.is_empty());
                // If the upper zone (per-key) is OFF there is nothing to draw, so the animation stops too.
                let keys_lit =
                    ls.on && !(area.zones.len() >= 2 && ls.zone_on.first() == Some(&false));
                let animated = live.is_none()
                    && keys_lit
                    && eff2.as_ref().is_some_and(|e| e.speed && !e.per_key);
                self.light_anim = animated;
                let t = self.anim_t;
                let spd = if area.speed_max > 0 {
                    ls.speed as f32 / area.speed_max as f32
                } else {
                    0.5
                };
                let eff_ref = eff.clone();
                let colors: Vec<Option<Rgb>> = match &live {
                    // The real device as is — unlit cells are not in the map, so None.
                    Some(m) => kc
                        .layout
                        .iter()
                        .enumerate()
                        .map(|(i, _)| m.get(&i).copied())
                        .collect(),
                    None => kc
                        .layout
                        .iter()
                        .enumerate()
                        .map(|(i, k)| preview_rgb(&ls, &area, eff_ref.as_ref(), k, i, t, spd))
                        .collect(),
                };
                // If "Preview on device" is ON, stream DirectDrive frames at ~30fps
                // (probe9: the device does not congest even with ~87fps continuous sending).
                // During live lighting, that feature uses DirectDrive, so the preview does not send.
                let fx_here = self.fx_on.is_some() && self.fx_on == self.device_id();
                let dd_on = !fx_here
                    && self
                        .device_id()
                        .and_then(|id| self.hub.session(&id))
                        .is_some_and(|s| s.dd_on);
                // On models where live readback works, the on-device preview is unnecessary — if it was left on,
                // stop it and return to the firmware's lighting (= the same thing as the display above).
                if live.is_some() && dd_on {
                    if let Some(id) = self.device_id() {
                        self.hub.set_dd(&id, false);
                    }
                }
                if live.is_none() && dd_on && self.dd_last.elapsed() >= Duration::from_millis(33) {
                    let cells: BTreeMap<usize, Rgb> = colors
                        .iter()
                        .enumerate()
                        .filter_map(|(i, c)| c.map(|c| (i, c)))
                        .collect();
                    if let Some(id) = self.device_id() {
                        self.hub.dd_frame(&id, cells);
                    }
                    self.dd_last = Instant::now();
                }
                let caps_v: Vec<Cap> = kc
                    .layout
                    .iter()
                    .zip(&colors)
                    .map(|(k, c)| Cap {
                        top: k.label.clone(),
                        tint: c.map(|c| theme::alpha(theme::rgb_of(c), 0.85)),
                        ..Default::default()
                    })
                    .collect();
                let view =
                    self.keyboard_view("lit-preview", &kc, caps_v, cx, Rc::new(|_, _, _, _| {}));
                let mut prev_body = ui::body().pt(px(10.)).pb(px(14.)).gap(px(8.)).child(view);
                // On DirectDrive-capable areas, show the "Preview on device" toggle.
                // The card's content width can exceed the window, so it goes not at the right end of the header but
                // at the start of the body (the always-visible left side).
                // While ON, the colors above are streamed to the device as [5,5] frames at ~30fps;
                // when OFF, DdStop is sent to return to the firmware-side animation.
                if area.direct_drive && fx_here {
                    prev_body = prev_body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(ui::button("fx-goto", "Live lighting running", Some("zap"), Btn::Ghost, cx, |this, _, cx| {
                                this.tab = Tab::Live;
                                cx.notify();
                            }))
                            .child(ui::faint("The device LEDs are driven by live lighting (when it stops, normal lighting returns)")),
                    );
                } else if area.direct_drive && live.is_none() {
                    let dd_label = if dd_on {
                        "Stop device preview"
                    } else {
                        "Preview on device"
                    };
                    let dd_style = if dd_on { Btn::Primary } else { Btn::Ghost };
                    prev_body = prev_body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(ui::button(
                                "dd-preview",
                                dd_label,
                                Some("monitor"),
                                dd_style,
                                cx,
                                move |this, _, cx| {
                                    if let Some(id) = this.device_id() {
                                        let cur = this.hub.session(&id).is_some_and(|s| s.dd_on);
                                        this.hub.set_dd(&id, !cur);
                                    }
                                    cx.notify();
                                },
                            ))
                            .child(ui::faint(
                                "While ON, the image above is streamed directly to the device LEDs",
                            )),
                    );
                }
                prev_body = prev_body.child(ui::faint(if live.is_some() {
                    "Showing the device's current lighting as read back"
                } else {
                    "An image based on the settings (an approximation of the device's actual lighting)"
                }));
                // isCustom cells light regardless of the base mode — if some remain
                // and no custom mode is selected, point to the editing path.
                if !ls.per_key.is_empty() {
                    prev_body = prev_body.child(ui::faint(format!(
                        "{} keys have custom colors set (edit or clear them in \"Per-key colors\" below)",
                        ls.per_key.len()
                    )));
                }
                // On DirectDrive-capable areas, show the "Preview on device" toggle.
                // While ON, the colors above are streamed to the device as [5,5] frames throttled to ~150ms;
                // when OFF, DdStop is sent to return to the firmware-side animation.
                col = col.child(ui::card_titled("Preview", "eye", None).child(prev_body));
            }

            // Per-key colors.
            // Per-key custom colors are an overlay that is always available, not a mode
            // (editable in any mode, like the official custom color).
            if area.per_key {
                if let Some(kc) = caps.keyboard.clone() {
                    let paint = self.paint;
                    let caps_v: Vec<Cap> = kc
                        .layout
                        .iter()
                        .enumerate()
                        .map(|(i, k)| Cap {
                            top: k.label.clone(),
                            tint: ls
                                .per_key
                                .get(&i)
                                .map(|c| theme::alpha(theme::rgb_of(*c), 0.75)),
                            ..Default::default()
                        })
                        .collect();
                    let base = ls.clone();
                    let view = self.keyboard_view(
                        "paint",
                        &kc,
                        caps_v,
                        cx,
                        Rc::new(move |this, i, shift, cx| {
                            let mut n = base.clone();
                            if shift {
                                n.per_key.remove(&i);
                            } else {
                                n.per_key.insert(i, this.paint);
                            }
                            this.apply(
                                Change::Lighting {
                                    area: aid,
                                    state: n,
                                },
                                cx,
                            );
                        }),
                    );
                    let base_all = ls.clone();
                    let base_clear = ls.clone();
                    let nkeys = kc.layout.len();
                    let tools = div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .id("paint-color")
                                .cursor_pointer()
                                .child(ui::swatch(theme::rgb_of(paint), 26., true))
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    overlay::pick_color(
                                        this,
                                        "Paint color",
                                        paint,
                                        w,
                                        cx,
                                        |this, c, _, cx| {
                                            this.paint = c;
                                            cx.notify();
                                        },
                                    );
                                })),
                        )
                        .child(ui::button(
                            "paint-all",
                            "All keys",
                            Some("paint-bucket"),
                            Btn::Ghost,
                            cx,
                            move |this, _, cx| {
                                let mut n = base_all.clone();
                                n.per_key = (0..nkeys).map(|i| (i, this.paint)).collect();
                                this.apply(
                                    Change::Lighting {
                                        area: aid,
                                        state: n,
                                    },
                                    cx,
                                );
                            },
                        ))
                        .child(ui::button(
                            "paint-clear",
                            "Clear",
                            Some("eraser"),
                            Btn::Ghost,
                            cx,
                            move |this, _, cx| {
                                let mut n = base_clear.clone();
                                n.per_key.clear();
                                this.apply(
                                    Change::Lighting {
                                        area: aid,
                                        state: n,
                                    },
                                    cx,
                                );
                            },
                        ));
                    col = col.child(
                        ui::card_titled("Per-key colors", "brush", Some(tools.into_any_element()))
                            .child(
                                ui::body()
                                    .pt(px(10.))
                                    .pb(px(14.))
                                    .gap(px(10.))
                                    .child(view)
                                    .child(ui::faint("Click to paint · Shift+click to erase")),
                            ),
                    );
                }
            }
        }
        let cards = self.section_cards(Tab::Lighting, window, cx);
        for c in cards {
            col = col.child(c);
        }
        col.into_any_element()
    }
}

/// For the preview: from the settings (effect/color/brightness/speed/direction/per_key) and time t,
/// compute an approximate color for key i. Because the LED state cannot be read back from the device, this is a
/// local reproduction of the official HUB's lighting image. The effect id is the effect order the driver lists.
///
/// Whether an effect has a color or a direction follows the `LightEffect` caps flags — effects with a color use
/// the selected color, effects without one cycle through hues. Only effects with a direction reverse their sweep by direction.
fn preview_rgb(
    ls: &LightingState,
    area: &LightArea,
    eff: Option<&LightEffect>,
    k: &LayoutKey,
    i: usize,
    t: f32,
    spd: f32,
) -> Option<Rgb> {
    if !ls.on {
        return None;
    }
    // DoubleLighting: zone_on = [upper=per-key, lower=underglow]. The keyboard preview
    // draws the per-key LEDs (= upper zone), so if the upper zone is OFF the keys go dark just like the real device.
    if area.zones.len() >= 2 && ls.zone_on.first().copied() == Some(false) {
        return None;
    }
    // Pseudo-random value per key (for rainbow pulse / rain, fixed by index).
    let rnd = (((i as u32).wrapping_mul(2654435761) >> 8) % 1000) as f32 / 1000.;
    // Only effects with a direction look at direction (0=Forward→sweep right / 1=Backward→sweep left).
    let dir_sign = if eff.is_some_and(|e| e.direction) && ls.direction == 1 {
        -1.0f32
    } else {
        1.0
    };
    let axis = k.x * dir_sign;
    // Distance from the center (for storm / waveform effects). Coordinates are in keycap units.
    let (dx, dy) = (k.x - 10.5, k.y - 3.);
    let dist = (dx * dx + dy * dy).sqrt();
    let ph = t * (0.25 + spd * 2.0); // phase speed
    let wave = |x: f32| (x * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let v = ls.brightness as f32 / area.brightness_max.max(1) as f32;
    let dim = |c: Rgb, f: f32| {
        Rgb::new(
            (c.r as f32 * f * v) as u8,
            (c.g as f32 * f * v) as u8,
            (c.b as f32 * f * v) as u8,
        )
    };
    let hsv = |h: f32, s: f32, b: f32| -> Rgb {
        let h = h.rem_euclid(1.) * 6.;
        let (p, q, tt) = (
            b * (1. - s),
            b * (1. - s * h.fract()),
            b * (1. - s * (1. - h.fract())),
        );
        let (r, g, bl) = match h as i32 {
            0 => (b, tt, p),
            1 => (q, b, p),
            2 => (p, b, tt),
            3 => (p, q, b),
            4 => (tt, p, b),
            _ => (b, p, q),
        };
        Rgb::new((r * 255.) as u8, (g * 255.) as u8, (bl * 255.) as u8)
    };
    // Structure found through device verification: each mode has a (hue pattern hp, brightness pattern bp),
    // with sel=0 (firmware rainbow slot) it is the rainbow of hp, and when a color slot is selected it uses the selected color's hue
    // so only the light/dark of bp shows (confirmed on camera that with sel=1=red all modes turn reddish).
    // Pseudo press point (for the preview of press-type 17-19): changes position at a fixed interval.
    let seg = (ph * 1.6).floor() as u32;
    let px_i = (((seg.wrapping_mul(2654435761)) >> 8) % 19 + 1) as f32;
    let py_i = (((seg.wrapping_mul(2246822519)) >> 10) % 5) as f32;
    let press_decay = 1. - (ph * 1.6).fract(); // decays after the press
    let (hp, bp): (f32, f32) = match ls.effect {
        // Static color: uniform across the whole board. sel0 is a fixed horizontal rainbow gradient (does not move, the gradient
        // matches the camera capture of the real device B→C→G→Y→M→B→C, about 1.2 turns).
        0 => (0.72 - k.x * 0.068 + k.y * 0.01, 1.),
        // Rainbow side-scroll: the whole rainbow gradient scrolls slowly left/right (real device ~2.6s/cycle).
        1 => (
            axis * 0.05 + ph * 0.3,
            0.75 + 0.25 * wave(axis * 0.18 - ph * 0.7),
        ),
        // Slow color change: per-row hue bands slowly cycle.
        2 => (
            k.y * 0.11 + ph * 0.15,
            0.7 + 0.3 * wave(ph * 0.2 + k.y * 0.3),
        ),
        // Fast color change: a center-symmetric radial hue rotates.
        3 => (ph * 0.3 - dist * 0.05, 0.8 + 0.2 * wave(ph * 1.3)),
        // Rainbow checker: a fine diagonal rainbow checker flows.
        4 => (
            axis * 0.07 + k.y * 0.08 + ph * 0.15,
            0.85 + 0.15 * wave(rnd * 4. + ph * 1.5),
        ),
        // Rainbow cycle: a large smooth horizontal gradient cycles hue.
        5 => (axis * 0.028 + ph * 0.15, 0.8 + 0.2 * wave(ph * 0.8)),
        // Rainbow pulse: gentle per-row hue gradient + per-key twinkle.
        6 => (
            k.y * 0.07 + ph * 0.15,
            0.45 + 0.55 * wave(ph * 2.2 + rnd * 6.),
        ),
        // Faint window: a faint window slowly crosses an almost uniform field
        // (on the real device the hue also changes slowly over a few seconds).
        7 => {
            let pos = (axis * 0.05 - ph * 0.15).rem_euclid(1.);
            (ph * 0.05, if pos < 0.25 { 0.75 } else { 1. })
        }
        // Gentle morph: slow irregular hue morph (patches change like stains).
        8 => (
            ph * 0.05 + (wave(axis * 0.11 + ph * 0.1) - wave(k.y * 0.2 - ph * 0.08)) * 0.1,
            0.55 + 0.45 * wave(dist * 0.3 + ph * 0.15),
        ),
        // Color band shift: bands of a different hue periodically slide into an almost uniform field
        // (on the real device the dominant color slowly swaps every few seconds).
        9 => {
            let zone = ((axis * 0.05 - ph * 0.1).rem_euclid(1.) - 0.5).abs() < 0.14;
            (
                ph * 0.06 + if zone { 0.14 } else { 0. },
                if zone { 1. } else { 0.85 },
            )
        }
        // Rainbow swell: a diagonal multi-band rainbow wave.
        10 => (
            (axis + k.y * 3.) * 0.05 - ph * 0.2,
            0.3 + 0.7 * wave(axis * 0.16 + k.y * 0.14 - ph * 0.5),
        ),
        // Diagonal light band: color blobs flow (dark areas and highly saturated blobs mixed).
        11 => (
            axis * 0.04 + ph * 0.12 + wave(k.y * 0.2 + ph * 0.15) * 0.12,
            0.12 + 0.88 * wave(axis * 0.09 + k.y * 0.11 + ph * 0.25),
        ),
        // Confetti: confetti-like patches blooming in scattered hues per key.
        12 => (
            rnd * 0.9 + ph * 0.25,
            0.15 + 0.85 * wave(ph * 1.3 + rnd * 5.),
        ),
        // Rainbow mosaic: a fine rainbow checker flows diagonally.
        13 => (
            axis * 0.09 + k.y * 0.11 + ph * 0.15,
            0.65 + 0.35 * wave(rnd * 3. + ph * 1.1),
        ),
        // Light rain: per column, color dots fall from top to bottom.
        14 => {
            let col_rnd =
                (((k.x * 37.) as u32).wrapping_mul(2654435761) >> 8) as f32 % 1000. / 1000.;
            let pos = (k.y * 0.14 - ph * (0.9 + col_rnd * 0.9) + col_rnd).rem_euclid(1.);
            (col_rnd * 0.9, if pos < 0.12 { 1. } else { 0.06 })
        }
        // Bouncing light: a lit block of about 5x3 jumps in position and color.
        15 => {
            let jx = (((seg.wrapping_mul(2654435761)) >> 8) % 15) as f32;
            let jy = (((seg.wrapping_mul(40503)) >> 10) % 4) as f32;
            let inb = (k.x - jx).abs() < 2.5 && (k.y - jy).abs() < 1.8;
            (seg as f32 * 0.13, if inb { 1. } else { 0.05 })
        }
        // Custom: only isCustom cells light in the painted color, the rest are off (drawn by the overlay at the end).
        16 => (0., 0.),
        // Press: surround: the block around the pressed key lights (reproduced with a pseudo press).
        17 => {
            let lit = (k.x - px_i).abs() <= 1.5 && (k.y - py_i).abs() <= 1.5;
            (k.y * 0.1 + 0.15, if lit { press_decay } else { 0.04 })
        }
        // Press: row: the row of the pressed key lights.
        18 => {
            let lit = (k.y - py_i).abs() <= 0.6;
            (k.y * 0.1 + 0.15, if lit { press_decay } else { 0.04 })
        }
        // Press: wide: a wide area around the pressed key lights.
        19 => {
            let lit = ((k.x - px_i).powi(2) + (k.y - py_i).powi(2)).sqrt() <= 4.2;
            (k.y * 0.1 + 0.15, if lit { press_decay } else { 0.04 })
        }
        _ => (0.5, 0.6),
    };
    let base = if ls.palette_sel == 0 {
        hsv(hp, 0.9, v * bp)
    } else {
        dim(ls.color, bp)
    };
    // isCustom cells light in the custom color regardless of the base mode (the same overlay as the official one).
    Some(
        ls.per_key
            .get(&i)
            .copied()
            .map(|c| dim(c, 1.))
            .unwrap_or(base),
    )
}
