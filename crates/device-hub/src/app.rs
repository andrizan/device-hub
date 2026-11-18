//! App shell (Shell).
//!
//! ```text
//!  ┌──────────────┬─────────────────────────────────────────┐
//!  │ ┌──────────┐ │  Device name            [Profile ▾] ●Sync │
//!  │ │ sidebar  │ │  ( DPI | Buttons | Macros | General )      │  ← floating Clear glass
//!  │ │  (glass) │ │  ┌ card ─────────┐ ┌ card ────────┐       │  ← Panel glass
//!  │ └──────────┘ │  └───────────────┘ └──────────────┘       │
//!  └──────────────┴─────────────────────────────────────────┘
//! ```
//!
//! All state lives in this single entity (so GPUI listeners can touch it directly).
//! Each screen is split into `pages/*.rs` as an `impl Shell`.

use crate::prefs::Prefs;
use crate::store::{Hub, Msg};
use crate::{glass, motion, theme, ui, wallpaper};
use devhub_core::*;
use futures::StreamExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

// ====================================================================== Root

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Device(DeviceId),
    Settings,
    /// Editor for the OBS overlay's appearance and displayed keys.
    Obs,
    /// Profiles + process-linked automation.
    Auto,
}

/// Tabs of the device screen (shown or hidden according to caps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Dpi,
    Buttons,
    Performance,
    Keymap,
    Trigger,
    Advanced,
    Lighting,
    Live,
    Macros,
    Audio,
    Test,
    System,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Dpi => "DPI",
            Tab::Buttons => "Buttons",
            Tab::Performance => "Performance",
            Tab::Keymap => "Key mapping",
            Tab::Trigger => "Triggers",
            Tab::Advanced => "Advanced keys",
            Tab::Lighting => "Lighting",
            Tab::Live => "Live",
            Tab::Macros => "Macros",
            Tab::Audio => "Audio",
            Tab::Test => "Test",
            Tab::System => "General",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Tab::Dpi => "crosshair",
            Tab::Buttons => "mouse-pointer-click",
            Tab::Performance => "gauge",
            Tab::Keymap => "keyboard",
            Tab::Trigger => "arrow-down-to-line",
            Tab::Advanced => "sparkles",
            Tab::Lighting => "lightbulb",
            Tab::Live => "zap",
            Tab::Macros => "list-video",
            Tab::Audio => "volume-2",
            Tab::Test => "flask-conical",
            Tab::System => "settings-2",
        }
    }
}

/// Which tab a generic settings section appears in.
pub fn tab_of_section(section: &str, keyboard: bool) -> Tab {
    use devhub_core::section::*;
    match section {
        PERFORMANCE | SENSOR | CLICK | MAGNETIC | POWER if !keyboard => Tab::Performance,
        LIGHTING => Tab::Lighting,
        AUDIO => Tab::Audio,
        _ => Tab::System,
    }
}

pub fn tabs_for(caps: &Capabilities) -> Vec<Tab> {
    let mut v = vec![];
    let kb = caps.keyboard.is_some();
    if let Some(m) = &caps.mouse {
        v.push(Tab::Dpi);
        if !m.buttons.is_empty() {
            v.push(Tab::Buttons);
        }
        if caps
            .settings
            .iter()
            .any(|s| tab_of_section(&s.section, false) == Tab::Performance)
            || caps
                .actions
                .iter()
                .any(|a| tab_of_section(&a.section, false) == Tab::Performance)
        {
            v.push(Tab::Performance);
        }
    }
    if let Some(k) = &caps.keyboard {
        v.push(Tab::Keymap);
        if k.perf.is_some() {
            v.push(Tab::Trigger);
        }
        if !k.advanced.is_empty() {
            v.push(Tab::Advanced);
        }
    }
    if !caps.lighting.is_empty()
        || caps
            .settings
            .iter()
            .any(|s| tab_of_section(&s.section, kb) == Tab::Lighting)
    {
        v.push(Tab::Lighting);
    }
    // Live lighting: only models that have DirectDrive + a live press-depth readout.
    if caps.lighting.iter().any(|a| a.direct_drive)
        && caps
            .keyboard
            .as_ref()
            .is_some_and(|k| k.perf.as_ref().is_some_and(|p| p.live_travel))
    {
        v.push(Tab::Live);
    }
    let macros = caps
        .mouse
        .as_ref()
        .and_then(|m| m.macros.as_ref())
        .is_some()
        || caps
            .keyboard
            .as_ref()
            .and_then(|k| k.macros.as_ref())
            .is_some();
    if macros {
        v.push(Tab::Macros);
    }
    if caps.peq.is_some()
        || caps
            .settings
            .iter()
            .any(|s| tab_of_section(&s.section, kb) == Tab::Audio)
        || caps
            .actions
            .iter()
            .any(|a| tab_of_section(&a.section, kb) == Tab::Audio)
    {
        v.push(Tab::Audio);
    }
    if caps.mouse.is_some() || caps.keyboard.is_some() {
        v.push(Tab::Test);
    }
    v.push(Tab::System);
    v
}

// ====================================================================== Overlays

pub type Cb = Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;
pub type CbIdx = Rc<dyn Fn(&mut Shell, usize, &mut Window, &mut Context<Shell>)>;
pub type CbF = Rc<dyn Fn(&mut Shell, f32, &mut Context<Shell>)>;
/// 2D drag (x, y are 0..1 within the track rect; y is 1 at the top).
pub type CbXY = Rc<dyn Fn(&mut Shell, f32, f32, &mut Context<Shell>)>;

/// State while dragging a slider etc.
pub struct Drag {
    /// motion::rect key (the track's rect).
    pub rect: String,
    /// Called with a 0..1 position.
    pub apply: CbF,
    /// Vertical drag (travel diagram).
    pub vertical: bool,
    /// 2D drag (EQ graph). If Some, this is called instead of `apply`.
    pub xy: Option<CbXY>,
}

#[derive(Clone)]
pub struct PopItem {
    pub label: String,
    pub sub: Option<String>,
    pub selected: bool,
    pub swatch: Option<Hsla>,
    pub icon: Option<&'static str>,
    pub danger: bool,
}

impl PopItem {
    pub fn new(label: impl Into<String>, selected: bool) -> Self {
        Self {
            label: label.into(),
            sub: None,
            selected,
            swatch: None,
            icon: None,
            danger: false,
        }
    }
}

pub struct Popup {
    pub anchor: [f32; 4],
    pub items: Vec<PopItem>,
    pub on_pick: CbIdx,
    pub min_w: f32,
    pub born: Instant,
}

pub struct Toast {
    pub msg: String,
    pub error: bool,
    pub born: Instant,
}

/// Modal. Its contents are drawn by `overlay.rs`.
pub enum Modal {
    Confirm {
        title: String,
        body: String,
        ok: String,
        danger: bool,
        on_ok: Cb,
    },
    /// Mouse button assignment editor.
    Button {
        slot: usize,
        draft: ButtonAction,
        capture: bool,
    },
    /// Capture of a single keystroke (shortcut). The result goes to `on_key`.
    CaptureKey {
        title: String,
        got: Option<KeyStroke>,
        on_key: Rc<dyn Fn(&mut Shell, KeyStroke, &mut Window, &mut Context<Shell>)>,
    },
    /// Color picker.
    Color {
        title: String,
        draft: Rgb,
        on_ok: Rc<dyn Fn(&mut Shell, Rgb, &mut Window, &mut Context<Shell>)>,
    },
    /// Command palette (Ctrl+K). The input field is `inputs["palette"]`.
    Palette,
    /// Calibration wizard. `maxes` is the maximum press depth per key (mm).
    Calibrate { started: bool, maxes: Vec<f32> },
    /// Text input (rename etc.). The input field is `inputs["modal"]`.
    Text {
        title: String,
        on_ok: Rc<dyn Fn(&mut Shell, String, &mut Window, &mut Context<Shell>)>,
    },
}

