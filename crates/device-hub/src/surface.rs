//! Flat surfaces.
//!
//! Every card, bar, control and popup gets its fill from here: one opaque
//! rounded quad with a 1px border. No blur, no refraction, no gradient, so a
//! frame costs the same whatever sits behind a surface.
//!
//! This replaced Liquid Glass (a GPU backdrop-filter design) on 2026-11-19 to
//! keep the app light. Do not bring back a GPU backdrop filter or translucent layers.
//!
//! Usage: make the parent div `.relative()` and put `surface::fill(Kind, r)`
//! as its first child.

use crate::theme::{self, pal, Pal};
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Card that holds content.
    Card,
    /// App chrome on the window background (sidebar, tab bar, toolbar).
    Bar,
    /// Control sitting on a card (buttons, inputs, selects).
    Control,
    /// Popups, toasts and dialogs.
    Popup,
}

fn colors(kind: Kind, p: &Pal) -> (Hsla, Hsla) {
    match kind {
        Kind::Card => (p.card, p.hair),
        Kind::Bar => (p.card, p.hair),
        Kind::Control => (p.ctrl, p.hair),
        Kind::Popup => (p.raised, p.edge),
    }
}

/// Surface filling its parent (absolutely positioned).
pub fn fill(kind: Kind, radius: f32) -> Div {
    let p = pal();
    let (bg, border) = colors(kind, &p);
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(radius))
        .bg(bg)
        .border_1()
        .border_color(border)
}

/// Same, a step lighter while the pointer is over it.
pub fn fill_hover(kind: Kind, radius: f32, id: impl Into<SharedString>) -> Stateful<Div> {
    let p = pal();
    let hover = match kind {
        Kind::Control => p.ctrl_hover,
        _ => theme::mix(p.card, p.fg, if p.light { 0.03 } else { 0.04 }),
    };
    fill(kind, radius).id(id.into()).hover(move |s| s.bg(hover))
}
