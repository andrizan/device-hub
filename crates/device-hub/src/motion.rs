//! Real-time animation and recording of element positions.
//!
//! - `spring(key, target)` … returns the current value moved toward target by a critically damped spring.
//!   While it is moving, `end_frame` requests the next frame (when it stops, drawing stops too =
//!   0 CPU/GPU when idle — light even when open in the background during a game).
//! - `track(id)` … a transparent canvas that records the element's rect every frame.
//!   Used for slider drags, popup alignment and hover tests.

use gpui_kit::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Copy)]
struct Spring {
    value: f32,
    vel: f32,
    frame: u64,
}

#[derive(Default)]
struct Sys {
    springs: HashMap<String, Spring>,
    rects: HashMap<String, [f32; 4]>,
    pointer: [f32; 2],
    down: bool,
    last: Option<Instant>,
    dt: f32,
    frame: u64,
    dirty: bool,
}

thread_local! {
    static SYS: RefCell<Sys> = RefCell::new(Sys { dt: 1. / 60., ..Default::default() });
}

/// Call at the start of a frame. Measures the elapsed time.
pub fn begin_frame() {
    SYS.with(|s| {
        let mut s = s.borrow_mut();
        let now = Instant::now();
        s.dt = s
            .last
            .map(|l| (now - l).as_secs_f32())
            .unwrap_or(1. / 60.)
            .clamp(0.001, 0.05);
        s.last = Some(now);
        s.frame += 1;
        s.dirty = false;
        if s.springs.len() > 2048 {
            let f = s.frame;
            s.springs.retain(|_, v| f - v.frame < 600);
        }
    });
}

/// Call at the end of a frame. If a spring is moving, requests the next frame.
pub fn end_frame(window: &mut Window) {
    let dirty = SYS.with(|s| {
        let mut s = s.borrow_mut();
        let d = s.dirty;
        if !d {
            s.last = None; // time while stopped is not counted
        }
        d
    });
    if dirty {
        window.request_animation_frame();
    }
}

/// Critically damped spring. A larger `speed` is faster (12 ≈ 0.35s).
pub fn spring_with(key: &str, target: f32, speed: f32) -> f32 {
    if !crate::theme::motion_on() {
        return target;
    }
    SYS.with(|s| {
        let mut s = s.borrow_mut();
        let dt = s.dt;
        let frame = s.frame;
        let sp = s.springs.entry(key.to_string()).or_insert(Spring {
            value: target,
            vel: 0.,
            frame,
        });
        if sp.frame == frame {
            return sp.value;
        }
        sp.frame = frame;
        // Critical damping: x'' = -w^2 (x - t) - 2w x'
        let w = speed;
        let mut x = sp.value;
        let mut v = sp.vel;
        // Stabilize with finely subdivided semi-implicit Euler.
        let steps = 4;
        let h = dt / steps as f32;
        for _ in 0..steps {
            let a = -w * w * (x - target) - 2. * w * v;
            v += a * h;
            x += v * h;
        }
        let settled = (x - target).abs() < 0.0008 && v.abs() < 0.002;
        if settled {
            x = target;
            v = 0.;
        }
        sp.value = x;
        sp.vel = v;
        if !settled {
            s.dirty = true;
        }
        x
    })
}

pub fn spring(key: &str, target: f32) -> f32 {
    spring_with(key, target, 16.)
}

/// A value that smooths a 0/1 state.
pub fn ease(key: &str, on: bool) -> f32 {
    spring_with(key, if on { 1. } else { 0. }, 18.)
}

/// Tell it from outside to "draw the next frame too" (for time-varying displays).
pub fn keep_alive() {
    SYS.with(|s| s.borrow_mut().dirty = true);
}

// ------------------------------------------------------------------ Position

/// A transparent layer that records an element's rect (fills its parent).
pub fn track(id: impl Into<String>) -> impl IntoElement {
    let id: String = id.into();
    canvas(
        move |bounds, _, _| {
            SYS.with(|s| {
                s.borrow_mut().rects.insert(
                    id.clone(),
                    [
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    ],
                );
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Rect [x, y, w, h] recorded in the previous frame.
pub fn rect(id: &str) -> Option<[f32; 4]> {
    SYS.with(|s| s.borrow().rects.get(id).copied())
}

pub fn set_pointer(x: f32, y: f32) {
    SYS.with(|s| s.borrow_mut().pointer = [x, y]);
}

pub fn pointer() -> [f32; 2] {
    SYS.with(|s| s.borrow().pointer)
}

pub fn set_down(d: bool) {
    SYS.with(|s| s.borrow_mut().down = d);
}

pub fn is_down() -> bool {
    SYS.with(|s| s.borrow().down)
}

/// Whether the cursor is over the element (judged with the previous frame's rect).
pub fn hovered(id: &str) -> bool {
    let p = pointer();
    rect(id)
        .is_some_and(|r| p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3])
}
