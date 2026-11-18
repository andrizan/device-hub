//! Test: check clicks / wheel / key input / chatter on the spot (a feature the web version lacks).
//!
//! Everything is measured from input events delivered by the OS, so it works even without a driver.
//! A page to confirm "it is really taking effect" right after you change a setting.

use crate::app::Shell;
use crate::pages::keyboard::{kb_parts, Cap};
use crate::ui::{self, Btn};
use crate::{keycode, motion, theme};
use devhub_core::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::time::Instant;

/// Consecutive clicks/keystrokes with a shorter interval than this are "suspected chatter".
pub const CHATTER_MS: u128 = 25;

#[derive(Clone, Default)]
pub struct BtnStat {
    pub count: u32,
    pub last: Option<Instant>,
    pub last_gap: Option<u128>,
    pub min_gap: Option<u128>,
    pub chatter: u32,
}

impl BtnStat {
    fn hit(&mut self) {
        let now = Instant::now();
        if let Some(l) = self.last {
            let g = (now - l).as_millis();
            self.last_gap = Some(g);
            self.min_gap = Some(self.min_gap.map_or(g, |m| m.min(g)));
            if g < CHATTER_MS {
                self.chatter += 1;
            }
        }
        self.last = Some(now);
        self.count += 1;
    }
}

impl TestState {
    /// Record one key event (rounded to the last 3 seconds).
    fn ev_push(&mut self) {
        let now = Instant::now();
        self.ev_ts.push_back(now);
        while self
            .ev_ts
            .front()
            .is_some_and(|t| now.duration_since(*t).as_millis() > 3000)
        {
            self.ev_ts.pop_front();
        }
    }

    /// (events/s over the last 3 seconds, minimum interval between consecutive events in ms).
    fn ev_rate(&self) -> (usize, Option<f32>) {
        let n = self.ev_ts.len();
        let min = self
            .ev_ts
            .iter()
            .zip(self.ev_ts.iter().skip(1))
            .map(|(a, b)| b.duration_since(*a).as_secs_f32() * 1000.)
            .reduce(f32::min);
        (n / 3, min)
    }
}

#[derive(Default)]
pub struct TestState {
    /// 0 left, 1 right, 2 middle, 3 back, 4 forward
    pub buttons: [BtnStat; 5],
    pub wheel_up: u32,
    pub wheel_down: u32,
    /// Times of move events on the pad (last 1 second).
    pub moves: VecDeque<Instant>,
    pub move_peak: usize,
    /// Keys: usages being pressed, keystroke count, stats.
    pub down: BTreeSet<u16>,
    pub keys: BTreeMap<u16, BtnStat>,
    pub max_simul: usize,
    pub total: u32,
    pub last_key: Option<u16>,
    /// Previous modifier state (for ModifiersChangedEvent diff detection).
    pub mods: Modifiers,
    /// Times of key events (press, release, modifier changes) — last 3 seconds. For the measured event rate.
    pub ev_ts: VecDeque<Instant>,
    /// Depth history of the magnetic buttons (last 5 s, (time, [L, R])).
    pub btn_hist: VecDeque<(Instant, [f32; 2])>,
    /// Maximum depth [L, R] on this test page.
    pub btn_peak: [f32; 2],
    /// Actuation count (rising edges of pressed) [L, R].
    pub btn_presses: [u32; 2],
    btn_was_pressed: [bool; 2],
}

impl Shell {
    pub fn test_key(&mut self, e: &KeyDownEvent, _down: bool, cx: &mut Context<Self>) {
        if e.is_held {
            return;
        }
        let Some(u) = keycode::usage_of(&e.keystroke.key) else {
            return;
        };
        let t = &mut self.test;
        t.down.insert(u);
        t.keys.entry(u).or_default().hit();
        t.total += 1;
        t.last_key = Some(u);
        t.max_simul = t.max_simul.max(t.down.len());
        t.ev_push();
        cx.notify();
    }

    pub fn test_key_up(&mut self, e: &KeyUpEvent, cx: &mut Context<Self>) {
        if let Some(u) = keycode::usage_of(&e.keystroke.key) {
            self.test.down.remove(&u);
            self.test.ev_push();
            cx.notify();
        }
    }