/// Macro recording in progress.
pub struct Recording {
    pub started: Instant,
    pub last: Instant,
    /// 0 = record real time, 1 = fixed delay, 2 = no delay.
    pub delay_mode: u8,
    pub fixed_ms: u32,
    pub mods: Modifiers,
}

// ====================================================================== Shell

pub struct Shell {
    pub hub: Hub,
    pub prefs: Prefs,
    pub route: Route,
    /// Device to restore at startup. Consumed by the first successful enumeration or a user action on screen.
    startup_device: Option<DeviceId>,
    pub tab: Tab,
    pub focus: FocusHandle,
    pub scroll: ScrollHandle,

    pub drag: Option<Drag>,
    pub popup: Option<Popup>,
    pub modal: Option<Modal>,
    pub toasts: Vec<Toast>,
    pub inputs: HashMap<String, Entity<InputState>>,
    /// Commit target of `num:*` inputs (min, max, step, apply). Re-registered on every render.
    pub num_fields: HashMap<
        String,
        (
            f32,
            f32,
            f32,
            Rc<dyn Fn(&mut Shell, f32, &mut Context<Shell>)>,
        ),
    >,
    /// Display string last written to a numeric input (not overwritten while focused).
    num_sync: HashMap<String, String>,

    // ---- Per-screen UI state
    /// DPI: stage being edited.
    pub dpi_sel: usize,
    /// DPI: whether the user picked a stage. If not, it follows the stage
    /// currently in use (active_stage).
    pub dpi_sel_touch: bool,
    /// Audio: EQ band being edited.
    pub peq_sel: usize,
    /// Live lighting engine state.
    pub fx: crate::livefx::Engine,
    /// Device running live lighting (None = stopped). Keeps running after leaving the tab or device.
    pub fx_on: Option<DeviceId>,
    /// Time of the previous frame (for dt).
    pub fx_last: Instant,
    /// Whether auto-start at launch was tried.
    pub fx_autostarted: bool,
    /// Keyboard: layer being shown.
    pub kb_layer: u8,
    /// Keyboard: selected key (layout index).
    pub kb_sel: BTreeSet<usize>,
    /// Candidate group for key assignment.
    pub key_group: usize,
    /// Modifiers for key combinations (key mapping).
    pub combo_mods: u8,
    /// Draft of the advanced key being edited.
    pub adv: crate::pages::advanced::AdvDraft,
    /// Lighting: zone and fill color.
    pub light_area: u8,
    pub paint: Rgb,
    /// Lighting link (official "link"): when ON, the top/bottom zone switches move together.
    pub light_link: bool,
    /// Animation phase of the lighting preview (seconds). Advanced by the ticker.
    pub anim_t: f32,
    /// OBS overlay settings (`%APPDATA%\DeviceHub\obs_overlay.json`).
    pub obs_cfg: devhub_obs::config::OverlayCfg,
    /// Source type to preview.
    pub obs_mode: devhub_obs::render::Mode,
    /// Preview image (replaced every frame; old ones are freed with drop_image).
    pub obs_img: Option<std::sync::Arc<gpui::RenderImage>>,
    /// Preview animation time / Japanese-text font.
    pub obs_t: f32,
    pub obs_font: Option<std::rc::Rc<Option<fontdue::Font>>>,
    /// Whether the Lighting page is showing an animated effect (set by render every time).
    pub light_anim: bool,
    /// Time the last DirectDrive frame was sent for "Preview on device" (for throttling).
    pub dd_last: Instant,
    /// Macros: selected slot and draft.
    pub macro_sel: usize,
    pub macro_draft: Option<MacroDef>,
    pub macro_event_sel: Option<usize>,
    pub recording: Option<Recording>,
    /// Undo / redo (device, snapshot from before the change).
    pub undo: Vec<(DeviceId, DeviceState)>,
    pub redo: Vec<(DeviceId, DeviceState)>,
    last_snap: Option<(DeviceId, Instant)>,
    /// Writing due to undo/redo/preset apply (not pushed onto the history).
    restoring: bool,
    /// State to apply once the config switch + re-read finishes (undo/redo to another config, etc.).
    pending_restore: Option<PendingRestore>,
    /// Test tab state.
    pub test: crate::pages::test::TestState,
    /// Profile being recorded (index into prefs.profiles). `apply` appends Changes.
    pub rec_profile: Option<usize>,
    /// Active process rules (rule_key → record used to restore on leaving).
    pub proc_states: std::collections::HashMap<String, crate::procmon::ProcApplied>,
    /// Foreground exe at the previous tick (Some = resolved). A transition is evaluated only when it is the same for 2 consecutive ticks.
    pub proc_fg: Option<String>,
    /// Changes held until the session becomes writable (device id → remaining Changes).
    pub proc_pending: Vec<(String, Vec<Change>)>,
    /// Time of the last reload attempt for a Failed session (to throttle drain retries).
    pub proc_retry: std::collections::HashMap<String, std::time::Instant>,
    /// Form selection state of the automation page.
    pub auto_form: crate::pages::auto::AutoForm,

    toast_timer: bool,
    /// HWND of our own window — used to return the working set to the OS while minimized/in the background.
    #[cfg(windows)]
    hwnd: isize,
    /// Whether memory priority is lowered (set while in the background or minimized).
    #[cfg(windows)]
    mem_low: bool,
    /// Ticks since going to the background (for the re-trim interval).
    #[cfg(windows)]
    mem_ticks: u32,
    /// Whether the window is in the foreground (updated by render every frame).
    /// In the background Windows 11 throttles raw input delivery, so the measured polling
    /// display uses this flag to decide whether the value can be trusted.
    pub(crate) last_window_active: bool,
    /// End time of the 5-second polling measurement (Some = measuring — cursor pinned).
    pub(crate) poll_meas_end: Option<std::time::Instant>,
    /// Latest 5-second measurement result (peak Hz, median interval ms, counts/s).
    pub(crate) poll_meas_result: Option<(f32, f32, f32)>,
}

/// State to apply after the config switch completes (undo/redo to another config, etc.).
/// If the switch or re-read fails, `rollback_*` is put back into the history.
struct PendingRestore {
    id: DeviceId,
    state: DeviceState,
    rollback_undo: Option<DeviceState>,
    rollback_redo: Option<DeviceState>,
}

/// Result of `apply_state`.
pub enum RestoreOutcome {
    /// Wrote the diff (count).
    Applied(usize),
    /// Sent the config switch first — applied after the switch + re-read finishes.
    Switching,
    /// Could not apply (no session/state/caps, or a switch in progress).
    Nothing,
}

