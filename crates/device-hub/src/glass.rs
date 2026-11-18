//! Liquid Glass surfaces.
//!
//! With the GPU backdrop-filter added to vendor/gpui-pre (`Window::paint_backdrop`),
//! **what is actually drawn behind the surface** (wallpaper, cards, text) is blurred and
//! refracted on the GPU every frame (CSS `backdrop-filter` + edge lens), in the same drawing order as the reference
//! implementation:
//!
//!   backdrop (blur/refraction) → surface fill (top→bottom gradient) → top-edge glow → thin rim → bottom-edge shadow
//!
//! Usage: make the parent div `.relative()` and put `glass::surface(Kind, r)` as its first child.
//! Turning glass off (setting / `DEVHUB_FAST=1`) falls back to an opaque surface.

use crate::motion;
use crate::theme::{self, pal};
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Card that holds content. Strong frosting; only the edge is refracted lightly.
    Panel,
    /// Controls floating directly over the wallpaper (tab bar, sidebar, toolbar). Crisp lens.
    Clear,
    /// Controls sitting on a card (buttons, inputs). No refraction, light frosting.
    Frost,
    /// Popups and dialogs. The strongest frosting.
    Sheet,
}

/// Glass surface (an absolutely positioned canvas filling its parent).
pub fn surface(kind: Kind, radius: f32) -> impl IntoElement {
    surface_keyed(kind, radius, None)
}

/// Glass that reflects light on hover (`key` is the motion spring key).
pub fn surface_hover(kind: Kind, radius: f32, key: impl Into<String>) -> impl IntoElement {
    surface_keyed(kind, radius, Some(key.into()))
}