    /// Record modifier presses/releases in the test. The OS does not emit KeyDownEvent for a lone modifier,
    /// so `ModifiersChangedEvent` is used (left and right cannot be told apart, so it is recorded as the left-side usage).
    pub fn test_mods(&mut self, e: &ModifiersChangedEvent, cx: &mut Context<Self>) {
        let prev = self.test.mods;
        let now = e.modifiers;
        let pairs = [
            (prev.control, now.control, 0xE0u16),
            (prev.shift, now.shift, 0xE1),
            (prev.alt, now.alt, 0xE2),
            (prev.platform, now.platform, 0xE3),
        ];
        let t = &mut self.test;
        let mut changed = false;
        for (a, b, u) in pairs {
            if a == b {
                continue;
            }
            changed = true;
            if b {
                t.down.insert(u);
                t.keys.entry(u).or_default().hit();
                t.total += 1;
                t.last_key = Some(u);
            } else {
                t.down.remove(&u);
            }
        }
        t.mods = now;
        if changed {
            t.max_simul = t.max_simul.max(t.down.len());
            t.ev_push();
            cx.notify();
        }
    }

    pub fn page_test(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap(px(theme::GAP));
        let (is_mouse, is_kb) = self
            .cur()
            .and_then(|s| s.caps.as_ref())
            .map(|c| (c.mouse.is_some(), c.keyboard.is_some()))
            .unwrap_or((false, false));
        if is_mouse {
            col = col.child(self.mouse_test(cx));
            if let Some(c) = self.rate_card(false, cx) {
                col = col.child(c);
            }
        }
        if is_kb {
            col = col.child(self.key_test(window, cx));
            if let Some(c) = self.rate_card(true, cx) {
                col = col.child(c);
            }
        }
        col
    }