/// Where to restore history to when a restore to another config fails.
enum HistSrc {
    None,
    Undo,
    Redo,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = Prefs::load();
        let startup_device = prefs.last_device.clone().map(DeviceId);
        theme::set_light(prefs.light());
        theme::set_accent(prefs.accent);
        theme::set_font_id(&prefs.font);
        theme::set_glass(prefs.glass && std::env::var_os("DEVHUB_FAST").is_none());
        theme::set_motion(prefs.motion);

        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Msg>();
        let (backend, _) = crate::store::connect_backend();
        let mut hub = Hub::new(backend, tx);
        hub.fw_enabled = prefs.fw_check;
        hub.fw_native = prefs.fw_native_notify;
        hub.scan();

        // Receive results from the worker (wakes only when one arrives).
        cx.spawn(async move |this: WeakEntity<Shell>, cx: &mut AsyncApp| {
            while let Some(msg) = rx.next().await {
                let alive = this
                    .update(cx, |this, cx| {
                        let scan_succeeded =
                            matches!(&msg, crate::store::Msg::Scanned { res: Ok(_), .. });
                        if let Some((m, e)) = this.hub.handle(msg) {
                            this.toast(m, e, cx);
                        }
                        this.after_msg(scan_succeeded, cx);
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        // Periodic: enumeration (2 s) and battery (30 s).
        cx.spawn(async move |this: WeakEntity<Shell>, cx: &mut AsyncApp| {
            let mut ticks = 0u32;
            loop {
                cx.background_executor()
                    .timer(crate::store::SCAN_EVERY)
                    .await;
                ticks += 1;
                let alive = this
                    .update(cx, |this, _cx| {
                        this.hub.scan();
                        if ticks.is_multiple_of(15) {
                            this.hub.poll_battery();
                        }
                        this.mem_tick();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        // Light ticker for the lighting preview (~30fps).
        // Advances the phase and redraws only while `light_anim`, set by render, is true.
        // The on-device preview (DirectDrive) also sends frames from this phase, so
        // the rate directly affects smoothness on the device (probe9: no congestion up to 87fps).
        // Also samples the measured report rate every ~400ms (only while Performance/Test is shown).
        cx.spawn(async move |this: WeakEntity<Shell>, cx: &mut AsyncApp| {
            let mut n = 0u32;
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(33))
                    .await;
                n += 1;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.light_anim && this.tab == Tab::Lighting {
                            this.anim_t += 0.033;
                            cx.notify();
                        }
                        // OBS overlay preview (~15fps).
                        if this.route == Route::Obs && n.is_multiple_of(2) {
                            this.obs_t += 0.066;
                            cx.notify();
                        }
                        // Live lighting (keeps running after leaving the tab/device and while minimized).
                        this.fx_tick(cx);
                        // Poll the lit color only while the Lighting screen is shown
                        // (as in the official HUB, shows the real device's [5,3] readback as is).
                        let lit_id = if this.tab == Tab::Lighting {
                            this.device_id()
                        } else {
                            None
                        };
                        this.hub.live_light(lit_id.as_ref());
                        // Stop the on-device preview when moving to a tab other than Lighting
                        // (the last picture stays even if the frame supply stops, so stop it explicitly).
                        // Devices running live lighting are excluded.
                        if this.tab != Tab::Lighting {
                            if let Some(id) = this
                                .device_id()
                                .filter(|id| this.fx_on.as_ref() != Some(id))
                            {
                                if this.hub.session(&id).is_some_and(|s| s.dd_on) {
                                    this.hub.set_dd(&id, false);
                                    cx.notify();
                                }
                            }
                        }
                        // The measured polling rate is taken continuously for the open device (~400ms).
                        // INPUTSINK delivers it in the background too, but Windows 11 throttles
                        // raw input delivery to background processes — whether the value can be trusted is
                        // decided on the display side via last_window_active (measurement itself continues).
                        if n.is_multiple_of(12) && matches!(this.route, Route::Device(_)) {
                            if let Some(id) = this.device_id() {
                                this.hub.poll_rate(&id);
                            }
                        }
                        // End of the 5-second measurement — finalize the result and release the cursor lock.
                        if this
                            .poll_meas_end
                            .is_some_and(|end| std::time::Instant::now() >= end)
                        {
                            this.poll_meas_end = None;
                            crate::cursor::unclip();
                            if let Some(id) = this.device_id() {
                                if let Some(st) =
                                    this.hub.session(&id).and_then(|s| s.poll.clone())
                                {
                                    let peak = st
                                        .segs
                                        .iter()
                                        .filter(|s| s.age_s <= 5.5)
                                        .map(|s| s.rate_hz)
                                        .chain(st.seg_hz)
                                        .fold(0f32, f32::max);
                                    let med = st.gap_stats.map(|(m, _)| m).unwrap_or(0.);
                                    let cnt = st.counts_per_sec.unwrap_or(0.);
                                    this.poll_meas_result = Some((peak, med, cnt));
                                }
                            }
                            cx.notify();
                        }
                        // Process monitoring (~1s): apply a profile on foreground transitions.
                        if n.is_multiple_of(30) {
                            this.proc_tick(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let route = Route::Home;
        Self {
            hub,
            prefs,
            route,
            startup_device,
            tab: Tab::Dpi,
            focus,
            scroll: ScrollHandle::new(),
            drag: None,
            popup: None,
            modal: None,
            toasts: vec![],
            inputs: HashMap::new(),
            num_fields: HashMap::new(),
            num_sync: HashMap::new(),
            dpi_sel: 0,
            dpi_sel_touch: false,
            peq_sel: 0,
            fx: Default::default(),
            fx_on: None,
            fx_last: Instant::now(),
            fx_autostarted: false,
            kb_layer: 0,
            kb_sel: BTreeSet::new(),
            key_group: 0,
            combo_mods: 0,
            adv: Default::default(),
            light_area: 0,
            paint: Rgb::hex(0x4cb4ff),
            light_link: false,
            anim_t: 0.0,
            obs_cfg: devhub_obs::config::load(),
            obs_mode: devhub_obs::render::Mode::Kb,
            obs_img: None,
            obs_t: 0.0,
            obs_font: None,
            light_anim: false,
            dd_last: Instant::now() - Duration::from_secs(1),
            macro_sel: 0,
            macro_draft: None,
            macro_event_sel: None,
            recording: None,
            undo: vec![],
            redo: vec![],
            last_snap: None,
            restoring: false,
            pending_restore: None,
            test: Default::default(),
            rec_profile: None,
            proc_states: Default::default(),
            proc_fg: None,
            proc_pending: vec![],
            proc_retry: Default::default(),
            auto_form: Default::default(),
            toast_timer: false,
            #[cfg(windows)]
            hwnd: window_hwnd(window),
            #[cfg(windows)]
            mem_low: false,
            #[cfg(windows)]
            mem_ticks: 0,
            last_window_active: true,
            poll_meas_end: None,
            poll_meas_result: None,
        }
    }

    /// After an enumeration result arrives: automatically enter the last opened device / go Home when it is unplugged.
    fn after_msg(&mut self, scan_succeeded: bool, cx: &mut Context<Self>) {
        if let Route::Device(id) = &self.route {
            if !self.hub.devices.iter().any(|d| d.id == *id) && self.hub.scanned {
                self.route = Route::Home;
            }
        }
        if scan_succeeded {
            if let Some(id) = self.startup_device.take() {
                if self.route == Route::Home
                    && self.hub.devices.iter().any(|d| d.id == id && d.supported)
                {
                    self.open_device(id);
                }
            }
        }
        // Look at the config switch result and handle the restore waiting for it.
        if let Some(pr) = &self.pending_restore {
            let (failed, ready) = match self.hub.session(&pr.id) {
                Some(s) if s.switch_failed => (true, false),
                Some(s) if !s.switching_profile && s.state.is_some() => (false, true),
                _ => (false, false), // still in the middle of the switch + re-read — re-evaluated on the next message.
            };
            if failed || ready {
                let pr = self.pending_restore.take().unwrap();
                if failed {
                    // Return to the history without writing anything.
                    if let Some(st) = pr.rollback_undo {
                        self.undo.push((pr.id.clone(), st));
                    }
                    if let Some(st) = pr.rollback_redo {
                        self.redo.push((pr.id.clone(), st));
                    }
                    if let Some(s) = self.hub.session_mut(&pr.id) {
                        s.switch_failed = false;
                    }
                } else {
                    // Write only the diff against the new config's state.
                    self.apply_state_to(&pr.id, &pr.state, false, HistSrc::None, cx);
                }
            }
        }
    }

    // ------------------------------------------------------------------ Common operations

    pub fn toast(&mut self, msg: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.toasts.push(Toast {
            msg: msg.into(),
            error,
            born: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
        if !self.toast_timer {
            self.toast_timer = true;
            cx.spawn(
                async move |this: WeakEntity<Shell>, cx: &mut AsyncApp| loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    let more = this
                        .update(cx, |this, cx| {
                            let before = this.toasts.len();
                            this.toasts
                                .retain(|t| t.born.elapsed() < Duration::from_millis(4200));
                            if this.toasts.len() != before {
                                cx.notify();
                            }
                            if this.toasts.is_empty() {
                                this.toast_timer = false;
                            }
                            !this.toasts.is_empty()
                        })
                        .unwrap_or(false);
                    if !more {
                        break;
                    }
                },
            )
            .detach();
        }
        cx.notify();
    }

    fn stop_device_preview(&mut self) {
        if let Some(id) = self.device_id() {
            // A device running live lighting stops neither measurement nor DirectDrive.
            if self.fx_on.as_ref() == Some(&id) {
                return;
            }
            self.hub.set_travel(&id, false);
            self.hub.set_dd(&id, false);
        }
    }

    pub fn go(&mut self, r: Route, cx: &mut Context<Self>) {
        self.startup_device = None;
        if self.route != r {
            self.popup = None;
            self.recording = None;
            self.stop_device_preview();
            self.route = r;
            self.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        }
    }

    pub fn open_device(&mut self, id: DeviceId) {
        self.startup_device = None;
        self.stop_device_preview();
        self.hub.open(&id, true);
        self.prefs.last_device = Some(id.0.clone());
        self.prefs.save();
        self.dpi_sel = 0;
        self.dpi_sel_touch = false;
        self.peq_sel = 0;
        self.kb_layer = 0;
        self.kb_sel.clear();
        self.macro_sel = 0;
        self.macro_draft = None;
        self.light_area = 0;
        self.adv = Default::default();
        // First tab.
        self.tab = self
            .hub
            .session(&id)
            .and_then(|s| s.caps.as_ref())
            .map(|c| tabs_for(c)[0])
            .unwrap_or(Tab::Dpi);
        self.route = Route::Device(id);
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    pub fn device_id(&self) -> Option<DeviceId> {
        match &self.route {
            Route::Device(id) => Some(id.clone()),
            _ => None,
        }
    }

    /// Send a change to the open device.
    pub fn apply(&mut self, change: Change, cx: &mut Context<Self>) {
        if let Some(id) = self.device_id() {
            // Block edits within a config during a config switch + full re-read
            // (so values from the pre-switch config are not written to the post-switch config).
            if self.hub.session(&id).is_some_and(|s| s.switching_profile) {
                self.toast("Switching configuration…", false, cx);
                return;
            }
            if !self.restoring {
                self.snapshot(&id);
            }
            // While recording a profile: append changes to the target device as a Change list.
            // Consecutive changes to the same target as the previous one (intermediate slider-drag values) keep only the latest.
            if let Some(pi) = self.rec_profile {
                if let Some(p) = self.prefs.profiles.get_mut(pi) {
                    if let Some(t) = p.devices.iter_mut().find(|t| t.device == id.0) {
                        match t.changes.last_mut() {
                            Some(last) if last.same_target(&change) => *last = change.clone(),
                            _ => t.changes.push(change.clone()),
                        }
                        self.prefs.save();
                    }
                }
            }
            self.hub.apply(&id, change);
            cx.notify();
        }
    }

    /// Push the pre-change state onto the history. Consecutive operations within 0.8 s (slider drags etc.) are merged into one.
    fn snapshot(&mut self, id: &DeviceId) {
        let fresh = match &self.last_snap {
            Some((d, t)) => d != id || t.elapsed() > Duration::from_millis(800),
            None => true,
        };
        if fresh {
            if let Some(st) = self.hub.session(id).and_then(|s| s.state.clone()) {
                self.undo.push((id.clone(), st));
                if self.undo.len() > 100 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
        }
        self.last_snap = Some((id.clone(), Instant::now()));
    }

    /// Bring the whole state to `target` (writing only the diff).
    /// If the config differs, send the switch first and apply after the re-read finishes (`pending_restore`).
    pub fn apply_state(
        &mut self,
        target: &DeviceState,
        record: bool,
        cx: &mut Context<Self>,
    ) -> RestoreOutcome {
        let Some(id) = self.device_id() else {
            return RestoreOutcome::Nothing;
        };
        self.apply_state_to(&id, target, record, HistSrc::None, cx)
    }

    fn apply_state_to(
        &mut self,
        id: &DeviceId,
        target: &DeviceState,
        record: bool,
        hist: HistSrc,
        cx: &mut Context<Self>,
    ) -> RestoreOutcome {
        let Some(s) = self.hub.session(id) else {
            return RestoreOutcome::Nothing;
        };
        let (Some(caps), Some(cur)) = (s.caps.clone(), s.state.clone()) else {
            return RestoreOutcome::Nothing;
        };
        if s.switching_profile {
            return RestoreOutcome::Nothing;
        }
        // Writing a diff against another config's snapshot would corrupt the post-switch config — switch first.
        if cur.profile.active != target.profile.active {
            if caps
                .profiles
                .as_ref()
                .is_none_or(|pc| target.profile.active >= pc.count)
            {
                return RestoreOutcome::Nothing;
            }
            if record {
                self.last_snap = None;
                self.snapshot(id);
            }
            self.pending_restore = Some(PendingRestore {
                id: id.clone(),
                state: target.clone(),
                rollback_undo: matches!(hist, HistSrc::Undo).then(|| target.clone()),
                rollback_redo: matches!(hist, HistSrc::Redo).then(|| target.clone()),
            });
            self.hub.apply(id, Change::Profile(target.profile.active));
            cx.notify();
            return RestoreOutcome::Switching;
        }
        let changes = crate::store::diff_changes(&caps, &cur, target);
        if changes.is_empty() {
            return RestoreOutcome::Applied(0);
        }
        if record {
            self.last_snap = None;
            self.snapshot(id);
        }
        self.restoring = true;
        let n = changes.len();
        for c in changes {
            self.apply(c, cx);
        }
        self.restoring = false;
        self.last_snap = None;
        RestoreOutcome::Applied(n)
    }

    /// Apply a preset to the currently selected config (compatibility is checked before applying).
    pub fn apply_preset(&mut self, name: &str, mut st: DeviceState, cx: &mut Context<Self>) {
        let Some((caps, cur)) = self
            .device_id()
            .and_then(|id| self.hub.session(&id))
            .and_then(|s| s.caps.clone().zip(s.state.clone()))
        else {
            self.toast("Device settings have not been loaded yet", true, cx);
            return;
        };
        match crate::store::state_compatible(&caps, &st) {
            Err(e) => {
                self.toast(format!("Cannot apply \"{name}\": {e}"), true, cx);
                return;
            }
            Ok(Some(w)) => self.toast(w, false, cx),
            Ok(None) => {}
        }
        // A preset is applied only to the currently selected config.
        st.profile.active = cur.profile.active;
        match self.apply_state(&st, true, cx) {
            RestoreOutcome::Applied(n) => self.toast(
                format!("Applied \"{name}\" ({n} item(s)) — Ctrl+Z to undo"),
                false,
                cx,
            ),
            RestoreOutcome::Switching => self.toast("Switching configuration to apply…", false, cx),
            RestoreOutcome::Nothing => self.toast("Could not apply", true, cx),
        }
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.device_id() else { return };
        let Some(pos) = self.undo.iter().rposition(|(d, _)| *d == id) else {
            self.toast("Nothing to undo", false, cx);
            return;
        };
        let (_, st) = self.undo.remove(pos);
        if let Some(cur) = self.hub.session(&id).and_then(|s| s.state.clone()) {
            self.redo.push((id.clone(), cur));
        }
        match self.apply_state_to(&id, &st, false, HistSrc::Undo, cx) {
            RestoreOutcome::Applied(n) => {
                self.toast(format!("Undone ({n} item(s))"), false, cx)
            }
            RestoreOutcome::Switching => self.toast("Switching configuration to undo…", false, cx),
            RestoreOutcome::Nothing => {
                self.undo.push((id, st));
                self.redo.pop();
            }
        }
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.device_id() else { return };
        let Some(pos) = self.redo.iter().rposition(|(d, _)| *d == id) else {
            return;
        };
        let (_, st) = self.redo.remove(pos);
        if let Some(cur) = self.hub.session(&id).and_then(|s| s.state.clone()) {
            self.undo.push((id.clone(), cur));
        }
        match self.apply_state_to(&id, &st, false, HistSrc::Redo, cx) {
            RestoreOutcome::Applied(n) => {
                self.toast(format!("Redone ({n} item(s))"), false, cx)
            }
            RestoreOutcome::Switching => self.toast("Switching configuration to redo…", false, cx),
            RestoreOutcome::Nothing => {
                self.redo.push((id, st));
                self.undo.pop();
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        self.device_id()
            .is_some_and(|id| self.undo.iter().any(|(d, _)| *d == id))
    }
    pub fn can_redo(&self) -> bool {
        self.device_id()
            .is_some_and(|id| self.redo.iter().any(|(d, _)| *d == id))
    }

    /// Run with a confirmation dialog.
    pub fn confirm(
        &mut self,
        title: &str,
        body: &str,
        ok: &str,
        danger: bool,
        on_ok: Cb,
        cx: &mut Context<Self>,
    ) {
        self.modal = Some(Modal::Confirm {
            title: title.into(),
            body: body.into(),
            ok: ok.into(),
            danger,
            on_ok,
        });
        cx.notify();
    }

    pub fn open_popup(
        &mut self,
        anchor_id: &str,
        items: Vec<PopItem>,
        min_w: f32,
        on_pick: CbIdx,
        cx: &mut Context<Self>,
    ) {
        let anchor = motion::rect(anchor_id).unwrap_or([200., 200., 160., 32.]);
        self.popup = Some(Popup {
            anchor,
            items,
            on_pick,
            min_w,
            born: Instant::now(),
        });
        cx.notify();
    }

    pub fn begin_drag(
        &mut self,
        rect: &str,
        vertical: bool,
        apply: CbF,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(Drag {
            rect: rect.into(),
            apply,
            vertical,
            xy: None,
        });
        self.drag_to(pos, cx);
    }

    /// Start a 2D drag (`f` is called with x, y = 0..1; y is 1 at the top).
    pub fn begin_drag_xy(
        &mut self,
        rect: &str,
        f: CbXY,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(Drag {
            rect: rect.into(),
            apply: Rc::new(|_, _, _| {}),
            vertical: false,
            xy: Some(f),
        });
        self.drag_to(pos, cx);
    }

    fn drag_to(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(d) = &self.drag else { return };
        let Some(r) = motion::rect(&d.rect) else {
            return;
        };
        if let Some(f) = d.xy.clone() {
            let tx = ((f32::from(pos.x) - r[0]) / r[2].max(1.)).clamp(0., 1.);
            let ty = 1. - ((f32::from(pos.y) - r[1]) / r[3].max(1.)).clamp(0., 1.);
            f(self, tx, ty, cx);
            cx.notify();
            return;
        }
        let t = if d.vertical {
            1. - ((f32::from(pos.y) - r[1]) / r[3].max(1.)).clamp(0., 1.)
        } else {
            ((f32::from(pos.x) - r[0]) / r[2].max(1.)).clamp(0., 1.)
        };
        let f = d.apply.clone();
        f(self, t, cx);
        cx.notify();
    }

    /// Text input (one is created per key and reused).
    pub fn input(
        &mut self,
        key: &str,
        initial: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(e) = self.inputs.get(key) {
            return e.clone();
        }
        let ph = placeholder.to_string();
        let init = initial.to_string();
        let e = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(ph);
            s.set_value(init, window, cx);
            s
        });
        let k = key.to_string();
        cx.subscribe_in(
            &e,
            window,
            move |this: &mut Shell, state, ev: &InputEvent, window, cx| {
                if k == "palette" {
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        this.palette_pick(0, window, cx);
                    } else if matches!(ev, InputEvent::Change) {
                        cx.notify();
                    }
                    return;
                }
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = state.read(cx).value().to_string();
                    this.commit_input(&k, v, window, cx);
                } else if matches!(ev, InputEvent::Change) && (k == "palette" || k == "key_search")
                {
                    cx.notify();
                }
            },
        )
        .detach();
        self.inputs.insert(key.to_string(), e.clone());
        e
    }

    /// Rewrite an input's value from outside (e.g. when another slot is selected).
    pub fn set_input(&mut self, key: &str, v: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = self.inputs.get(key).cloned() {
            let v = v.to_string();
            e.update(cx, |s, cx| s.set_value(v, window, cx));
        }
    }

    /// Numeric input: prepare the `num:{id}` entity and, if the value changed (unless focused),
    /// sync the display. `f` is called with the quantized value on Enter/focus-out.
    pub fn num_field(
        &mut self,
        id: &str,
        value: f32,
        min: f32,
        max: f32,
        step: f32,
        unit: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: Rc<dyn Fn(&mut Shell, f32, &mut Context<Shell>)>,
    ) -> Entity<InputState> {
        self.num_fields.insert(id.to_string(), (min, max, step, f));
        let key = format!("num:{id}");
        let e = self.input(&key, "", unit, window, cx);
        let text = crate::ui::fmt_num(value, step, unit);
        let focused = e.read(cx).focus_handle(cx).is_focused(window);
        if !focused && self.num_sync.get(id).map(String::as_str) != Some(text.as_str()) {
            self.set_input(&key, &text, window, cx);
            self.num_sync.insert(id.to_string(), text);
        }
        e
    }

    pub fn input_value(&self, key: &str, cx: &App) -> Option<String> {
        self.inputs.get(key).map(|e| e.read(cx).value().to_string())
    }

    fn commit_input(&mut self, key: &str, v: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = key.strip_prefix("num:") {
            // Numeric input: if it cannot be parsed, keep what is shown (it returns to the canonical form on the next render).
            let v = v.trim().replace(',', ".");
            if let (Ok(v), Some((min, max, step, f))) =
                (v.parse::<f32>(), self.num_fields.get(id).cloned())
            {
                let v = crate::ui::quantize(v, min, max, step);
                f(self, v, cx);
            }
            self.num_sync.remove(id);
            cx.notify();
            return;
        }
        match key {
            "dpi_exact" => crate::pages::mouse::commit_dpi_text(self, &v, false, cx),
            "dpi_exact_y" => crate::pages::mouse::commit_dpi_text(self, &v, true, cx),
            "obs_title" => {
                self.obs_edit(cx, |c| c.title_text = v.trim().to_string());
            }
            "macro_name" => {
                if let Some(d) = self.macro_draft.as_mut() {
                    d.name = v;
                    cx.notify();
                }
            }
            "modal" => {
                // Enter acts the same as OK.
                if matches!(self.modal, Some(Modal::Text { .. })) {
                    if let Some(Modal::Text { on_ok, .. }) = self.modal.take() {
                        on_ok(self, v, window, cx);
                    }
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ Theme

    pub fn set_light(&mut self, light: bool, cx: &mut Context<Self>) {
        self.prefs.theme = if light { "light" } else { "dark" }.into();
        self.prefs.save();
        theme::set_light(light);
        Theme::change(
            if light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
            None,
            cx,
        );
        clear_component_chrome(cx);
        cx.notify();
    }

    // ------------------------------------------------------------------ Key input

    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Key capture while assigning a button.
        if let Some(Modal::Button {
            draft,
            capture: true,
            ..
        }) = self.modal.as_mut()
        {
            if let Some(u) = crate::keycode::usage_of(&e.keystroke.key) {
                let m = crate::keycode::mods_of(&e.keystroke.modifiers);
                let m = if crate::keycode::usage_of(&e.keystroke.key)
                    .is_some_and(|u| (0xE0..=0xE7).contains(&u))
                {
                    0
                } else {
                    m
                };
                *draft = ButtonAction::Key(KeyStroke { mods: m, usage: u });
            }
            if let Some(Modal::Button { capture, .. }) = self.modal.as_mut() {
                *capture = false;
            }
            cx.notify();
            return;
        }
        // Shortcut capture.
        if let Some(Modal::CaptureKey { got, .. }) = self.modal.as_mut() {
            if let Some(u) = crate::keycode::usage_of(&e.keystroke.key) {
                let m = crate::keycode::mods_of(&e.keystroke.modifiers);
                *got = Some(KeyStroke { mods: m, usage: u });
                cx.notify();
            }
            return;
        }
        if self.recording.is_some() {
            if e.is_held {
                return;
            }
            if let Some(u) = crate::keycode::usage_of(&e.keystroke.key) {
                self.record_event(MacroInput::Key(u), MacroEdge::Down, cx);
            }
            return;
        }
        let root_focused = self.focus.is_focused(window);
        let m = e.keystroke.modifiers;
        if root_focused
            && self.modal.is_none()
            && self.tab == Tab::Test
            && matches!(self.route, Route::Device(_))
        {
            self.test_key(e, true, cx);
            if !(m.control && matches!(e.keystroke.key.as_str(), "z" | "y" | "k")) {
                return;
            }
        }
        match e.keystroke.key.as_str() {
            "escape" if self.popup.take().is_some() || self.modal.take().is_some() => {
                cx.notify();
            }
            "k" if m.control => {
                self.open_palette(window, cx);
            }
            "z" if m.control && !m.shift && root_focused && self.modal.is_none() => self.undo(cx),
            "y" if m.control && root_focused && self.modal.is_none() => self.redo(cx),
            "z" if m.control && m.shift && root_focused && self.modal.is_none() => self.redo(cx),
            "a" if e.keystroke.modifiers.control && self.modal.is_none() => {
                if let Some(n) = self.keyboard_len() {
                    if matches!(self.tab, Tab::Trigger | Tab::Lighting) {
                        self.kb_sel = (0..n).collect();
                        cx.notify();
                    }
                }
            }
            _ => {}
        }
        let _ = window;
    }

    fn on_key_up(&mut self, e: &KeyUpEvent, cx: &mut Context<Self>) {
        if self.tab == Tab::Test && self.modal.is_none() {
            self.test_key_up(e, cx);
        }
        if self.recording.is_some() {
            if let Some(u) = crate::keycode::usage_of(&e.keystroke.key) {
                self.record_event(MacroInput::Key(u), MacroEdge::Up, cx);
            }
        }
    }

    fn on_modifiers(&mut self, e: &ModifiersChangedEvent, cx: &mut Context<Self>) {
        if let Some(Modal::CaptureKey { got, .. }) = self.modal.as_mut() {
            // Also show presses of modifiers alone.
            let m = crate::keycode::mods_of(&e.modifiers);
            if m != 0 && got.is_none_or(|g| g.usage == 0) {
                *got = Some(KeyStroke { mods: m, usage: 0 });
                cx.notify();
            }
            return;
        }
        let Some(rec) = self.recording.as_ref() else {
            if self.tab == Tab::Test
                && self.modal.is_none()
                && matches!(self.route, Route::Device(_))
            {
                self.test_mods(e, cx);
            }
            return;
        };
        let prev = rec.mods;
        let now = e.modifiers;
        let pairs = [
            (prev.control, now.control, 0xE0u16),
            (prev.shift, now.shift, 0xE1),
            (prev.alt, now.alt, 0xE2),
            (prev.platform, now.platform, 0xE3),
        ];
        for (a, b, u) in pairs {
            if a != b {
                self.record_event(
                    MacroInput::Key(u),
                    if b { MacroEdge::Down } else { MacroEdge::Up },
                    cx,
                );
            }
        }
        if let Some(r) = self.recording.as_mut() {
            r.mods = now;
        }
    }

    pub fn record_event(&mut self, input: MacroInput, edge: MacroEdge, cx: &mut Context<Self>) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        let now = Instant::now();
        let since = (now - rec.last).as_millis() as u32;
        rec.last = now;
        let (mode, fixed) = (rec.delay_mode, rec.fixed_ms);
        let max = self
            .macro_caps()
            .map(|m| m.max_events as usize)
            .unwrap_or(256);
        let max_delay = self.macro_caps().map(|m| m.max_delay_ms).unwrap_or(65_535);
        let Some(d) = self.macro_draft.as_mut() else {
            return;
        };
        // Fill the previous event's "wait after" in real time.
        if let Some(last) = d.events.last_mut() {
            last.delay_ms = match mode {
                0 => since.min(max_delay),
                1 => fixed,
                _ => 0,
            };
        }
        if d.events.len() < max {
            d.events.push(MacroEvent {
                input,
                edge,
                delay_ms: 0,
            });
        }
        cx.notify();
    }

    pub fn macro_caps(&self) -> Option<MacroCaps> {
        let s = self.cur()?;
        let c = s.caps.as_ref()?;
        c.mouse
            .as_ref()
            .and_then(|m| m.macros.clone())
            .or_else(|| c.keyboard.as_ref().and_then(|k| k.macros.clone()))
    }

    fn keyboard_len(&self) -> Option<usize> {
        self.cur()?
            .caps
            .as_ref()?
            .keyboard
            .as_ref()
            .map(|k| k.layout.len())
    }

    /// Session of the open device.
    pub fn cur(&self) -> Option<&crate::store::Session> {
        match &self.route {
            Route::Device(id) => self.hub.session(id),
            _ => None,
        }
    }

    // ------------------------------------------------------------------ Sidebar

    fn sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let mut list = div().flex().flex_col().gap(px(4.));
        let mut devices = self.hub.devices.clone();
        devices.sort_by_key(|d| (!d.supported, d.kind != DeviceKind::Mouse, d.name.clone()));
        for d in devices {
            let selected = self.route == Route::Device(d.id.clone());
            list = list.child(self.side_device(&d, selected, cx));
        }
        if self.hub.devices.is_empty() {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(10.))
                    .text_size(px(12.))
                    .text_color(p.faint)
                    .child(if self.hub.scanned {
                        "No devices found"
                    } else {
                        "Searching…"
                    }),
            );
        }

        let settings_sel = self.route == Route::Settings;
        let obs_sel = self.route == Route::Obs;
        let home_sel = self.route == Route::Home;
        div()
            .relative()
            .w(px(theme::SIDEBAR_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .rounded(px(26.))
            .shadow(vec![BoxShadow {
                color: p.shadow,
                offset: point(px(0.), px(14.)),
                blur_radius: px(40.),
                spread_radius: px(-10.),
                inset: false,
            }])
            .child(glass::surface(glass::Kind::Clear, 26.))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .pt(px(46.))
                    .px(px(10.))
                    .pb(px(10.))
                    // Brand
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(8.))
                            .pb(px(16.))
                            .child(
                                div()
                                    .size(px(30.))
                                    .rounded(px(10.))
                                    .bg(theme::accent_grad())
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .shadow(vec![BoxShadow {
                                        color: theme::alpha(p.accent, 0.45),
                                        offset: point(px(0.), px(6.)),
                                        blur_radius: px(16.),
                                        spread_radius: px(-4.),
                                        inset: false,
                                    }])
                                    .child(ui::icon("orbit", 17., p.on_accent)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(p.fg)
                                            .child("Device Hub"),
                                    )
                                    .child(div().text_size(px(10.5)).text_color(p.faint).child(
                                        if self.hub.demo {
                                            "Demo mode"
                                        } else {
                                            "No browser needed"
                                        },
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id("side-search")
                            .h(px(34.))
                            .mb(px(6.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(11.))
                            .cursor_pointer()
                            .bg(p.ctrl)
                            .border_1()
                            .border_color(p.hair)
                            .hover(|s| s.bg(p.ctrl_hover))
                            .child(ui::icon("search", 14., p.faint))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(12.5))
                                    .text_color(p.faint)
                                    .child("Search"),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(p.faint)
                                    .child("Ctrl+K"),
                            )
                            .on_click(cx.listener(|this, _, w, cx| this.open_palette(w, cx))),
                    )
                    .child(self.side_nav(
                        "home",
                        "layout-grid",
                        "Home",
                        home_sel,
                        Route::Home,
                        cx,
                    ))
                    .child(
                        ui::section_label("Devices")
                            .px(px(10.))
                            .pt(px(14.))
                            .pb(px(6.)),
                    )
                    .child(
                        div()
                            .id("side-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(list),
                    )
                    .child(div().h(px(1.)).mx(px(8.)).my(px(8.)).bg(p.hair))
                    .child(self.side_nav(
                        "obs",
                        "cast",
                        "OBS overlay",
                        obs_sel,
                        Route::Obs,
                        cx,
                    ))
                    .child(self.side_nav(
                        "auto",
                        "zap",
                        "Automation",
                        self.route == Route::Auto,
                        Route::Auto,
                        cx,
                    ))
                    .child(self.side_nav(
                        "settings",
                        "settings",
                        "App settings",
                        settings_sel,
                        Route::Settings,
                        cx,
                    ))
                    .child(self.theme_switch(cx)),
            )
    }

    fn side_nav(
        &mut self,
        id: &str,
        icon: &'static str,
        label: &str,
        sel: bool,
        r: Route,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = theme::pal();
        let k = motion::ease(&format!("nav:{id}"), sel);
        div()
            .id(SharedString::from(format!("nav-{id}")))
            .relative()
            .h(px(38.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(p.ctrl))
            .when(k > 0.01, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(12.))
                        .bg(theme::alpha(p.accent, 0.18 * k))
                        .border_1()
                        .border_color(theme::alpha(p.accent, 0.28 * k)),
                )
            })
            .child(ui::icon(icon, 16., if sel { p.accent } else { p.sub }))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(if sel {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if sel { p.fg } else { p.sub })
                    .child(label.to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.go(r.clone(), cx)))
    }

    fn side_device(
        &mut self,
        d: &DeviceInfo,
        sel: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = theme::pal();
        let k = motion::ease(&format!("dev:{}", d.id), sel);
        let id = d.id.clone();
        let name = self
            .prefs
            .nicknames
            .get(&d.id.0)
            .cloned()
            .unwrap_or_else(|| d.name.clone());
        let icon = match d.kind {
            DeviceKind::Mouse => "mouse",
            DeviceKind::Keyboard => "keyboard",
            DeviceKind::Other if d.driver == DriverKind::Walkplay => "headphones",
            DeviceKind::Other => "usb",
            DeviceKind::Audio => "headphones",
        };
        let sub = if !d.supported {
            "Unsupported".to_string()
        } else if d.asleep {
            "Asleep".to_string()
        } else {
            match d.transport {
                Transport::Wired => "Wired".to_string(),
                Transport::Wireless24G => "2.4GHz".to_string(),
                Transport::Bluetooth => "Bluetooth".to_string(),
                Transport::Unknown => "Connected".to_string(),
            }
        };
        let pending = self
            .hub
            .session(&d.id)
            .map(|s| s.pending > 0)
            .unwrap_or(false);
        div()
            .id(SharedString::from(format!("dev-{}", d.id)))
            .relative()
            .px(px(8.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(14.))
            .cursor_pointer()
            .hover(|s| s.bg(p.ctrl))
            .when(k > 0.01, |dv| {
                dv.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(14.))
                        .bg(theme::alpha(p.accent, 0.16 * k))
                        .border_1()
                        .border_color(theme::alpha(p.accent, 0.3 * k)),
                )
            })
            .child(
                div()
                    .size(px(34.))
                    .rounded(px(11.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if d.supported {
                        p.ctrl
                    } else {
                        theme::alpha(p.warn, 0.12)
                    })
                    .border_1()
                    .border_color(p.hair)
                    .child(ui::icon(icon, 17., if sel { p.accent } else { p.fg })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(name),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .text_size(px(11.))
                            .text_color(if d.supported { p.faint } else { p.warn })
                            .child(sub)
                            .when(pending, |d| d.child(ui::sync_dot(true))),
                    ),
            )
            .when_some(
                d.battery.and_then(|b| b.percent.map(|pc| (pc, b.charging))),
                |dv, (pc, ch)| {
                    dv.child(if d.asleep && !ch {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(11.))
                            .text_color(p.faint)
                            .child(ui::icon("battery", 14., p.faint))
                            .child(format!("{pc}% · Asleep"))
                    } else {
                        ui::battery_pill(pc, ch)
                    })
                },
            )
            .when(
                self.hub
                    .fw_status
                    .get(&d.id)
                    .is_some_and(|s| s.new_available),
                |dv| {
                    dv.child(
                        div()
                            .size(px(9.))
                            .rounded_full()
                            .bg(p.accent)
                            .border_2()
                            .border_color(p.hair),
                    )
                },
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_device(id.clone());
                cx.notify();
            }))
    }

    fn theme_switch(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let light = theme::is_light();
        let opt = |id: &'static str, icon: &'static str, on: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .h(px(30.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(on, |d| d.bg(p.ctrl_hover).border_1().border_color(p.edge))
                .child(ui::icon(icon, 15., if on { p.fg } else { p.faint }))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_light(id == "theme-light", cx)),
                )
        };
        div()
            .mt(px(6.))
            .p(px(3.))
            .flex()
            .gap(px(3.))
            .rounded(px(13.))
            .bg(p.ctrl)
            .child(opt("theme-dark", "moon", !light, cx))
            .child(opt("theme-light", "sun", light, cx))
    }

    // ------------------------------------------------------------------ Content

    fn content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.route.clone() {
            Route::Home => self.page_home(cx).into_any_element(),
            Route::Settings => self.page_settings(window, cx).into_any_element(),
            Route::Obs => self.page_obs(window, cx).into_any_element(),
            Route::Auto => self.page_auto(window, cx).into_any_element(),
            Route::Device(id) => self.page_device(&id, window, cx),
        }
    }
}

/// Clear the window-frame colors of the gpui-component theme that get in the way of the glass.
pub fn clear_component_chrome(cx: &mut App) {
    let t = Theme::global_mut(cx);
    t.colors.title_bar = hsla(0., 0., 0., 0.);
    t.colors.title_bar_border = hsla(0., 0., 0., 0.);
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        motion::begin_frame();
        let p = theme::pal();
        let active = window.is_window_active();
        if active != self.last_window_active {
            self.last_window_active = active;
        }
        let wall = wallpaper::current(self.prefs.wallpaper.as_deref());
        let content = self.content(window, cx);
        let sidebar = self.sidebar(cx).into_any_element();

        let mut root = div()
            .id("root")
            .size_full()
            .relative()
            .font(theme::ui_font())
            .text_color(p.fg)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, w, cx| this.on_key_down(e, w, cx)))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, cx| this.on_key_up(e, cx)))
            .on_modifiers_changed(
                cx.listener(|this, e: &ModifiersChangedEvent, _, cx| this.on_modifiers(e, cx)),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                motion::set_pointer(f32::from(e.position.x), f32::from(e.position.y));
                if this.drag.is_some() {
                    this.drag_to(e.position, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    motion::set_down(true);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    motion::set_down(false);
                    this.drag = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    motion::set_down(false);
                    this.drag = None;
                    cx.notify();
                }),
            )
            .child(motion::track("root-size"))
            // Wallpaper
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(p.solid)
                    .child(img(wall).size_full().object_fit(ObjectFit::Cover)),
            )
            // Sidebar | content
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_row()
                    .p(px(12.))
                    .gap(px(12.))
                    .child(sidebar)
                    .child(div().flex_1().min_w_0().h_full().relative().child(content)),
            )
            // Title bar (transparent, draggable, window buttons at the right edge)
            .child(
                div().absolute().top_0().left_0().right_0().child(
                    TitleBar::new()
                        .h(px(40.))
                        .bg(hsla(0., 0., 0., 0.))
                        .border_0(),
                ),
            );

        if let Some(m) = self.modal_layer(window, cx) {
            root = root.child(m);
        }
        if let Some(pop) = self.popup_layer(cx) {
            root = root.child(pop);
        }
        if let Some(t) = self.toast_layer(cx) {
            root = root.child(t);
        }
        motion::end_frame(window);
        root
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        // On exit: stop DirectDrive so the device does not freeze on the last frame.
        // Allow a short grace period for delivery to the worker (the process may exit before it is delivered).
        self.hub.dd_stop_all();
        std::thread::sleep(Duration::from_millis(80));
    }
}