fn surface_keyed(kind: Kind, radius: f32, key: Option<String>) -> impl IntoElement {
    canvas(
        |b, _, _| b,
        move |_, bounds, window, _| paint(kind, radius, key.as_deref(), bounds, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn paint(kind: Kind, radius: f32, key: Option<&str>, bounds: Bounds<Pixels>, window: &mut Window) {
    let p = pal();
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w < 1. || h < 1. {
        return;
    }
    let r = radius.min(w / 2.).min(h / 2.);
    let corners = Corners::all(px(r));
    let glass = theme::glass_on();

    // Hover reflection (spring).
    let energy = key
        .map(|k| {
            let m = window.mouse_position();
            let inside = bounds.contains(&m);
            let target = if inside {
                if motion::is_down() {
                    1.
                } else {
                    0.5
                }
            } else {
                0.
            };
            motion::spring_with(&format!("glass:{k}"), target, 20.)
        })
        .unwrap_or(0.);

    if glass {
        let small = w.min(h);
        let filter = match kind {
            Kind::Panel => BackdropFilter {
                blur: 22.,
                saturation: 1.7,
                brightness: if p.light { 1.06 } else { 0.86 },
                bezel: (r * 0.55).clamp(6., 16.),
                refraction: (r * 0.45).clamp(4., 12.),
                dispersion: 0.05,
                light: if p.light {
                    [0.9, 0.3, 0.22, 0.05]
                } else {
                    [0.5, 0.12, 0.14, 0.3]
                },
                rim: rim(p.light, small),
                lens: if p.light {
                    [1., 0.08, 0., 0.3]
                } else {
                    [1., 0.16, 0., 0.26]
                },
                ..Default::default()
            },
            Kind::Sheet => BackdropFilter {
                blur: 34.,
                saturation: 1.8,
                brightness: if p.light { 1.08 } else { 0.78 },
                bezel: (r * 0.5).clamp(6., 14.),
                refraction: (r * 0.4).clamp(4., 10.),
                dispersion: 0.05,
                rim: rim(p.light, small),
                ..Default::default()
            },
            Kind::Clear => BackdropFilter {
                blur: 2.,
                saturation: 1.25,
                brightness: if p.light { 1.04 } else { 0.95 },
                glass: [2., energy, if p.light { 1. } else { 0. }, 1.],
                ..Default::default()
            },
            Kind::Frost => BackdropFilter {
                blur: 10.,
                saturation: 1.3,
                brightness: if p.light { 1.03 } else { 1.05 },
                rim: if small >= 20. {
                    rim(p.light, small)
                } else {
                    [0.; 4]
                },
                ..Default::default()
            },
        };
        window.paint_backdrop(bounds, corners, filter);
    }

    // Surface fill.
    let (top, bot) = match kind {
        Kind::Panel => (p.glass_top, p.glass_bot),
        Kind::Sheet => (
            theme::alpha(p.glass_top, (p.glass_top.a + 0.22).min(0.96)),
            theme::alpha(p.glass_bot, (p.glass_bot.a + 0.26).min(0.96)),
        ),
        Kind::Clear => (
            theme::alpha(p.glass_top, p.glass_top.a * 0.55 + energy * 0.08),
            theme::alpha(p.glass_bot, p.glass_bot.a * 0.45 + energy * 0.06),
        ),
        Kind::Frost => (
            theme::alpha(p.ctrl, p.ctrl.a + energy * 0.08),
            theme::alpha(p.ctrl, p.ctrl.a * 0.7 + energy * 0.06),
        ),
    };
    let (top, bot) = if glass {
        (top, bot)
    } else {
        // Opaque fallback.
        let base = match kind {
            Kind::Frost => theme::mix(p.solid, p.fg, 0.06 + energy * 0.04),
            _ => theme::mix(p.solid, p.fg, if p.light { 0.0 } else { 0.03 }),
        };
        (base, base)
    };
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: corners,
        background: linear_gradient(180., linear_color_stop(top, 0.), linear_color_stop(bot, 1.)),
        border_widths: Edges::all(px(1.)),
        border_color: if glass { p.edge } else { p.hair },
        border_style: BorderStyle::default(),
    });

    // Top-edge glow (only the straight part, avoiding the rounded-corner arcs).
    if glass && h >= 14. {
        let inset = (r * 0.8).min(w / 3.);
        let lw = (w - inset * 2.).max(0.);
        if lw > 2. {
            window.paint_quad(PaintQuad {
                bounds: Bounds {
                    origin: point(bounds.origin.x + px(inset), bounds.origin.y + px(0.5)),
                    size: size(px(lw), px(1.)),
                },
                corner_radii: Corners::default(),
                background: linear_gradient(
                    90.,
                    linear_color_stop(theme::alpha(p.rim, 0.), 0.),
                    linear_color_stop(p.rim, 0.5),
                ),
                border_widths: Edges::default(),
                border_color: Hsla::default(),
                border_style: BorderStyle::default(),
            });
        }
    }
}

/// Glossy rim on the edge (rim width, whiteness, key light, fill light).
fn rim(light: bool, small: f32) -> [f32; 4] {
    let w = if small <= 72. { 1.4 } else { 2.2 };
    if light {
        [w, 0.12, 0.8, 0.36]
    } else {
        [w, 0.07, 0.55, 0.2]
    }
}

/// Round lens (toggle knob, slider knob). Swells like a bubble only while pressed.
pub fn bubble(bounds: Bounds<Pixels>, r: f32, window: &mut Window) {
    if !theme::glass_on() {
        return;
    }
    let light = theme::is_light();
    let d = r * 2.;
    window.paint_backdrop(
        bounds,
        Corners::all(px(r)),
        BackdropFilter {
            lens: [1.22, if light { 0.14 } else { 0.42 }, 0., 0.],
            bezel: (d * 0.16).max(2.),
            refraction: (d * 0.13).max(2.),
            dispersion: 0.07,
            saturation: 1.08,
            brightness: 1.03,
            rim: if light {
                [0., 0., 0.85, 0.42]
            } else {
                [0., 0., 0.32, 0.16]
            },
            ..Default::default()
        },
    );
}