    /// Magnetic button press depth in one line (to be inserted into the Performance tab's magnetic switch card, etc.).
    /// Call only for devices that expose `live_buttons`.
    pub fn btn_depth_row(&mut self) -> Div {
        let p = theme::pal();
        let btns = self.cur().map(|s| s.buttons.clone()).unwrap_or_default();
        motion::keep_alive();
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .pt(px(10.))
            .pb(px(4.))
            .child(ui::faint("Live"))
            .children(["L", "R"].iter().enumerate().map(|(i, n)| {
                let b = btns.get(i).copied().unwrap_or_default();
                let on = b.pressed.unwrap_or(false);
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(ui::faint((*n).to_string()))
                    .child(
                        div()
                            .relative()
                            .w(px(110.))
                            .h(px(10.))
                            .rounded(px(5.))
                            .bg(p.ctrl)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(relative(b.depth.clamp(0., 1.)))
                                    .rounded(px(5.))
                                    .bg(if on {
                                        p.accent
                                    } else {
                                        theme::alpha(p.sub, 0.55)
                                    }),
                            )
                            .when_some(b.trigger, |d, t| {
                                d.child(
                                    div()
                                        .absolute()
                                        .top(px(-2.))
                                        .bottom(px(-2.))
                                        .w(px(2.))
                                        .left(relative(t.clamp(0., 1.)))
                                        .bg(p.warn),
                                )
                            }),
                    )
                    .child(
                        div()
                            .w(px(34.))
                            .text_size(px(11.))
                            .text_color(if on { p.accent } else { p.sub })
                            .child(format!("{:.0}%", b.depth * 100.)),
                    )
            }))
    }

    fn mouse_test(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        // On models with magnetic button press depth, depth measurement is turned ON automatically (OFF on leaving).
        let live_slots = self
            .cur()
            .and_then(|s| s.caps.as_ref())
            .and_then(|c| c.mouse.as_ref())
            .map(|m| m.live_buttons.clone())
            .unwrap_or_default();
        if !live_slots.is_empty()
            && self.cur().is_some_and(|s| !s.travel_on)
            && self.device_id().is_some()
        {
            let id = self.device_id().unwrap();
            self.hub.set_travel(&id, true);
        }
        // Update the depth history, peak and actuation count (called at about 30Hz).
        let now = Instant::now();
        if let Some(s) = self.cur() {
            if let (Some(&l), Some(&r)) = (s.buttons.first(), s.buttons.get(1)) {
                let t = &mut self.test;
                t.btn_hist.push_back((now, [l.depth, r.depth]));
                while t
                    .btn_hist
                    .front()
                    .is_some_and(|(t0, _)| now.duration_since(*t0).as_millis() > 5000)
                {
                    t.btn_hist.pop_front();
                }
                for (i, b) in [l, r].iter().enumerate() {
                    t.btn_peak[i] = t.btn_peak[i].max(b.depth);
                    let pr = b.pressed.unwrap_or(false);
                    if pr && !t.btn_was_pressed[i] {
                        t.btn_presses[i] += 1;
                    }
                    t.btn_was_pressed[i] = pr;
                }
            }
        }
        // Number of move events in the last 1 second.
        let now = Instant::now();
        while self
            .test
            .moves
            .front()
            .is_some_and(|t| now.duration_since(*t).as_millis() > 1000)
        {
            self.test.moves.pop_front();
        }
        let rate = self.test.moves.len();
        if rate > 0 {
            motion::keep_alive();
        }
        let names = ["Left", "Right", "Middle", "Back", "Forward"];
        let mut stats = div().flex().gap(px(10.));
        for (i, n) in names.iter().enumerate() {
            let s = self.test.buttons[i].clone();
            let flash = s.last.is_some_and(|l| l.elapsed().as_millis() < 160);
            if flash {
                motion::keep_alive();
            }
            let warn = s.chatter > 0;
            stats = stats.child(
                div()
                    .flex_1()
                    .p(px(12.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(if warn {
                        theme::alpha(p.danger, 0.5)
                    } else {
                        p.hair
                    })
                    .bg(if flash { p.accent_soft } else { p.ctrl })
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(ui::faint(n.to_string()))
                    .child(ui::big_number(format!("{}", s.count), 26.))
                    .child(ui::faint(match s.last_gap {
                        Some(g) => format!("Gap {g}ms · min {}ms", s.min_gap.unwrap_or(g)),
                        None => "—".into(),
                    }))
                    .when(warn, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(p.danger)
                                .child(format!("Possible chatter {} times", s.chatter)),
                        )
                    }),
            );
        }
        let pad = div()
            .id("test-pad")
            .relative()
            .h(px(220.))
            .rounded(px(18.))
            .border_1()
            .border_color(theme::alpha(p.accent, 0.35))
            .bg(theme::alpha(p.accent, 0.06))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .child(ui::icon("mouse-pointer-click", 26., p.accent))
            .child(ui::sub("Click, scroll the wheel and move inside this box"))
            .child(ui::faint(format!(
                "Wheel ↑{} ↓{} · move events {}/s (max {}) — the OS thins out move values, so treat them as a guide",
                self.test.wheel_up, self.test.wheel_down, rate, self.test.move_peak
            )))
            .on_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                let i = match e.button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Navigate(NavigationDirection::Back) => 3,
                    MouseButton::Navigate(NavigationDirection::Forward) => 4,
                };
                this.test.buttons[i].hit();
                cx.notify();
            }))
            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                let dy = match e.delta {
                    ScrollDelta::Lines(l) => l.y,
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                };
                if dy > 0. {
                    this.test.wheel_up += 1;
                } else if dy < 0. {
                    this.test.wheel_down += 1;
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, cx| {
                this.test.moves.push_back(Instant::now());
                let n = this.test.moves.len();
                this.test.move_peak = this.test.move_peak.max(n);
                cx.notify();
            }));
        let reset = ui::button(
            "test-reset-m",
            "Reset",
            Some("rotate-ccw"),
            Btn::Ghost,
            cx,
            |this, _, cx| {
                this.test.buttons = Default::default();
                this.test.wheel_up = 0;
                this.test.wheel_down = 0;
                this.test.moves.clear();
                this.test.move_peak = 0;
                cx.notify();
            },
        );
        let clicks = ui::card_titled("Click / wheel test", "mouse-pointer-click", Some(reset.into_any_element())).child(
            ui::body()
                .gap(px(12.))
                .pt(px(8.))
                .pb(px(16.))
                .child(pad)
                .child(stats)
                .child(ui::faint(format!(
                    "Consecutive inputs shorter than {CHATTER_MS}ms are counted as suspected chatter. If the count keeps rising, increase the debounce time on the Performance tab."
                ))),
        );
        let mut col = div().flex().flex_col().gap(px(theme::GAP)).child(clicks);
        if !live_slots.is_empty() {
            col = col.child(self.btn_depth_card(&live_slots));
        }
        col
    }

    /// Card for the measured polling rate — peak + segment rate + estimated period + line chart.
    /// With `keyboard=true`, a note "counts only while typing" is shown (the 5-second measurement is mouse only).
    fn rate_card(&self, keyboard: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = self.cur()?;
        let st = s.poll.clone()?;
        let hist = st.hist.clone();
        let min_gap = st.min_gap_ms;
        // In the background, Windows 11 restricts raw input delivery so the value drops — not reliable.
        let bg = !self.last_window_active;
        let p = theme::pal();
        // The big number is the "peak" (the max rate in the moving segment — excludes idle time).
        // The average (1 s) is a lower bound that also mixes in deceleration and stops.
        let big = st.peak_hz.or(st.seg_hz).unwrap_or(st.hz);
        // Line chart (250ms buckets × 60 = 15 s). Auto-scaled to the maximum.
        let spark = canvas(
            move |_, _, _| hist.clone(),
            move |bounds, hist, window, _| {
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let max = hist.iter().copied().fold(10f32, f32::max).max(1.);
                let mut pb = PathBuilder::stroke(px(1.5));
                let n = hist.len().max(1) as f32;
                for (i, v) in hist.iter().enumerate() {
                    let x = px(w * i as f32 / (n - 1.).max(1.));
                    let y = px(h * (1. - (v / max).clamp(0., 1.) * 0.92));
                    let pt = bounds.origin + point(x, y);
                    if i == 0 {
                        pb.move_to(pt);
                    } else {
                        pb.line_to(pt);
                    }
                }
                if let Ok(path) = pb.build() {
                    window.paint_path(path, theme::hex(0x3ec7ff, 0.9));
                }
            },
        )
        .h(px(64.))
        .w_full();

        // "Measure 5 s" — while measuring, the cursor is pinned inside the window (game style).
        // The end is handled by the tick in app.rs watching poll_meas_end.
        let measuring = self.poll_meas_end.is_some();
        let meas_btn = (!keyboard).then(|| {
            let label = if measuring { "Measuring…" } else { "Measure 5 s" };
            ui::button("poll-measure", label, Some("timer"), Btn::Ghost, cx, |this, w, cx| {
                if this.poll_meas_end.is_some() {
                    return;
                }
                this.poll_meas_end = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
                this.poll_meas_result = None;
                crate::cursor::clip_to_window(w);
                cx.notify();
            })
            .when(measuring, |d| d.opacity(0.5))
        });

        let mut col = ui::body()
            .pt(px(8.))
            .pb(px(14.))
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(10.))
                    .child(
                        ui::big_number(format!("{big:.0} Hz"), 28.)
                            .when(bg, |d| d.text_color(p.faint)),
                    )
                    .child(ui::faint(if bg {
                        "In the background, so the OS is restricting input — bring this window to the front to measure".to_string()
                    } else if big < 1. {
                        if keyboard {
                            "— press a key to measure".to_string()
                        } else {
                            "— move the mouse to measure".to_string()
                        }
                    } else if keyboard {
                        "(key events/s — not the same as the configured polling rate)".to_string()
                    } else {
                        let mut s = format!("Peak · average {:.0}", st.hz);
                        if let Some(seg) = st.seg_hz {
                            s = format!("{s} · now {seg:.0}");
                        }
                        format!("{s} (while moving)")
                    })),
            )
            .child(spark)
            .when_some(st.gap_stats, |d, (med, p95)| {
                // Median-based measurement — more robust against dropped events than counts per second.
                // If 8K is working it shows up near ~0.125ms (=8000Hz).
                let mut line = format!("Estimated interval {med:.3} ms → ~{:.0} Hz", 1000. / med);
                if let Some(g) = min_gap {
                    line = format!("{line} · min {g:.2} ms");
                }
                if p95 > med * 2. {
                    line = format!("{line} · p95 {p95:.2} ms (high variance)");
                }
                d.child(ui::faint(line))
            })
            .when(st.gap_stats.is_none(), |d| {
                d.when_some(min_gap, |d, g| {
                    d.child(ui::faint(format!(
                        "{}{:.2} ms → ~{:.0} Hz equivalent",
                        if keyboard {
                            "Min event interval "
                        } else {
                            "Min report interval "
                        },
                        g,
                        1000. / g
                    )))
                })
            });
        // Upper-limit diagnosis from motion amount — counts/s ≈ the upper limit reachable at this speed and DPI.
        // If most are ±1 count, the DPI or speed is insufficient and reports are being thinned out.
        if !keyboard {
            col = col.when_some(st.counts_per_sec, |d, c| {
                let mut line = format!("Motion {c:.0}/s → upper limit at this speed and DPI ≈ {c:.0} Hz");
                if st.low_frac.is_some_and(|f| f > 0.5) {
                    line = format!("{line} (about 1 count per report — raise DPI and move faster to push it higher)");
                }
                d.child(ui::faint(line))
            });
        }
        // Guidance while measuring / result.
        if measuring {
            let left = self
                .poll_meas_end
                .map(|e| e.saturating_duration_since(std::time::Instant::now()).as_secs_f32())
                .unwrap_or(0.);
            col = col.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.accent)
                    .child(format!("Measuring — the cursor is pinned inside the window. Draw fast, large circles in the window ({left:.1} s left)")),
            );
        } else if let Some((peak, med, cnt)) = self.poll_meas_result {
            let mut line = format!("Measurement result: peak {peak:.0} Hz");
            if med > 0. {
                line = format!("{line} · median interval {med:.3} ms (~{:.0} Hz)", 1000. / med);
            }
            if cnt > 0. {
                line = format!("{line} · motion {cnt:.0}/s");
            }
            col = col.child(div().text_size(px(12.)).text_color(p.accent).child(line));
        }
        col = col.child(ui::faint(if keyboard {
            "Measured from keyboard input reports that reached the OS. A report arrives each time the key state changes, so only typing is counted (polling while idle never reaches the OS)."
        } else {
            "Measured from input reports that reached the OS (Raw Input). A mouse only sends reports for movement, so \"Peak\" is the value closest to the actual polling rate. For an accurate reading, set DPI to 3200 or higher and draw fast, large circles."
        }));
        Some(
            ui::card_titled("Polling rate measurement", "gauge", meas_btn.map(|b| b.into_any_element()))
                .child(col)
                .into_any_element(),
        )
    }

    /// The "Button press depth" card — left/right vertical bars + stats + a line chart of the last 5 s.
    fn btn_depth_card(&mut self, live_slots: &[usize]) -> AnyElement {
        let p = theme::pal();
        let names: Vec<String> = live_slots
            .iter()
            .map(|&i| {
                self.cur()
                    .and_then(|s| s.caps.as_ref())
                    .and_then(|c| c.mouse.as_ref())
                    .and_then(|m| m.buttons.get(i))
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| format!("Button {i}"))
            })
            .collect();
        let btns = self.cur().map(|s| s.buttons.clone()).unwrap_or_default();
        // Vertical bar (0-100%). Accent color while actuated, gray when not. A line at the actuation point.
        let bar = |i: usize, name: &str| {
            let b = btns.get(i).copied().unwrap_or_default();
            let on = b.pressed.unwrap_or(false);
            let h = 120.;
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .child(ui::faint(name.to_string()))
                .child(
                    div()
                        .relative()
                        .w(px(30.))
                        .h(px(h))
                        .rounded(px(9.))
                        .bg(p.ctrl)
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .h(relative(b.depth.clamp(0., 1.)))
                                .rounded_b(px(9.))
                                .bg(if on {
                                    p.accent
                                } else {
                                    theme::alpha(p.sub, 0.55)
                                }),
                        )
                        .when_some(b.trigger, |d, t| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .h(px(2.))
                                    .bottom(px(t.clamp(0., 1.) * h - 1.))
                                    .bg(p.warn),
                            )
                        }),
                )
                .child(ui::big_number(format!("{:.0}", b.depth * 100.), 20.))
                .child(ui::faint(format!(
                    "Max {:.0}% · actuations {}",
                    self.test.btn_peak.get(i).copied().unwrap_or(0.) * 100.,
                    self.test.btn_presses.get(i).copied().unwrap_or(0)
                )))
        };
        // Line chart of the last 5 s (two colors, left/right).
        let hist: Vec<(Instant, [f32; 2])> = self.test.btn_hist.iter().cloned().collect();
        let spark = canvas(
            move |_, _, _| hist.clone(),
            move |bounds, hist, window, _| {
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let t_end = Instant::now();
                for (s, color) in [
                    (0usize, theme::hex(0x3ec7ff, 0.9)),
                    (1, theme::hex(0xff7a3d, 0.9)),
                ] {
                    let mut pb = PathBuilder::stroke(px(1.5));
                    let mut first = true;
                    for (t, v) in &hist {
                        let age = t_end.duration_since(*t).as_secs_f32().min(5.);
                        let x = px(w * (1. - age / 5.));
                        let y = px(h * (1. - v[s].clamp(0., 1.)));
                        let pt = bounds.origin + point(x, y);
                        if first {
                            pb.move_to(pt);
                            first = false;
                        } else {
                            pb.line_to(pt);
                        }
                    }
                    if let Ok(path) = pb.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .h(px(80.))
        .w_full();
        ui::card_titled("Button press depth", "activity", None)
            .child(
                ui::body().gap(px(12.)).pt(px(10.)).pb(px(16.)).child(
                    div()
                        .flex()
                        .gap(px(24.))
                        .child(
                            div()
                                .flex()
                                .gap(px(28.))
                                .children(names.iter().enumerate().map(|(i, n)| bar(i, n))),
                        )
                        .child(
                            div().flex_1().child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(ui::faint("Last 5 s"))
                                    .child(spark)
                                    .child(ui::faint("Lines: L=light blue R=orange · orange horizontal line = actuation point")),
                            ),
                        ),
                ),
            )
            .into_any_element()
    }

    fn key_test(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let Some((kc, ks)) = kb_parts(self) else {
            return div().into_any_element();
        };
        // If live_travel is supported, depth measurement is turned ON automatically (turned OFF on tab / device change).
        let can_live = kc.perf.as_ref().is_some_and(|pc| pc.live_travel);
        if can_live && self.cur().is_some_and(|s| !s.travel_on) && self.device_id().is_some() {
            self.hub.set_travel(&self.device_id().unwrap(), true);
        }
        let live = self
            .cur()
            .map(|s| (s.travel.clone(), s.axis.clone(), s.travel_max))
            .unwrap_or_default();
        // Normalize by the measured maximum depth (the real bottom is shallower than the nominal 4.0mm).
        let live_max = live.2.max(0.5);
        // usage → layout index (looked up from the main layer's assignments).
        let index_of = |u: u16| -> Option<usize> {
            ks.keymap
                .first()
                .and_then(|l| l.iter().position(|a| *a == KeyAction::Code(u as u32)))
        };
        let caps: Vec<Cap> = kc
            .layout
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let usage = ks
                    .keymap
                    .first()
                    .and_then(|l| l.get(i))
                    .and_then(|a| match a {
                        KeyAction::Code(c) if *c < 0x100 => Some(*c as u16),
                        _ => None,
                    });
                let down = usage.is_some_and(|u| self.test.down.contains(&u))
                    || live.1.get(i).is_some_and(|a| a.status != 0);
                let st = usage.and_then(|u| self.test.keys.get(&u));
                let tint = if down {
                    Some(theme::alpha(p.accent, 0.75))
                } else if st.is_some_and(|s| s.chatter > 0) {
                    Some(theme::alpha(p.danger, 0.45))
                } else if st.is_some() {
                    Some(theme::alpha(p.ok, 0.25))
                } else {
                    None
                };
                Cap {
                    top: k.label.clone(),
                    sub: st.map(|s| format!("{}", s.count)),
                    tint,
                    depth: live.0.get(i).map(|d| d / live_max),
                    ..Default::default()
                }
            })
            .collect();
        let view = self.keyboard_view("keytest", &kc, caps, cx, Rc::new(|_, _, _, _| {}));
        let chatter: u32 = self.test.keys.values().map(|s| s.chatter).sum();
        let unknown: Vec<String> = self
            .test
            .keys
            .keys()
            .filter(|u| index_of(**u).is_none())
            .map(|u| devhub_core::keys::hid_name(*u))
            .collect();
        let focused = self.focus.is_focused(window);
        let reset = ui::button(
            "test-reset-k",
            "Reset",
            Some("rotate-ccw"),
            Btn::Ghost,
            cx,
            |this, w, cx| {
                this.test.down.clear();
                this.test.keys.clear();
                this.test.max_simul = 0;
                this.test.total = 0;
                this.test.last_key = None;
                w.focus(&this.focus, cx);
                cx.notify();
            },
        );
        let stat = |label: &str, v: String| {
            div()
                .flex_1()
                .p(px(12.))
                .rounded(px(14.))
                .bg(p.ctrl)
                .border_1()
                .border_color(p.hair)
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(ui::faint(label.to_string()))
                .child(ui::big_number(v, 24.))
        };
        ui::card_titled("Key input test", "keyboard", Some(reset.into_any_element())).child(
            ui::body()
                .gap(px(12.))
                .pt(px(8.))
                .pb(px(16.))
                .when(!focused, |d| {
                    d.child(
                        div()
                            .id("keytest-focus")
                            .p(px(10.))
                            .rounded(px(12.))
                            .cursor_pointer()
                            .bg(theme::alpha(p.warn, 0.12))
                            .border_1()
                            .border_color(theme::alpha(p.warn, 0.35))
                            .child(ui::sub("Click here, then press keys (cannot measure while an input field has focus)"))
                            .on_click(cx.listener(|this, _, w, cx| {
                                w.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    )
                })
                .child(view)
                .child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .child(stat("Total keystrokes", format!("{}", self.test.total)))
                        .child(stat("Simultaneous (max)", format!("{}", self.test.max_simul)))
                        .child(stat("Keys pressed now", format!("{}", self.test.down.len())))
                        .child(stat("Suspected chatter", format!("{chatter}"))),
                )
                .child({
                    let (ev_s, _) = self.test.ev_rate();
                    if !self.test.ev_ts.is_empty() {
                        motion::keep_alive();
                    }
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(ui::faint(format!("OS events {ev_s}/s")))
                })
                // Raw data of pressed keys (live_axis: depth mm / ADC / state).
                .when(!live.1.is_empty(), |d| {
                    let rows: Vec<String> = self
                        .test
                        .down
                        .iter()
                        .filter_map(|u| {
                            let i = index_of(*u)?;
                            let label = kc.layout.get(i)?.label.clone();
                            let a = live.1.get(i)?;
                            Some(format!(
                                "{label}: {:.2}mm · ADC {} · status {} · cal {}",
                                a.route, a.adc, a.status, a.calibrated
                            ))
                        })
                        .collect();
                    let last = self
                        .test
                        .last_key
                        .and_then(|u| index_of(u).map(|i| (u, i)))
                        .and_then(|(_, i)| {
                            let a = live.1.get(i)?;
                            Some(format!(
                                "Last: {} {:.2}mm · ADC {} · status {}",
                                kc.layout.get(i)?.label, a.route, a.adc, a.status
                            ))
                        });
                    d.child(
                        div()
                            .p(px(10.))
                            .rounded(px(12.))
                            .bg(p.ctrl)
                            .border_1()
                            .border_color(p.hair)
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(ui::faint(format!(
                                "Raw data (pressed keys: {}) · observed max {:.2}mm:",
                                if rows.is_empty() { "none" } else { "present" },
                                live.2
                            )))
                            .children(rows.into_iter().map(ui::sub))
                            .when_some(last, |d, s| d.child(ui::faint(s))),
                    )
                })
                .child(ui::faint(
                    "Pressed keys show green, keys being held show blue, and double inputs under 25ms show red. Left and right modifier keys cannot be told apart (recorded as the left side). Keys that Windows intercepts (Win alone, Alt+Tab, etc.) and Fn never reach the OS, so they cannot be measured.",
                ))
                .when(!unknown.is_empty(), |d| d.child(ui::faint(format!("Keys not in the diagram: {}", unknown.join(" "))))),
        )
        .into_any_element()
    }
}