#[cfg(windows)]
fn window_hwnd(window: &Window) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

impl Shell {
    /// Memory management (called from the 2-second tick).
    /// - Minimized: return the working set to the OS every time (rendering stops, so it shrinks to ~5MB).
    /// - In the background (inactive): trim once on transition + lower the memory priority
    ///   to BELOW_NORMAL, and re-trim every 60 seconds.
    ///   Rendering still runs every 250ms in the background, so trimming every time
    ///   causes churn from page-fault round trips — with the priority lowered,
    ///   the OS reclaims from this process first when memory is short.
    /// - When returning to the foreground, restore the priority to NORMAL.
    #[cfg(windows)]
    fn mem_tick(&mut self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::Threading::{
            GetCurrentProcess, SetProcessInformation, SetProcessWorkingSetSize,
            MEMORY_PRIORITY_BELOW_NORMAL, MEMORY_PRIORITY_INFORMATION, MEMORY_PRIORITY_NORMAL,
            ProcessMemoryPriority,
        };
        use windows::Win32::UI::WindowsAndMessaging::IsIconic;
        if self.hwnd == 0 {
            return;
        }
        let iconic = unsafe { IsIconic(HWND(self.hwnd as _)).as_bool() };
        let inactive = iconic || !self.last_window_active;
        unsafe {
            let trim = || {
                let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
            };
            if inactive {
                if !self.mem_low {
                    self.mem_low = true;
                    let mpi = MEMORY_PRIORITY_INFORMATION {
                        MemoryPriority: MEMORY_PRIORITY_BELOW_NORMAL,
                    };
                    let _ = SetProcessInformation(
                        GetCurrentProcess(),
                        ProcessMemoryPriority,
                        &mpi as *const _ as *const _,
                        std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
                    );
                    trim(); // trim once on transition
                    self.mem_ticks = 0;
                }
                self.mem_ticks += 1;
                if iconic || self.mem_ticks >= 30 {
                    self.mem_ticks = 0;
                    trim();
                }
            } else if self.mem_low {
                self.mem_low = false;
                self.mem_ticks = 0;
                let mpi = MEMORY_PRIORITY_INFORMATION {
                    MemoryPriority: MEMORY_PRIORITY_NORMAL,
                };
                let _ = SetProcessInformation(
                    GetCurrentProcess(),
                    ProcessMemoryPriority,
                    &mpi as *const _ as *const _,
                    std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
                );
            }
        }
    }

    #[cfg(not(windows))]
    fn mem_tick(&mut self) {}
}
