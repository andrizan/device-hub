//! Live measurement of the device. A single background thread hits HID,
//! and the render side just reads the latest frame.

use devhub_core::backend::{DeviceDriver, HubBackend};
use devhub_core::model::{AxisSample, ButtonDepth, KeyPerf, LayoutKey};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Snapshot read by the render side.
#[derive(Clone, Default)]
pub struct Live {
    pub kb: Option<KbLive>,
    pub mouse: Option<MouseLive>,
    /// Guidance shown while no device has been found yet.
    pub status: String,
    /// Frame number (for change detection).
    pub seq: u64,
}

#[derive(Clone)]
pub struct KbLive {
    pub name: String,
    pub layout: Vec<LayoutKey>,
    /// Full stroke in mm (used for normalization).
    pub travel: f32,
    /// Actuation point in mm per key (in layout order; RT keys use First touch).
    /// Empty when `DeviceDriver::live_actuation` is unsupported or the read fails.
    pub actuation: Vec<f32>,
    pub perf: Vec<KeyPerf>,
    /// Same order as layout.
    pub samples: Vec<AxisSample>,
}

#[derive(Clone)]
pub struct MouseLive {
    pub name: String,
    /// Display names in the same order as `live_buttons`.
    pub names: Vec<String>,
    pub depths: Vec<ButtonDepth>,
    pub fast: Vec<Option<u8>>,
}

/// Keyboard-side target: driver / name / layout / travel (mm) / actuation points (mm, in layout order).
type KbTarget = (
    Arc<dyn DeviceDriver>,
    String,
    Vec<LayoutKey>,
    f32,
    Vec<f32>,
    Vec<KeyPerf>,
);
/// Mouse-side target: driver / name / list of button names / fast flags.
type MouseTarget = (Arc<dyn DeviceDriver>, String, Vec<String>, Vec<Option<u8>>);

struct Targets {
    kb: Option<KbTarget>,
    mouse: Option<MouseTarget>,
}

fn find_targets(hub: &dyn HubBackend, kb: bool, mouse: bool) -> Targets {
    let mut t = Targets {
        kb: None,
        mouse: None,
    };
    for info in hub.scan().unwrap_or_default() {
        if !info.supported
            || match info.kind {
                devhub_core::DeviceKind::Keyboard => !kb || t.kb.is_some(),
                devhub_core::DeviceKind::Mouse => !mouse || t.mouse.is_some(),
                _ => true,
            }
        {
            continue;
        }
        let Ok(d) = hub.driver(&info.id) else {
            continue;
        };
        let caps = d.caps();
        if let Some(k) = &caps.keyboard {
            if t.kb.is_none() && k.perf.as_ref().is_some_and(|p| p.live_travel) {
                // Read the actuation points only once, on discovery (the display still works if it fails, so treat it as empty).
                let perf = d.live_key_perf().unwrap_or_default();
                let actuation = if perf.is_empty() {
                    d.live_actuation().unwrap_or_default()
                } else {
                    vec![]
                };
                t.kb = Some((
                    d.clone(),
                    info.name.clone(),
                    k.layout.clone(),
                    k.perf.as_ref().unwrap().travel,
                    actuation,
                    perf,
                ));
            }
        }
        if let Some(m) = &caps.mouse {
            if t.mouse.is_none() && !m.live_buttons.is_empty() {
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
                t.mouse = Some((
                    d.clone(),
                    info.name.clone(),
                    names,
                    d.live_button_fast().unwrap_or_default(),
                ));
            }
        }
    }
    t
}

/// If one side fails to respond this many times in a row, drop the target and go back to searching
/// (so we do not keep holding an unplugged device; ~8 consecutive failures = no response for a few seconds).
const FAIL_DROP: u32 = 8;

