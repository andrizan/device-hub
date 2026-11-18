//! Editor page for the OBS overlay.
//!
//! Draws the preview with the same renderer as the OBS plugin (`devhub-obs`), and saves
//! the appearance (theme, colors, scale...) and **per-key/button visibility** to `obs_overlay.json`.
//! The plugin rechecks the file every second, so changes take effect immediately even with OBS open.

use crate::app::Shell;
use crate::pages::keyboard::Cap;
use crate::ui::{self, Btn};
use crate::{overlay, theme};
use devhub_core::model::KeyboardCaps;
use devhub_core::*;
use devhub_obs::config::{Arrange, FillMode, MouseStyle, OverlayCfg, Theme as ObsTheme};
use devhub_obs::render::{self, Mode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;
use std::sync::Arc;

const ACCENT_CHOICES: [u32; 8] = [
    0x3cbeff, 0x34d6a8, 0xa98bff, 0xff8a5c, 0xff5c8a, 0xf5c24c, 0xffffff, 0x78e678,
];

/// The keyboard to edit (caps of the connected one, otherwise the SU66 Max default layout) and the mouse button names.
fn targets(this: &Shell) -> (String, KeyboardCaps, f32, String, Vec<String>) {
    let mut kb: Option<(String, KeyboardCaps)> = None;
    let mut mouse: Option<(String, Vec<String>)> = None;
    for d in &this.hub.devices {
        let Some(s) = this.hub.session(&d.id) else {
            continue;
        };
        let Some(c) = &s.caps else { continue };
        if kb.is_none() {
            if let Some(k) = &c.keyboard {
                if k.perf.as_ref().is_some_and(|p| p.live_travel) {
                    kb = Some((d.name.clone(), k.clone()));
                }
            }
        }
        if mouse.is_none() {
            if let Some(m) = &c.mouse {
                if !m.live_buttons.is_empty() {
                    let names = m
                        .live_buttons
                        .iter()
                        .map(|&i| {
                            m.buttons
                                .get(i)
                                .map(|b| b.name.clone())
                                .unwrap_or_else(|| format!("B{i}"))
                        })
                        .collect();
                    mouse = Some((d.name.clone(), names));
                }
            }
        }
    }
    let (kb_name, kc) = kb.unwrap_or_else(|| {
        let mut kc = devhub_core::demo::keyboard_caps()
            .keyboard
            .expect("demo kb");
        kc.layout = devhub_core::registry::su66max_layout();
        ("SU66 Max".into(), kc)
    });
    let travel = kc.perf.as_ref().map(|p| p.travel).unwrap_or(4.);
    let (m_name, names) =
        mouse.unwrap_or_else(|| ("Mouse".into(), vec!["Left".into(), "Right".into()]));
    (kb_name, kc, travel, m_name, names)
}

impl Shell {
    fn obs_save(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = devhub_obs::config::save(&self.obs_cfg) {
            self.toast(format!("Cannot save OBS settings: {e}"), true, cx);
        }
        cx.notify();
    }

    pub(crate) fn obs_edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut OverlayCfg)) {
        f(&mut self.obs_cfg);
        self.obs_save(cx);
    }

    /// Redraw the preview image (the previous image is released from the GPU).
    fn obs_preview(&mut self, window: &mut Window) -> Option<(Arc<RenderImage>, u32, u32)> {
        let (kb_name, kc, travel, m_name, names) = targets(self);
        let live = render::demo_live(
            Some((kb_name, kc.layout.clone(), travel)),
            Some((m_name, names)),
            self.obs_t,
        );
        let vis = render::Vis::default().with_cfg(&self.obs_cfg);
        let font = self
            .obs_font
            .get_or_insert_with(|| Rc::new(render::font()))
            .clone();
        let pix = render::draw(&live, (*font).as_ref(), self.obs_mode, &vis);
        let (w, h) = (pix.w, pix.h);
        let mut buf = pix.buf;
        // RenderImage is BGRA.
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let img = image::RgbaImage::from_raw(w, h, buf)?;
        let ri = Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
            img
        )]));
        if let Some(old) = self.obs_img.replace(ri.clone()) {
            let _ = window.drop_image(old);
        }
        Some((ri, w, h))
    }

    pub fn page_obs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let cfg = self.obs_cfg.clone();
        let (_, kc, _, _, names) = targets(self);

        // ---------------- Header
        let header = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .size(px(48.))
                    .rounded(px(14.))
                    .bg(p.accent_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ui::icon("cast", 22., p.accent)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(ui::title("OBS overlay"))
                    .child(ui::sub("Live display of key/button press depth on your stream. Changes made here reach OBS within 1 second")),
            );

        // ---------------- Preview
        let modes = ["Keyboard", "Mouse", "Both"].map(String::from);
        let cur_mode = match self.obs_mode {
            Mode::Kb => 0,
            Mode::Mouse => 1,
            Mode::Both => 2,
        };
        let mode_sw = div().w(px(300.)).child(ui::segmented(
            "obs-mode",
            &modes,
            cur_mode,
            cx,
            |this, i, cx| {
                this.obs_mode = [Mode::Kb, Mode::Mouse, Mode::Both][i];
                cx.notify();
            },
        ));
        let src_name = match self.obs_mode {
            Mode::Kb => "Device HUB Keyboard",
            Mode::Mouse => "Device HUB Mouse",
            Mode::Both => "Device HUB Live",
        };
        let mut stage = div()
            .w_full()
            .min_h(px(200.))
            .p(px(18.))
            .rounded(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            // Placed on a dark stream-like gradient so that transparency is visible
            .bg(linear_gradient(
                135.,
                linear_color_stop(theme::hex(0x1b2a4a, 1.), 0.),
                linear_color_stop(theme::hex(0x3a2140, 1.), 1.),
            ));
        if let Some((img_, w, h)) = self.obs_preview(window) {
            let max_w = 880f32;
            let k = (max_w / w as f32).min(1.);
            stage = stage.child(
                img(img_)
                    .w(px(w as f32 * k))
                    .h(px(h as f32 * k))
                    .object_fit(ObjectFit::Contain),
            );
        }
        let preview = ui::card_titled("Preview", "monitor-play", Some(mode_sw.into_any_element())).child(
            ui::body()
                .pt(px(8.))
                .gap(px(8.))
                .child(stage)
                .child(ui::faint(format!(
                    "In OBS, add it via Add source → \"{src_name}\" (this is a demo animation; the real display follows press depth on the connected device)"
                ))),
        );

        // ---------------- Design
        let themes: Vec<String> = ObsTheme::ALL
            .iter()
            .map(|t| t.label().to_string())
            .collect();
        let theme_i = ObsTheme::ALL
            .iter()
            .position(|t| *t == cfg.theme)
            .unwrap_or(0);
        let mstyles: Vec<String> = MouseStyle::ALL
            .iter()
            .map(|t| t.label().to_string())
            .collect();
        let ms_i = MouseStyle::ALL
            .iter()
            .position(|t| *t == cfg.mouse_style)
            .unwrap_or(0);
        let scales = ["1x", "1.5x", "2x"].map(String::from);
        let scale_i = [1.0f32, 1.5, 2.0]
            .iter()
            .position(|s| (cfg.scale - s).abs() < 0.01)
            .unwrap_or(0);
        let cur_acc = Rgb::new(cfg.accent[0], cfg.accent[1], cfg.accent[2]);
        let mut acc_row = div().flex().items_center().gap(px(6.));
        for hex in ACCENT_CHOICES {
            let c = Rgb::hex(hex);
            acc_row = acc_row.child(
                div()
                    .id(ElementId::Name(format!("obs-acc-{hex:06x}").into()))
                    .cursor_pointer()
                    .child(ui::swatch(theme::rgb_of(c), 20., c == cur_acc))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.obs_edit(cx, |c2| c2.accent = [c.r, c.g, c.b]);
                    })),
            );
        }
        let custom = !ACCENT_CHOICES.iter().any(|h| Rgb::hex(*h) == cur_acc);
        acc_row = acc_row.child(
            div()
                .id("obs-acc-custom")
                .cursor_pointer()
                .child(if custom {
                    ui::swatch(theme::rgb_of(cur_acc), 20., true)
                } else {
                    div()
                        .size(px(20.))
                        .rounded_full()
                        .border_1()
                        .border_color(p.hair)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::icon("plus", 12., p.sub))
                })
                .on_click(cx.listener(move |this, _, w, cx| {
                    overlay::pick_color(this, "Accent color", cur_acc, w, cx, |this, c, _, cx| {
                        this.obs_edit(cx, |c2| c2.accent = [c.r, c.g, c.b]);
                    });
                })),
        );
        // Swatch for the press color (Auto = follows the accent).
        let cur_pre = cfg.pressed.map(|c| Rgb::new(c[0], c[1], c[2]));
        let mut pre_row = div().flex().items_center().gap(px(6.));
        pre_row = pre_row.child(
            div()
                .id("obs-pre-auto")
                .cursor_pointer()
                .px(px(8.))
                .h(px(20.))
                .rounded(px(10.))
                .border_1()
                .border_color(if cur_pre.is_none() { p.accent } else { p.hair })
                .bg(if cur_pre.is_none() {
                    p.accent_soft
                } else {
                    gpui::transparent_black()
                })
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(if cur_pre.is_none() { p.accent } else { p.sub })
                .child("Auto")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.obs_edit(cx, |c| c.pressed = None);
                })),
        );
        for hex in ACCENT_CHOICES {
            let c = Rgb::hex(hex);
            let sel = cur_pre == Some(c);
            pre_row = pre_row.child(
                div()
                    .id(ElementId::Name(format!("obs-pre-{hex:06x}").into()))
                    .cursor_pointer()
                    .child(ui::swatch(theme::rgb_of(c), 20., sel))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.obs_edit(cx, |c2| c2.pressed = Some([c.r, c.g, c.b]));
                    })),
            );
        }
        {
            let pcustom =
                cur_pre.is_some_and(|c| !ACCENT_CHOICES.iter().any(|h| Rgb::hex(*h) == c));
            pre_row = pre_row.child(
                div()
                    .id("obs-pre-custom")
                    .cursor_pointer()
                    .child(if let (true, Some(c)) = (pcustom, cur_pre) {
                        ui::swatch(theme::rgb_of(c), 20., true).into_any_element()
                    } else {
                        div()
                            .size(px(20.))
                            .rounded_full()
                            .border_1()
                            .border_color(p.hair)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(ui::icon("plus", 12., p.sub))
                            .into_any_element()
                    })
                    .on_click(cx.listener(move |this, _, w, cx| {
                        let base = cur_pre.unwrap_or(Rgb::new(255, 150, 60));
                        overlay::pick_color(this, "Press color", base, w, cx, |this, c, _, cx| {
                            this.obs_edit(cx, |c2| c2.pressed = Some([c.r, c.g, c.b]));
                        });
                    })),
            );
        }
        let fills: Vec<String> = FillMode::ALL
            .iter()
            .map(|f| f.label().to_string())
            .collect();
        let fill_i = FillMode::ALL
            .iter()
            .position(|f| *f == cfg.fill_mode)
            .unwrap_or(0);
        let title_in = self.input(
            "obs_title",
            &cfg.title_text,
            "Blank = device name",
            window,
            cx,
        );
        let design = ui::card_titled("Design", "palette", None).child(
            ui::body()
                .child(ui::row_first(
                    "Theme",
                    Some("\"No background\" overlays only the keys on the stream"),
                    div().w(px(360.)).child(ui::segmented(
                        "obs-theme",
                        &themes,
                        theme_i,
                        cx,
                        |this, i, cx| {
                            this.obs_edit(cx, |c| c.theme = ObsTheme::ALL[i]);
                        },
                    )),
                ))
                .child(ui::row(
                    "Accent color",
                    Some("Color of the frame, title and Hz. A color chosen in the OBS source settings takes priority"),
                    acc_row,
                ))
                .child(ui::row(
                    "Press color",
                    Some("Fill, glow and outline while pressed. \"Auto\" is the same as the accent"),
                    pre_row,
                ))
                .child(ui::row(
                    "Fill direction",
                    Some("How the press amount is filled"),
                    div().w(px(300.)).child(ui::segmented(
                        "obs-fill",
                        &fills,
                        fill_i,
                        cx,
                        |this, i, cx| {
                            this.obs_edit(cx, |c| c.fill_mode = FillMode::ALL[i]);
                        },
                    )),
                ))
                .child(ui::row(
                    "Background opacity",
                    Some("How see-through the panel is. 0% is the same as \"No background\""),
                    ui::slider_value(
                        "obs-alpha",
                        cfg.panel_alpha as f32,
                        0.,
                        100.,
                        5.,
                        "%",
                        cx,
                        |this, v, cx| {
                            this.obs_edit(cx, |c| c.panel_alpha = v as u8);
                        },
                    ),
                ))
                .child(ui::row(
                    "Glow strength",
                    Some("Outer glow when pressed. 0 turns it off"),
                    ui::slider_value(
                        "obs-glow",
                        cfg.glow as f32,
                        0.,
                        200.,
                        10.,
                        "%",
                        cx,
                        |this, v, cx| {
                            this.obs_edit(cx, |c| c.glow = v as u8);
                        },
                    ),
                ))
                .child(ui::row(
                    "Shadow",
                    Some("Drop shadow of keys and panel"),
                    ui::toggle("obs-shadow", cfg.shadow, cx, |this, v, cx| {
                        this.obs_edit(cx, |c| c.shadow = v)
                    }),
                ))
                .child(ui::row(
                    "Resolution",
                    Some("If you scale up in OBS, use 1.5x / 2x to avoid blurring"),
                    div().w(px(220.)).child(ui::segmented(
                        "obs-scale",
                        &scales,
                        scale_i,
                        cx,
                        |this, i, cx| {
                            this.obs_edit(cx, |c| c.scale = [1.0, 1.5, 2.0][i]);
                        },
                    )),
                ))
                .child(ui::row(
                    "Mouse display",
                    None,
                    div().w(px(300.)).child(ui::segmented(
                        "obs-mstyle",
                        &mstyles,
                        ms_i,
                        cx,
                        |this, i, cx| {
                            this.obs_edit(cx, |c| c.mouse_style = MouseStyle::ALL[i]);
                        },
                    )),
                )),
        );

        // ---------------- Layout
        let arranges: Vec<String> = Arrange::ALL.iter().map(|a| a.label().to_string()).collect();
        let arr_i = Arrange::ALL
            .iter()
            .position(|a| *a == cfg.arrange)
            .unwrap_or(0);
        let layout_card = ui::card_titled("Layout", "layout-dashboard", None).child(
            ui::body()
                .child(ui::row_first(
                    "Key size",
                    Some(
                        "Size of one key. Only the density changes; the resolution stays the same",
                    ),
                    ui::slider_value(
                        "obs-ksize",
                        cfg.key_size,
                        0.7,
                        1.4,
                        0.05,
                        "x",
                        cx,
                        |this, v, cx| {
                            this.obs_edit(cx, |c| c.key_size = v);
                        },
                    ),
                ))
                .child(ui::row(
                    "Corner radius",
                    Some("Roundness of keys and panel. 0 is square"),
                    ui::slider_value(
                        "obs-radius",
                        cfg.radius,
                        0.,
                        2.,
                        0.1,
                        "x",
                        cx,
                        |this, v, cx| {
                            this.obs_edit(cx, |c| c.radius = v);
                        },
                    ),
                ))
                .child(ui::row(
                    "Padding",
                    Some("Empty space inside the panel"),
                    ui::slider_value("obs-pad", cfg.pad, 0.5, 2., 0.1, "x", cx, |this, v, cx| {
                        this.obs_edit(cx, |c| c.pad = v);
                    }),
                ))
                .child(ui::row(
                    "Arrangement",
                    Some("Order in the combined keyboard + mouse source"),
                    div().w(px(300.)).child(ui::segmented(
                        "obs-arr",
                        &arranges,
                        arr_i,
                        cx,
                        |this, i, cx| {
                            this.obs_edit(cx, |c| c.arrange = Arrange::ALL[i]);
                        },
                    )),
                )),
        );

        // ---------------- What to show
        let show_card = ui::card_titled("What to show", "eye", None).child(
            ui::body()
                .child(ui::row_first(
                    "Title (device name)",
                    Some("You can change the text in the input on the right (blank = device name)"),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(ui::toggle("obs-title", cfg.title, cx, |this, v, cx| {
                            this.obs_edit(cx, |c| c.title = v)
                        }))
                        .child(
                            div()
                                .w(px(200.))
                                .child(gpui_kit::component::input::Input::new(&title_in)),
                        ),
                ))
                .child(ui::row(
                    "Key / button names",
                    None,
                    ui::toggle("obs-labels", cfg.labels, cx, |this, v, cx| {
                        this.obs_edit(cx, |c| c.labels = v)
                    }),
                ))
                .child(ui::row(
                    "Setting badge (AP/R)",
                    Some("Actuation point and reset values inside the key (with RT, the two press/release values)"),
                    ui::toggle("obs-badges", cfg.badges, cx, |this, v, cx| {
                        this.obs_edit(cx, |c| c.badges = v)
                    }),
                ))
                .child(ui::row(
                    "Actuation point line",
                    Some("Horizontal line inside the key, at the depth where it turns ON"),
                    ui::toggle("obs-tline", cfg.trigger_line, cx, |this, v, cx| {
                        this.obs_edit(cx, |c| c.trigger_line = v)
                    }),
                ))
                .child(ui::row(
                    "Press amount number",
                    Some("Shows the depth under the key only while it is pressed"),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(ui::toggle("obs-num", cfg.numbers, cx, |this, v, cx| {
                            this.obs_edit(cx, |c| c.numbers = v)
                        }))
                        .child(ui::segmented(
                            "obs-numfmt",
                            &["mm", "%"].map(String::from),
                            cfg.num_pct as usize,
                            cx,
                            |this, i, cx| this.obs_edit(cx, |c| c.num_pct = i == 1),
                        )),
                )),
        );

        // ---------------- Keys to show
        // show_keys empty = show all. Click to choose the set to show.
        let restricted = !cfg.show_keys.is_empty();
        let caps_v: Vec<Cap> = kc
            .layout
            .iter()
            .map(|k| {
                let shown = !cfg.hide_keys.contains(&k.id)
                    && (!restricted || cfg.show_keys.contains(&k.id));
                Cap {
                    top: k.label.clone(),
                    dim: !shown,
                    tint: (restricted && cfg.show_keys.contains(&k.id))
                        .then(|| theme::alpha(theme::rgb_of(cur_acc), 0.28)),
                    badge: (!shown).then(|| "Hidden".into()),
                    ..Default::default()
                }
            })
            .collect();
        let ids: Vec<u16> = kc.layout.iter().map(|k| k.id).collect();
        let ids2 = ids.clone();
        let view = self.keyboard_view(
            "obs-keys",
            &kc,
            caps_v,
            cx,
            Rc::new(move |this, i, _, cx| {
                let id = ids2[i];
                this.obs_edit(cx, |c| {
                    if c.show_keys.is_empty() {
                        // Show all → show only this key
                        c.show_keys = vec![id];
                    } else if let Some(pos) = c.show_keys.iter().position(|x| *x == id) {
                        c.show_keys.remove(pos);
                    } else {
                        c.show_keys.push(id);
                    }
                });
            }),
        );
        let only = |labels: &'static [&'static str]| {
            let layout = kc.layout.clone();
            move |this: &mut Shell, _: &mut Window, cx: &mut Context<Shell>| {
                let show: Vec<u16> = layout
                    .iter()
                    .filter(|k| labels.iter().any(|l| l.eq_ignore_ascii_case(&k.label)))
                    .map(|k| k.id)
                    .collect();
                this.obs_edit(cx, |c| c.show_keys = show);
            }
        };
        let left_half = {
            let layout = kc.layout.clone();
            move |this: &mut Shell, _: &mut Window, cx: &mut Context<Shell>| {
                // Left-hand side (up to the G column + the left modifier keys).
                let show: Vec<u16> = layout
                    .iter()
                    .filter(|k| k.x + k.w / 2. <= 6.6)
                    .map(|k| k.id)
                    .collect();
                this.obs_edit(cx, |c| c.show_keys = show);
            }
        };
        let n_shown = ids
            .iter()
            .filter(|id| !cfg.hide_keys.contains(id) && (!restricted || cfg.show_keys.contains(id)))
            .count();
        let key_tools = div()
            .flex()
            .gap(px(6.))
            .child(ui::button(
                "obs-k-all",
                "Show all",
                Some("eye"),
                Btn::Ghost,
                cx,
                |this, _, cx| {
                    this.obs_edit(cx, |c| {
                        c.show_keys.clear();
                        c.hide_keys.clear();
                    });
                },
            ))
            .child(ui::button(
                "obs-k-wasd",
                "WASD + Space",
                None,
                Btn::Ghost,
                cx,
                only(&["W", "A", "S", "D", "Space"]),
            ))
            .child(ui::button(
                "obs-k-fps",
                "FPS",
                None,
                Btn::Ghost,
                cx,
                only(&[
                    "Esc", "1", "2", "3", "4", "5", "Tab", "Q", "W", "E", "R", "T", "Caps", "A",
                    "S", "D", "F", "G", "LShift", "Shift", "Z", "X", "C", "V", "LCtrl", "Ctrl",
                    "LAlt", "Space",
                ]),
            ))
            .child(ui::button(
                "obs-k-left",
                "Left half",
                None,
                Btn::Ghost,
                cx,
                left_half,
            ))
            .child({
                let ids3 = ids.clone();
                let cur_show = cfg.show_keys.clone();
                ui::button(
                    "obs-k-inv",
                    "Invert",
                    None,
                    Btn::Ghost,
                    cx,
                    move |this, _, cx| {
                        let ids3 = ids3.clone();
                        let cur_show = cur_show.clone();
                        this.obs_edit(cx, |c| {
                            // Select the complement of the currently effective shown set (empty = show all → invert never ends up empty by deselecting every key).
                            let cur: std::collections::HashSet<u16> = if cur_show.is_empty() {
                                ids3.iter().copied().collect()
                            } else {
                                cur_show.iter().copied().collect()
                            };
                            c.show_keys = ids3
                                .iter()
                                .copied()
                                .filter(|id| !cur.contains(id))
                                .collect();
                            if c.show_keys.len() == ids3.len() {
                                c.show_keys.clear(); // Selecting everything is the same as "Show all"
                            }
                        });
                    },
                )
            });
        let keys_card = ui::card_titled("Keys to show", "keyboard", Some(key_tools.into_any_element())).child(
            ui::body()
                .pt(px(10.))
                .gap(px(8.))
                .child(view)
                .child(ui::faint(format!(
                    "Click to select / deselect the keys to show ({n_shown}/{} keys shown. None selected = show all). The overlay is cropped to just the keys shown",
                    ids.len()
                ))),
        );

        // ---------------- Mouse buttons to show (show_buttons empty = show all)
        let btn_restricted = !cfg.show_buttons.is_empty();
        let mut chips = div().flex().flex_wrap().gap(px(8.));
        for (i, name) in names.iter().enumerate() {
            let off = cfg.hide_buttons.contains(name)
                || (btn_restricted && !cfg.show_buttons.contains(name));
            let n2 = name.clone();
            chips = chips.child(
                div()
                    .id(ElementId::Name(format!("obs-btn-{i}").into()))
                    .px(px(12.))
                    .py(px(7.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .bg(if off {
                        theme::alpha(p.faint, 0.06)
                    } else {
                        p.accent_soft
                    })
                    .border_1()
                    .border_color(if off { p.hair } else { p.accent })
                    .child(ui::icon(
                        if off { "eye-off" } else { "eye" },
                        14.,
                        if off { p.faint } else { p.accent },
                    ))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(if off { p.faint } else { p.fg })
                            .child(name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let n2 = n2.clone();
                        this.obs_edit(cx, |c| {
                            if c.show_buttons.is_empty() {
                                // Show all → show only this button
                                c.show_buttons = vec![n2];
                            } else if let Some(pos) = c.show_buttons.iter().position(|x| *x == n2) {
                                c.show_buttons.remove(pos);
                            } else {
                                c.show_buttons.push(n2);
                            }
                        });
                    })),
            );
        }
        let btn_card = ui::card_titled("Mouse buttons to show", "mouse", None).child(
            ui::body()
                .pt(px(8.))
                .gap(px(8.))
                .child(chips)
                .child(ui::faint(
                    "Only buttons whose press depth can be measured (magnetic switches) are listed. Click to toggle",
                )),
        );

        // ---------------- How to install
        let path = devhub_obs::config::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let step = |n: &str, t: &str| {
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(20.))
                        .flex_none()
                        .rounded_full()
                        .bg(p.accent_soft)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .text_color(p.accent)
                        .child(n.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.5))
                        .text_color(p.sub)
                        .child(t.to_string()),
                )
        };
        let howto = ui::card_titled("Add to OBS", "badge-info", None).child(
            ui::body()
                .pt(px(8.))
                .gap(px(10.))
                .child(step("1", "Put devhub_obs.dll in C:\\ProgramData\\obs-studio\\plugins\\devhub_obs\\bin\\64bit\\ and start OBS"))
                .child(step("2", "Add source → add \"Device HUB Keyboard\" and \"Device HUB Mouse\" separately (to combine both in one, use \"Device HUB Live\")"))
                .child(step("3", "Change the appearance and shown keys on this page — applied to all sources immediately. You can also pick the keys/buttons to show in the source properties"))
                .child(step("4", "Device HUB itself can stay closed (the OBS plugin reads the device directly)"))
                .child(div().min_w_0().child(ui::faint(format!("Settings file: {path}")))),
        );

        div()
            .id("obs-scroll")
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
                    .child(header)
                    .child(preview)
                    .child(design)
                    .child(layout_card)
                    .child(show_card)
                    .child(keys_card)
                    .child(ui::grid2(vec![
                        btn_card.into_any_element(),
                        howto.into_any_element(),
                    ])),
            )
    }
}