fn loop_body(shared: &Arc<Mutex<Live>>, stop: &Arc<AtomicBool>) {
    let Ok(hub) = devhub_hid::connect() else {
        shared.lock().unwrap().status = "Cannot open HID".into();
        return;
    };
    let mut targets = find_targets(hub.as_ref(), true, true);
    let mut retry = Instant::now() - Duration::from_secs(10);
    let mut fails = (0u32, 0u32);
    while !stop.load(Ordering::Relaxed) {
        // If either side is missing, search again every 2 seconds, only for the missing side (handles plugging/unplugging).
        if (targets.kb.is_none() || targets.mouse.is_none())
            && retry.elapsed() > Duration::from_secs(2)
        {
            retry = Instant::now();
            let t = find_targets(hub.as_ref(), targets.kb.is_none(), targets.mouse.is_none());
            if targets.kb.is_none() {
                targets.kb = t.kb;
            }
            if targets.mouse.is_none() {
                targets.mouse = t.mouse;
            }
        }
        if targets.kb.is_none() && targets.mouse.is_none() {
            {
                let mut l = shared.lock().unwrap();
                l.kb = None;
                l.mouse = None;
                l.status = "Searching for supported devices…".into();
                l.seq += 1;
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        // Do not stall OBS rendering while waiting for HID responses. Measurement results are published together at the end.
        let mut next = Live::default();
        let mut drop_side = (false, false);
        if let Some((d, name, layout, actuation_travel, actuation, perf)) = &targets.kb {
            match d.live_axis() {
                Ok(samples) => {
                    fails.0 = 0;
                    next.kb = Some(KbLive {
                        name: name.clone(),
                        layout: layout.clone(),
                        travel: *actuation_travel,
                        actuation: actuation.clone(),
                        perf: perf.clone(),
                        samples,
                    });
                }
                Err(_) => {
                    fails.0 += 1;
                    drop_side.0 = fails.0 >= FAIL_DROP;
                    next.status = format!("No response from {name}");
                }
            }
        }
        if let Some((d, name, names, fast)) = &targets.mouse {
            match d.live_buttons() {
                Ok(depths) => {
                    fails.1 = 0;
                    next.mouse = Some(MouseLive {
                        name: name.clone(),
                        names: names.clone(),
                        depths,
                        fast: fast.clone(),
                    });
                }
                Err(_) => {
                    fails.1 += 1;
                    drop_side.1 = fails.1 >= FAIL_DROP;
                    next.status = format!("No response from {name}");
                }
            }
        }
        // Discard the side that failed repeatedly; it goes back to searching on the next pass.
        if drop_side.0 {
            targets.kb = None;
            fails.0 = 0;
            retry = Instant::now() - Duration::from_secs(2);
        }
        if drop_side.1 {
            targets.mouse = None;
            fails.1 = 0;
            retry = Instant::now() - Duration::from_secs(2);
        }
        {
            let mut l = shared.lock().unwrap();
            next.seq = l.seq.wrapping_add(1);
            *l = next;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub struct Poller {
    pub shared: Arc<Mutex<Live>>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Poller {
    pub fn start() -> Self {
        let shared = Arc::new(Mutex::new(Live {
            status: "Starting…".into(),
            ..Live::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, st2) = (shared.clone(), stop.clone());
        let join = std::thread::Builder::new()
            .name("devhub-obs-poll".into())
            .spawn(move || loop_body(&s2, &st2))
            .ok();
        Self { shared, stop, join }
    }
}

/// Poller shared by all sources (HID polling stays at one even with 3 sources registered).
/// It lives as long as the module, so it is never stopped.
pub fn shared() -> &'static Mutex<Live> {
    static S: std::sync::OnceLock<Arc<Mutex<Live>>> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let shared = Arc::new(Mutex::new(Live {
            status: "Starting…".into(),
            ..Live::default()
        }));
        let (s2, st2) = (shared.clone(), Arc::new(AtomicBool::new(false)));
        let _ = std::thread::Builder::new()
            .name("devhub-obs-poll".into())
            .spawn(move || loop_body(&s2, &st2));
        shared
    })
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
