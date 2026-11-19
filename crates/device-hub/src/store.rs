//! Bridge between the UI and the backend.
//!
//! - Holds one **dedicated worker thread** per device. All HID round trips happen there, and
//!   the UI thread is never blocked.
//! - Changes are **optimistic**: the UI state is rewritten immediately, then the `Change` is sent to the worker.
//!   The worker thins out consecutive changes that are fully replaced by a later value (drag handling).
//!   On failure it shows a toast and restores the device's value with `read`.
//! - Results come back to the UI over a futures channel (nothing runs at all when idle).

use crate::rawinput;
use devhub_core::*;
use futures::channel::mpsc::UnboundedSender;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

pub const SCAN_EVERY: Duration = Duration::from_secs(2);
pub const BATTERY_EVERY: Duration = Duration::from_secs(60);

/// Worker → UI.
pub enum Msg {
    /// Enumeration result. `gen` is the generation at issue time — older results are discarded.
    Scanned {
        gen: u64,
        res: Result<Vec<DeviceInfo>, String>,
    },
    Opened {
        id: DeviceId,
        info: DeviceInfo,
        caps: Box<Capabilities>,
    },
    OpenFailed {
        id: DeviceId,
        err: String,
    },
    /// `gen` is the `submit_seq` at the time that read was issued — pending changes
    /// submitted after it are layered onto the confirmed state. Carrying the generation in the
    /// response lets a stale response that arrives after another read still be layered correctly.
    State {
        id: DeviceId,
        gen: u64,
        state: Box<DeviceState>,
    },
    ReadFailed {
        id: DeviceId,
        err: String,
    },
    Applied {
        id: DeviceId,
        err: Option<String>,
    },
    /// Result of a config switch job (whether switch + full re-read succeeded).
    ProfileSwitched {
        id: DeviceId,
        err: Option<String>,
    },
    /// Live measurement stopped due to a failure on the device side (reverts the UI's travel_on).
    TravelStopped {
        id: DeviceId,
    },
    ActionDone {
        id: DeviceId,
        label: String,
        result: Result<ActionOutcome, String>,
    },
    Battery {
        id: DeviceId,
        battery: Battery,
    },
    Travel {
        id: DeviceId,
        depths: Vec<f32>,
        /// Raw data (only for drivers supporting `live_axis`; empty if unsupported).
        axis: Vec<devhub_core::AxisSample>,
    },
    /// Press depth of the mouse's magnetic buttons (same order as `MouseCaps::live_buttons`).
    Buttons {
        id: DeviceId,
        buttons: Vec<ButtonDepth>,
    },
    /// Result of a firmware update check (a failure is quietly dropped as Err).
    FwResult {
        id: DeviceId,
        res: Result<devhub_update::UpdateStatus, String>,
    },
    /// Result of fetching axis names (vendor axis ID → name). Failures are dropped.
    AxisNames {
        id: DeviceId,
        names: std::collections::BTreeMap<u16, String>,
    },
    /// Measured report rate (Raw Input).
    PollRate {
        id: DeviceId,
        stats: Option<rawinput::PollStats>,
    },
    /// DirectDrive send result. On Err the UI stops the on-device preview and shows a toast.
    DirectDrive {
        id: DeviceId,
        err: Option<String>,
    },
    /// The stop request went through, but the device's lighting could not be restored.
    DdStopFailed {
        id: DeviceId,
        err: String,
    },
    /// Readback of live lit colors (key index → color actually lit).
    /// While the Lighting screen is shown the worker sends this at ~6Hz. Does not arrive for unsupported devices.
    LiveLight {
        id: DeviceId,
        colors: std::collections::BTreeMap<usize, devhub_core::Rgb>,
    },
    Event(HubEvent),
}

enum Job {
    Open,
    /// Read all settings — `gen` is the submit_seq at issue time (used as the response's generation).
    Read {
        gen: u64,
    },
    Apply(Change),
    /// Config (on-board profile) switch — re-read all settings after switching.
    /// `gen` is the submit_seq at issue time (the generation of the internal re-read).
    Profile {
        profile: u8,
        gen: u64,
    },
    Action {
        id: String,
        label: String,
        file: Option<PathBuf>,
    },
    Battery,
    Travel(bool),
    /// Take a measured report rate once.
    PollRate,
    /// Send one DirectDrive frame (key index → color; the driver expands it to LED cells).
    DirectDrive {
        cells: std::collections::BTreeMap<usize, devhub_core::Rgb>,
    },
    /// Leave DirectDrive and return to firmware-side lighting.
    DdStop,
    /// Live lit-color polling ON/OFF (ON only while the Lighting screen is shown).
    LiveLight(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Opening,
    Ready,
    Failed(String),
}

/// One open device.
pub struct Session {
    pub id: DeviceId,
    pub info: DeviceInfo,
    pub caps: Option<Capabilities>,
    pub state: Option<DeviceState>,
    pub status: Status,
    /// Number of changes waiting to be written (header sync indicator).
    pub pending: usize,
    /// In the middle of a config switch + full re-read — edits within a config are not accepted.
    pub switching_profile: bool,
    /// The latest config switch failed (the app picks this up and discards the pending restore).
    pub switch_failed: bool,
    /// Serial number of jobs submitted to the worker (used to judge the read generation).
    submit_seq: u64,
    /// `submit_seq` at the time of the last read (Read/Open/Profile) issued.
    /// Unconfirmed changes submitted after this are layered on when `Msg::State` arrives.
    read_seq: u64,
    /// Unconfirmed changes sent to the worker (FIFO; confirmed from the front on `Applied`).
    pending_changes: VecDeque<(u64, Change)>,
    /// Operation in progress (label).
    pub running: Option<String>,
    /// Stroke test depth (mm).
    pub travel: Vec<f32>,
    /// Raw stroke measurement data (ADC, state). Empty for unsupported drivers.
    pub axis: Vec<devhub_core::AxisSample>,
    /// Maximum depth observed in this session (mm). Used to normalize the bar
    /// (because the measured bottom is shallower than the nominal full stroke).
    pub travel_max: f32,
    pub travel_on: bool,
    /// Whether Lighting's "Preview on device" is on (streaming DirectDrive frames).
    pub dd_on: bool,
    /// The Lighting screen is shown — the worker polls the device's lit colors with [5,3].
    live_lit: bool,
    /// Latest live lit colors (key index → color). Empty = not fetched / unsupported.
    pub live_rgb: std::collections::BTreeMap<usize, devhub_core::Rgb>,
    /// Press depth of the mouse's magnetic buttons (`live_buttons`).
    pub buttons: Vec<ButtonDepth>,
    /// Progress of a long operation (label, 0..1).
    pub progress: Option<(String, Option<f32>)>,
    /// Whether the low-battery notification was already sent.
    pub low_battery_warned: bool,
    /// User-initiated read — show a toast when it completes (not for background prefetch).
    pub read_notify: bool,
    /// Measured input report rate. Measured with Raw Input; the UI fetches it periodically.
    /// Includes average Hz, interval rate, peak, interval statistics and the movement cap (docs/POLLING_RATE.md).
    pub poll: Option<rawinput::PollStats>,
    tx: Sender<Job>,
}

impl Session {
    pub fn ready(&self) -> bool {
        self.status == Status::Ready && self.caps.is_some() && self.state.is_some()
    }
}

pub struct Hub {
    pub backend: Arc<dyn HubBackend>,
    pub demo: bool,
    pub devices: Vec<DeviceInfo>,
    pub sessions: HashMap<DeviceId, Session>,
    /// Whether the first scan returned.
    pub scanned: bool,
    /// Enumeration generation — increments with each issue; old results are discarded (backend switch handling).
    scan_gen: u64,
    /// Latest firmware update check result (per DeviceId). For the UI's badge/card.
    pub fw_status: HashMap<DeviceId, devhub_update::UpdateStatus>,
    /// Persisted fingerprint, last check time and dismiss (`%APPDATA%\DeviceHub\firmware.json`).
    pub fw: crate::fwstate::FwState,
    /// Whether to check automatically (prefs). If false, check_firmware does nothing.
    pub fw_enabled: bool,
    /// Whether to also show a Windows notification (prefs).
    pub fw_native: bool,
    /// A check fired by "Check now" — the result is always reported with a toast.
    fw_pending: HashSet<DeviceId>,
    out: UnboundedSender<Msg>,
    scanning: Arc<AtomicBool>,
}

/// Open the real-device backend. Demo if `DEVHUB_DEMO=1` or the device cannot be opened.
pub fn connect_backend() -> (Arc<dyn HubBackend>, bool) {
    let force_demo = std::env::var_os("DEVHUB_DEMO").is_some();
    if !force_demo {
        match devhub_hid::connect() {
            Ok(b) => return (b, false),
            Err(e) => eprintln!("[devhub] could not open HID ({e}) — starting in demo mode"),
        }
    }
    (Arc::new(devhub_core::demo::DemoHub::new()), true)
}

impl Hub {
    pub fn new(backend: Arc<dyn HubBackend>, out: UnboundedSender<Msg>) -> Self {
        let demo = backend.is_demo();
        let tx = out.clone();
        backend.watch(Box::new(move |ev| {
            let _ = tx.unbounded_send(Msg::Event(ev));
        }));
        Self {
            backend,
            demo,
            devices: vec![],
            sessions: HashMap::new(),
            scanned: false,
            scan_gen: 0,
            fw_status: HashMap::new(),
            fw: crate::fwstate::FwState::load(),
            fw_enabled: true,
            fw_native: false,
            fw_pending: HashSet::new(),
            out,
            scanning: Arc::new(AtomicBool::new(false)),
        }
    }

    /// When there is no real device at all, switch to demo (button on the settings screen).
    pub fn switch_backend(&mut self, backend: Arc<dyn HubBackend>) {
        self.sessions.clear();
        self.devices.clear();
        self.demo = backend.is_demo();
        self.backend = backend;
        self.scanned = false;
        let tx = self.out.clone();
        self.backend.watch(Box::new(move |ev| {
            let _ = tx.unbounded_send(Msg::Event(ev));
        }));
        self.scan();
    }

    /// Enumerate on a background thread (never started multiple times).
    pub fn scan(&mut self) {
        if self.scanning.swap(true, Ordering::AcqRel) {
            return;
        }
        self.scan_gen += 1;
        let gen = self.scan_gen;
        let b = self.backend.clone();
        let out = self.out.clone();
        let flag = self.scanning.clone();
        std::thread::spawn(move || {
            let r = b.scan().map_err(|e| e.to_string());
            flag.store(false, Ordering::Release);
            let _ = out.unbounded_send(Msg::Scanned { gen, res: r });
        });
    }

    pub fn session(&self, id: &DeviceId) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn session_mut(&mut self, id: &DeviceId) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// Open a session (does nothing if already open).
    /// `notify` = the user is looking at the screen (report read completion with a toast).
    pub fn open(&mut self, id: &DeviceId, notify: bool) {
        if let Some(s) = self.sessions.get_mut(id) {
            if notify && !s.ready() {
                s.read_notify = true;
            }
            return;
        }
        let Some(info) = self.devices.iter().find(|d| d.id == *id).cloned() else {
            return;
        };
        let (tx, rx) = mpsc::channel::<Job>();
        let backend = self.backend.clone();
        let out = self.out.clone();
        let did = id.clone();
        std::thread::Builder::new()
            .name(format!("devhub-{}", info.name))
            .spawn(move || worker(backend, did, rx, out))
            .ok();
        let supported = info.supported;
        let _ = tx.send(Job::Open);
        self.sessions.insert(
            id.clone(),
            Session {
                id: id.clone(),
                info,
                caps: None,
                state: None,
                status: if supported {
                    Status::Opening
                } else {
                    Status::Failed("No driver is implemented for this device yet".into())
                },
                pending: 0,
                running: None,
                travel: vec![],
                axis: vec![],
                travel_max: 0.0,
                travel_on: false,
                dd_on: false,
                live_lit: false,
                live_rgb: Default::default(),
                buttons: vec![],
                progress: None,
                low_battery_warned: false,
                read_notify: notify,
                switching_profile: false,
                switch_failed: false,
                submit_seq: 0,
                read_seq: 0,
                pending_changes: VecDeque::new(),
                poll: None,
                tx,
            },
        );
    }

    /// Reflect the change in the UI immediately and send it to the worker.
    pub fn apply(&mut self, id: &DeviceId, change: Change) {
        let Some(s) = self.sessions.get_mut(id) else {
            return;
        };
        // Edits are not accepted during a config switch + re-read (the UI guards this too).
        if s.switching_profile {
            return;
        }
        if let Change::Profile(p) = change {
            // A config switch is a dedicated job — state does not change until the switch and full re-read finish.
            let ok = s
                .caps
                .as_ref()
                .and_then(|c| c.profiles.as_ref())
                .is_some_and(|pc| pc.count > 1 && p < pc.count);
            if !ok {
                return;
            }
            s.switching_profile = true;
            s.switch_failed = false;
            if s.tx
                .send(Job::Profile {
                    profile: p,
                    gen: s.submit_seq,
                })
                .is_ok()
            {
                // The switch job re-reads everything internally — changes up to this point are included in the re-read.
                s.read_seq = s.submit_seq;
                s.pending += 1;
            } else {
                s.switching_profile = false;
            }
            return;
        }
        if let Some(st) = s.state.as_mut() {
            apply_local(st, &change);
        }
        if s.tx.send(Job::Apply(change.clone())).is_ok() {
            s.submit_seq += 1;
            s.pending_changes.push_back((s.submit_seq, change));
            s.pending += 1;
        }
    }

    /// The user pressed "Reload" — show a completion toast.
    pub fn reload(&mut self, id: &DeviceId) {
        self.reload_inner(id, true);
    }

    /// Send DirectDrive stop to all sessions (at app exit — so the device does not
    /// freeze on the last frame).
    pub fn dd_stop_all(&self) {
        for s in self.sessions.values() {
            let _ = s.tx.send(Job::DdStop);
        }
    }

    /// Background re-read, e.g. on a device-side change notification (`notify=false` = no toast).
    fn reload_inner(&mut self, id: &DeviceId, notify: bool) {
        if let Some(s) = self.sessions.get_mut(id) {
            s.read_notify = notify;
            if matches!(s.status, Status::Failed(_)) {
                s.status = Status::Opening;
                send_open(s);
            } else {
                send_read(s);
            }
        }
    }

    pub fn run_action(&mut self, id: &DeviceId, action: &str, label: &str, file: Option<PathBuf>) {
        if let Some(s) = self.sessions.get_mut(id) {
            s.running = Some(label.to_string());
            let _ = s.tx.send(Job::Action {
                id: action.into(),
                label: label.into(),
                file,
            });
        }
    }

    /// Take a measured report rate once (for the UI's periodic polling).
    pub fn poll_rate(&self, id: &DeviceId) {
        if let Some(s) = self.sessions.get(id) {
            let _ = s.tx.send(Job::PollRate);
        }
    }

    pub fn poll_battery(&self) {
        for s in self.sessions.values() {
            let has_battery = s.caps.as_ref().is_some_and(|c| c.battery);
            if s.ready() && (has_battery || s.info.battery.is_some()) {
                let _ = s.tx.send(Job::Battery);
            }
        }
    }

    pub fn set_travel(&mut self, id: &DeviceId, on: bool) {
        if let Some(s) = self.sessions.get_mut(id) {
            if s.travel_on != on {
                s.travel_on = on;
                if !on {
                    s.travel.clear();
                    s.axis.clear();
                    s.buttons.clear();
                }
                let _ = s.tx.send(Job::Travel(on));
            }
        }
    }

    /// Lighting "Preview on device" ON/OFF. OFF sends DdStop and
    /// returns to the firmware-side animation.
    pub fn set_dd(&mut self, id: &DeviceId, on: bool) {
        if let Some(s) = self.sessions.get_mut(id) {
            if s.dd_on != on {
                s.dd_on = on;
                if !on {
                    let _ = s.tx.send(Job::DdStop);
                }
            }
        }
    }

    /// Live lit-color polling: ON only for the session `id`, OFF for all others.
    /// `None` stops everything. The UI calls this only while the Lighting screen is shown.
    pub fn live_light(&mut self, id: Option<&DeviceId>) {
        for s in self.sessions.values_mut() {
            // Do not send to devices with no area that has live_readback in caps
            // (an unsupported driver returns Unsupported immediately, but we keep to the look-only-at-caps principle).
            let supported = s
                .caps
                .as_ref()
                .is_some_and(|c| c.lighting.iter().any(|a| a.live_readback));
            let on = Some(&s.id) == id && supported;
            if s.live_lit != on {
                s.live_lit = on;
                if !on {
                    s.live_rgb.clear();
                }
                let _ = s.tx.send(Job::LiveLight(on));
            }
        }
    }

    /// Send one DirectDrive frame to the worker (only while dd_on).
    /// The worker thins out consecutive frames.
    pub fn dd_frame(
        &self,
        id: &DeviceId,
        cells: std::collections::BTreeMap<usize, devhub_core::Rgb>,
    ) {
        if let Some(s) = self.sessions.get(id) {
            if s.dd_on && !s.switching_profile && s.ready() {
                let _ = s.tx.send(Job::DirectDrive { cells });
            }
        }
    }

    /// Key of firmware.json — grouped by model (one entry even for several units of the same model).
    fn fw_key(info: &DeviceInfo) -> String {
        if info.model.is_empty() {
            info.id.0.clone()
        } else {
            info.model.clone()
        }
    }

    /// Check the distribution source for one device (HTTP uses a dedicated thread — the UI/HID workers are not stopped).
    /// `force` = skip the 24h throttle for "Check now". Returns true if a check was fired.
    pub fn check_firmware(&mut self, id: &DeviceId, force: bool) -> bool {
        if !self.fw_enabled {
            return false;
        }
        let Some(s) = self.sessions.get(id) else {
            return false;
        };
        let Some(feed) = s.caps.as_ref().and_then(|c| c.firmware_feed.clone()) else {
            return false;
        };
        let key = Self::fw_key(&s.info);
        if !self.fw.due(&key, force) {
            return false;
        }
        self.fw.touch(&key);
        self.fw.save();
        let last_fp = self
            .fw
            .devices
            .get(&key)
            .map(|e| e.fingerprint.clone())
            .unwrap_or_default();
        let cur_fw = s.info.firmware.clone();
        let out = self.out.clone();
        let did = id.clone();
        if force {
            self.fw_pending.insert(id.clone());
        }
        std::thread::spawn(move || {
            let res = devhub_update::check(&feed, cur_fw.as_deref(), &last_fp).map_err(|e| e.0);
            let _ = out.unbounded_send(Msg::FwResult { id: did, res });
        });
        true
    }

    /// "Check now" — check the feed of every open session. Returns the number of checks fired.
    pub fn check_all_firmware(&mut self, force: bool) -> usize {
        let ids: Vec<DeviceId> = self.sessions.keys().cloned().collect();
        ids.iter()
            .filter(|id| self.check_firmware(id, force))
            .count()
    }

    /// Dismiss the update card/badge (the same fingerprint is not notified again).
    pub fn fw_dismiss(&mut self, id: &DeviceId) {
        let Some(st) = self.fw_status.get_mut(id) else {
            return;
        };
        st.new_available = false;
        let fp = st.fingerprint.clone();
        if let Some(s) = self.sessions.get(id) {
            let key = Self::fw_key(&s.info);
            self.fw.devices.entry(key).or_default().dismissed = fp;
            self.fw.save();
        }
    }

    /// Take in results from the worker. Return value: what to show in a toast (text, is-error).
    pub fn handle(&mut self, msg: Msg) -> Option<(String, bool)> {
        match msg {
            Msg::Scanned { gen, .. } if gen != self.scan_gen => {
                // Discard stale enumerations that arrive late, e.g. after a backend switch.
                None
            }
            Msg::Scanned {
                res: Ok(mut list), ..
            } => {
                self.scanned = true;
                // Close the sessions of unplugged devices.
                self.sessions
                    .retain(|id, _| list.iter().any(|d| d.id == *id));
                for d in &mut list {
                    // Carry over the battery value from the previous list/session (scan has no battery).
                    if d.battery.is_none() {
                        d.battery = self
                            .devices
                            .iter()
                            .find(|o| o.id == d.id)
                            .and_then(|o| o.battery)
                            .or_else(|| self.sessions.get(&d.id).and_then(|s| s.info.battery));
                    }
                    if let Some(s) = self.sessions.get_mut(&d.id) {
                        // asleep is unknown to scan — copy the session's (driver's) value to the list.
                        // Overwriting in the reverse direction (d→s) would make asleep disappear every 2 seconds.
                        d.asleep = s.info.asleep;
                        // Same for the resolved name of the model inside a dongle (enumeration returns the receiver's name).
                        if s.info.model != d.model {
                            d.name = s.info.name.clone();
                            d.model = s.info.model.clone();
                        }
                        if d.battery.is_none() {
                            d.battery = s.info.battery;
                        }
                    }
                }
                self.devices = list;
                // Open sessions of supported devices in the background to get battery/notifications
                // (the screen is not opened. open() does nothing if already open).
                let ids: Vec<DeviceId> = self
                    .devices
                    .iter()
                    .filter(|d| d.supported)
                    .map(|d| d.id.clone())
                    .collect();
                for id in ids {
                    if !self.sessions.contains_key(&id) {
                        self.open(&id, false);
                    }
                }
                None
            }
            Msg::Scanned { res: Err(e), .. } => {
                self.scanned = true;
                Some((format!("Failed to enumerate devices: {e}"), true))
            }
            Msg::Opened { id, info, caps } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
                        d.asleep = info.asleep;
                        // name/model can change when the model inside a dongle is resolved
                        // (EID detection — mousefit8::maybe_detect).
                        d.name = info.name.clone();
                        d.model = info.model.clone();
                        if info.battery.is_some() {
                            d.battery = info.battery;
                        }
                    }
                    s.info = info;
                    s.caps = Some(*caps);
                    send_read(s);
                    // Fetch axis names in the background (vendor API; nothing changes if it fails).
                    if let (Some(bid), Some(pc)) = (
                        s.info.board_id,
                        s.caps
                            .as_ref()
                            .and_then(|c| c.keyboard.as_ref())
                            .and_then(|k| k.perf.as_ref()),
                    ) {
                        if !pc.switch_ids.is_empty() {
                            let (vid, pid) = (s.info.vid, s.info.pid);
                            let out = self.out.clone();
                            let did = id.clone();
                            std::thread::spawn(move || {
                                if let Ok(names) =
                                    devhub_update::sparklink_axis_names(bid, vid, pid)
                                {
                                    let _ = out.unbounded_send(Msg::AxisNames { id: did, names });
                                }
                            });
                        }
                    }
                }
                // Check for updates once opened (skipped internally if within 24h).
                self.check_firmware(&id, false);
                None
            }
            Msg::PollRate { id, stats } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.poll = stats;
                }
                None
            }
            Msg::AxisNames { id, names } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    if let Some(pc) = s
                        .caps
                        .as_mut()
                        .and_then(|c| c.keyboard.as_mut())
                        .and_then(|k| k.perf.as_mut())
                    {
                        for (i, id2) in pc.switch_ids.iter().enumerate() {
                            if let Some(n) = names.get(id2) {
                                if let Some(slot) = pc.switch_types.get_mut(i) {
                                    *slot = format!("{n} (axis {id2})");
                                }
                            }
                        }
                    }
                }
                None
            }
            Msg::OpenFailed { id, err } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.status = Status::Failed(err);
                }
                None
            }
            Msg::State { id, gen, state } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    // Layer pending changes submitted after this read was issued — don't roll back
                    // optimistic changes with a stale read (they are still running).
                    // `gen` is the generation carried by the response, so even a delayed response that arrives
                    // before a later read is issued is layered correctly.
                    let mut st = *state;
                    for (seq, c) in &s.pending_changes {
                        if *seq > gen {
                            apply_local(&mut st, c);
                        }
                    }
                    s.state = Some(st);
                    s.status = Status::Ready;
                    if s.read_notify {
                        s.read_notify = false;
                        return Some((format!("Loaded settings for {}", s.info.name), false));
                    }
                }
                None
            }
            Msg::ReadFailed { id, err } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.read_notify = false;
                    if s.state.is_none() {
                        s.status = Status::Failed(err.clone());
                    }
                }
                Some((format!("Failed to load: {err}"), true))
            }
            Msg::DirectDrive { id, err } => {
                let s = self.sessions.get_mut(&id)?;
                if let Some(e) = err {
                    // On failure, stop the on-device preview, return the device lighting to firmware, and re-read the state.
                    s.dd_on = false;
                    let _ = s.tx.send(Job::DdStop);
                    send_read(s);
                    return Some((format!("On-device preview failed: {e}"), true));
                }
                None
            }
            Msg::DdStopFailed { id, err } => {
                self.sessions.get(&id)?;
                Some((
                    format!("Failed to restore lighting after stopping DirectDrive: {err}"),
                    true,
                ))
            }
            Msg::LiveLight { id, colors } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    if s.live_lit {
                        s.live_rgb = colors;
                    }
                }
                None
            }
            Msg::TravelStopped { id } => {
                let s = self.sessions.get_mut(&id)?;
                if !s.travel_on {
                    return None; // failure after the user had already stopped it
                }
                s.travel_on = false;
                s.travel.clear();
                s.axis.clear();
                Some(("Live measurement stopped on the device side".into(), true))
            }
            Msg::Applied { id, err } => {
                let s = self.sessions.get_mut(&id)?;
                s.pending = s.pending.saturating_sub(1);
                s.pending_changes.pop_front();
                if let Some(e) = err {
                    // Return to the device's value.
                    send_read(s);
                    return Some((format!("Write failed: {e}"), true));
                }
                None
            }
            Msg::ProfileSwitched { id, err } => {
                let s = self.sessions.get_mut(&id)?;
                s.switching_profile = false;
                s.pending = s.pending.saturating_sub(1);
                if let Some(e) = err {
                    // The switch or the re-read right after it failed — we do not know which config's values
                    // these are, so discard the state and re-read (do not write old values back into another config).
                    s.state = None;
                    s.switch_failed = true;
                    send_read(s);
                    return Some((format!("Failed to switch configuration: {e}"), true));
                }
                None
            }
            Msg::ActionDone { id, label, result } => {
                let s = self.sessions.get_mut(&id)?;
                s.running = None;
                match result {
                    Ok(o) => {
                        if o.reload {
                            send_read(s);
                        }
                        Some((
                            o.message.unwrap_or_else(|| format!("{label}: completed")),
                            false,
                        ))
                    }
                    Err(e) => Some((format!("{label}: {e}"), true)),
                }
            }
            Msg::Battery { id, battery } => {
                let mut warn = None;
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.info.battery = Some(battery);
                    if let Some(p) = battery.percent {
                        if p <= 15 && !battery.charging && !s.low_battery_warned {
                            s.low_battery_warned = true;
                            warn = Some((format!("{} battery is at {p}%", s.info.name), true));
                        }
                        if p > 30 {
                            s.low_battery_warned = false;
                        }
                    }
                }
                if warn.is_some() {
                    if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
                        d.battery = Some(battery);
                    }
                    return warn;
                }
                if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
                    d.battery = Some(battery);
                }
                None
            }
            Msg::Travel { id, depths, axis } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    if s.travel_on {
                        s.travel_max = depths.iter().fold(s.travel_max, |a, &b| a.max(b));
                        s.travel = depths;
                        s.axis = axis;
                    }
                }
                None
            }
            Msg::Buttons { id, buttons } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    if s.travel_on {
                        s.buttons = buttons;
                    }
                }
                None
            }
            Msg::FwResult { id, res } => {
                let manual = self.fw_pending.remove(&id);
                let mut st = match res {
                    Ok(st) => st,
                    Err(e) => {
                        // Report a failure of a manual check; quietly drop a failure of an automatic check.
                        return manual.then(|| (format!("Firmware check failed: {e}"), true));
                    }
                };
                let s = self.sessions.get(&id)?;
                let key = Self::fw_key(&s.info);
                let name = s.info.name.clone();
                // If the same fingerprint was dismissed before, do not show it this time either.
                let dismissed =
                    self.fw.devices.get(&key).is_some_and(|e| {
                        e.dismissed == st.fingerprint && !st.fingerprint.is_empty()
                    });
                let entry = self.fw.devices.entry(key).or_default();
                if !st.fingerprint.is_empty() {
                    entry.fingerprint = st.fingerprint.clone();
                }
                self.fw.save();
                st.new_available = st.new_available && !dismissed;
                let fresh = st.new_available;
                self.fw_status.insert(id, st);
                if !fresh {
                    return manual.then(|| (format!("{name} firmware is up to date"), false));
                }
                if self.fw_native {
                    let _ = notify_rust::Notification::new()
                        .summary("Device HUB")
                        .body(&format!("New firmware is available for {name}"))
                        .show();
                }
                Some((format!("New firmware is available for {name}"), false))
            }
            Msg::Event(ev) => {
                match ev {
                    HubEvent::DevicesChanged => self.scan(),
                    HubEvent::StateChanged(id) => self.reload_inner(&id, false),
                    HubEvent::Battery(id, b) => {
                        if let Some(s) = self.sessions.get_mut(&id) {
                            s.info.battery = Some(b);
                        }
                        if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
                            d.battery = Some(b);
                        }
                    }
                    HubEvent::Info(id, info) => {
                        // Changes in sleep/battery picked up by the driver go to the session and the list.
                        let mut woke = false;
                        // The model (name/model) was resolved/changed, e.g. by EID detection → caps change too,
                        // so re-fetch info+caps with Job::Open.
                        let mut renamed = false;
                        if let Some(s) = self.sessions.get_mut(&id) {
                            // Sleep → wake transition: re-read the empty state that was read while asleep.
                            woke = s.info.asleep && !info.asleep;
                            renamed = s.info.model != info.model || s.info.name != info.name;
                            s.info.asleep = info.asleep;
                            if info.battery.is_some() {
                                s.info.battery = info.battery;
                            }
                            if renamed {
                                s.info.name = info.name.clone();
                                s.info.model = info.model.clone();
                                let _ = s.tx.send(Job::Open);
                            }
                        }
                        if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
                            d.asleep = info.asleep;
                            if renamed {
                                d.name = info.name.clone();
                                d.model = info.model.clone();
                            }
                            if info.battery.is_some() {
                                d.battery = info.battery;
                            }
                        }
                        if woke {
                            self.reload_inner(&id, false);
                        }
                    }
                    HubEvent::Progress {
                        id,
                        label,
                        fraction,
                        done,
                    } => {
                        if let Some(s) = self.sessions.get_mut(&id) {
                            s.progress = if done { None } else { Some((label, fraction)) };
                        }
                    }
                }
                None
            }
        }
    }
}

// ------------------------------------------------------------------ Worker

/// Request a read/reopen — records the generation, so unconfirmed changes submitted
/// after it are layered onto the confirmed state when `Msg::State` arrives (no rollback).
fn send_read(s: &mut Session) {
    s.read_seq = s.submit_seq;
    let _ = s.tx.send(Job::Read { gen: s.read_seq });
}

fn send_open(s: &mut Session) {
    s.read_seq = s.submit_seq;
    let _ = s.tx.send(Job::Open);
}

/// Thin out only consecutive jobs. An intermediate read/operation or another change may
/// depend on an earlier write. Skipped Applys are still counted in the UI's pending-write count.
fn coalesce_jobs(jobs: impl IntoIterator<Item = Job>) -> (Vec<Job>, usize) {
    let mut kept = Vec::new();
    let mut skipped_applies = 0;
    for job in jobs {
        let replace = match (kept.last(), &job) {
            (Some(Job::Apply(prev)), Job::Apply(next)) if prev.is_superseded_by(next) => {
                skipped_applies += 1;
                true
            }
            (Some(Job::DirectDrive { .. }), Job::DirectDrive { .. }) => true,
            _ => false,
        };
        if replace {
            kept.pop();
        }
        kept.push(job);
    }
    (kept, skipped_applies)
}

fn worker(
    backend: Arc<dyn HubBackend>,
    id: DeviceId,
    rx: mpsc::Receiver<Job>,
    out: UnboundedSender<Msg>,
) {
    let mut driver: Option<Arc<dyn DeviceDriver>> = None;
    let mut vidpid: Option<(u16, u16)> = None;
    let mut travel = false;
    // Live lit-color polling (only while the Lighting screen is shown). ~33ms interval —
    // measured: a 9-chunk round trip takes ~3.4ms (litpoll). Enough for effect motion to look smooth.
    let mut live_lit = false;
    let mut live_at = std::time::Instant::now();
    let send = |m: Msg| out.unbounded_send(m).is_ok();
    loop {
        let first = if travel || live_lit {
            // During travel, depth takes priority at 33ms; lit-color polling uses the same 33ms period.
            let wait = 33;
            match rx.recv_timeout(Duration::from_millis(wait)) {
                Ok(j) => Some(j),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(j) => Some(j),
                Err(_) => return,
            }
        };
        let (kept, skipped) = coalesce_jobs(first.into_iter().chain(rx.try_iter()));
        for _ in 0..skipped {
            if !send(Msg::Applied {
                id: id.clone(),
                err: None,
            }) {
                return;
            }
        }
        for job in kept {
            let ok = match job {
                Job::Open => match backend.driver(&id) {
                    Ok(d) => {
                        let info = d.info();
                        let caps = d.caps();
                        vidpid = Some((info.vid, info.pid));
                        driver = Some(d);
                        send(Msg::Opened {
                            id: id.clone(),
                            info,
                            caps: Box::new(caps),
                        })
                    }
                    Err(e) => send(Msg::OpenFailed {
                        id: id.clone(),
                        err: e.to_string(),
                    }),
                },
                Job::Read { gen } => match driver.as_ref().map(|d| d.read()) {
                    Some(Ok(st)) => send(Msg::State {
                        id: id.clone(),
                        gen,
                        state: Box::new(st),
                    }),
                    Some(Err(e)) => {
                        if matches!(e, devhub_core::backend::HubError::Disconnected) {
                            backend.invalidate(&id);
                            driver = None; // Discard the stale handle and re-acquire it by reopening.
                        }
                        send(Msg::ReadFailed {
                            id: id.clone(),
                            err: e.to_string(),
                        })
                    }
                    None => true,
                },
                Job::Apply(c) => {
                    let (err, dead) = match &driver {
                        Some(d) => match d.apply(&c) {
                            Ok(()) => (None, false),
                            Err(e) => (
                                Some(e.to_string()),
                                matches!(e, devhub_core::backend::HubError::Disconnected),
                            ),
                        },
                        None => (Some("Device is not open".into()), false),
                    };
                    if dead {
                        backend.invalidate(&id);
                        driver = None;
                    }
                    send(Msg::Applied {
                        id: id.clone(),
                        err,
                    })
                }
                // Config switch: after a successful write, re-read all settings and then report.
                Job::Profile { profile: p, gen } => {
                    let (err, dead) = match &driver {
                        Some(d) => match d.apply(&Change::Profile(p)) {
                            Ok(()) => match d.read() {
                                Ok(st) => {
                                    send(Msg::State {
                                        id: id.clone(),
                                        gen,
                                        state: Box::new(st),
                                    });
                                    (None, false)
                                }
                                Err(e) => (
                                    Some(e.to_string()),
                                    matches!(e, devhub_core::backend::HubError::Disconnected),
                                ),
                            },
                            Err(e) => (
                                Some(e.to_string()),
                                matches!(e, devhub_core::backend::HubError::Disconnected),
                            ),
                        },
                        None => (Some("Device is not open".into()), false),
                    };
                    if dead {
                        backend.invalidate(&id);
                        driver = None;
                    }
                    send(Msg::ProfileSwitched {
                        id: id.clone(),
                        err,
                    })
                }
                Job::Action {
                    id: aid,
                    label,
                    file,
                } => {
                    let (result, dead) = match &driver {
                        Some(d) => match d.run_action(&aid, file.as_deref()) {
                            Ok(o) => (Ok(o), false),
                            Err(e) => (
                                Err(e.to_string()),
                                matches!(e, devhub_core::backend::HubError::Disconnected),
                            ),
                        },
                        None => (Err("Device is not open".into()), false),
                    };
                    if dead {
                        backend.invalidate(&id);
                        driver = None;
                    }
                    send(Msg::ActionDone {
                        id: id.clone(),
                        label,
                        result,
                    })
                }
                Job::Battery => match driver.as_ref().map(|d| d.battery()) {
                    Some(Ok(b)) => send(Msg::Battery {
                        id: id.clone(),
                        battery: b,
                    }),
                    _ => true,
                },
                Job::PollRate => {
                    // Raw Input arrival rate (measured in-process — no driver needed).
                    send(Msg::PollRate {
                        id: id.clone(),
                        stats: vidpid.and_then(|(v, p)| rawinput::stats(v, p)),
                    })
                }
                Job::Travel(on) => {
                    travel = on;
                    true
                }
                Job::DirectDrive { cells } => {
                    let err = match &driver {
                        Some(d) => d.direct_drive(0, &cells).err().map(|e| e.to_string()),
                        None => Some("Device is not open".into()),
                    };
                    send(Msg::DirectDrive {
                        id: id.clone(),
                        err,
                    })
                }
                Job::DdStop => {
                    let result = driver.as_ref().map(|d| d.direct_drive_stop(0));
                    match result {
                        Some(Err(e)) if !matches!(e, HubError::Unsupported(_)) => {
                            // At exit the UI may already have closed.
                            send(Msg::DdStopFailed {
                                id: id.clone(),
                                err: e.to_string(),
                            })
                        }
                        _ => true,
                    }
                }
                Job::LiveLight(on) => {
                    live_lit = on;
                    // Set a past time so it can be read once immediately.
                    live_at = std::time::Instant::now()
                        .checked_sub(Duration::from_millis(33))
                        .unwrap_or_else(std::time::Instant::now);
                    true
                }
            };
            if !ok {
                return;
            }
        }
        // Lit-color polling — an unsupported driver returns Unsupported immediately, so
        // even on failure we just retry on the next tick (HID is not touched).
        if live_lit && live_at.elapsed() >= Duration::from_millis(33) {
            live_at = std::time::Instant::now();
            if let Some(d) = &driver {
                if let Ok(colors) = d.live_lighting(0) {
                    if !send(Msg::LiveLight {
                        id: id.clone(),
                        colors,
                    }) {
                        return;
                    }
                }
            }
        }
        if travel {
            if let Some(d) = &driver {
                // Mouse magnetic buttons → live_buttons. Keyboards prefer live_axis
                // (raw data), falling back to live_travel if unsupported.
                if let Ok(buttons) = d.live_buttons() {
                    if !send(Msg::Buttons {
                        id: id.clone(),
                        buttons,
                    }) {
                        return;
                    }
                    continue;
                }
                let got = match d.live_axis() {
                    Ok(a) => Ok((a.iter().map(|s| s.route).collect::<Vec<_>>(), a)),
                    Err(_) => d.live_travel().map(|v| (v, Vec::new())),
                };
                match got {
                    Ok((depths, axis)) => {
                        if !send(Msg::Travel {
                            id: id.clone(),
                            depths,
                            axis,
                        }) {
                            return;
                        }
                    }
                    Err(_) => {
                        travel = false;
                        send(Msg::TravelStopped { id: id.clone() });
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Optimistic application

/// Reflect a `Change` into the UI-side state (same meaning as the driver's write).
pub fn apply_local(s: &mut DeviceState, change: &Change) {
    match change.clone() {
        Change::Setting { id, value } => {
            s.settings.insert(id, value);
        }
        Change::Profile(p) => s.profile.active = p,
        Change::ProfileName { index, name } => {
            if let Some(n) = s.profile.names.get_mut(index as usize) {
                *n = name;
            }
        }
        Change::DpiStages { stages, active } => {
            if let Some(m) = s.mouse.as_mut() {
                m.stages = stages;
                m.active_stage = active.min(m.stages.len().saturating_sub(1));
            }
        }
        Change::ActiveStage(a) => {
            if let Some(m) = s.mouse.as_mut() {
                m.active_stage = a;
            }
        }
        Change::Button { slot, action } => {
            if let Some(b) = s.mouse.as_mut().and_then(|m| m.buttons.get_mut(slot)) {
                *b = action;
            }
        }
        Change::Macro { slot, def } => {
            let list = if let Some(m) = s.mouse.as_mut() {
                &mut m.macros
            } else if let Some(k) = s.keyboard.as_mut() {
                &mut k.macros
            } else {
                return;
            };
            if slot < list.len() {
                list[slot] = def;
            }
        }
        Change::Lighting { area, state } => {
            s.lighting.insert(area, state);
        }
        Change::KeyMap { layer, key, action } => {
            if let Some(slot) = s
                .keyboard
                .as_mut()
                .and_then(|k| k.keymap.get_mut(layer as usize))
                .and_then(|l| l.get_mut(key))
            {
                *slot = action;
            }
        }
        Change::KeyMapReset { .. } => {
            // The UI does not know the default value. Re-read after the worker (send Read after Applied).
        }
        Change::KeyPerf { keys, perf } => {
            if let Some(k) = s.keyboard.as_mut() {
                for i in keys {
                    if let Some(p) = k.perf.get_mut(i) {
                        *p = perf;
                    }
                }
            }
        }
        Change::Advanced {
            key,
            layer,
            binding,
        } => {
            if let Some(k) = s.keyboard.as_mut() {
                k.advanced.retain(|a| !(a.key == key && a.layer == layer));
                if let Some(b) = binding {
                    k.advanced.push(AdvancedKey {
                        key,
                        layer,
                        binding: b,
                    });
                }
            }
        }
        Change::Peq { band, value } => {
            if let Some(b) = s.peq.as_mut().and_then(|p| p.get_mut(band)) {
                *b = value;
            }
        }
        Change::PeqAll { bands } => {
            if s.peq.is_some() {
                s.peq = Some(bands);
            }
        }
        Change::Adaptive(curve) => {
            if let Some(m) = s.mouse.as_mut().filter(|m| m.adaptive.is_some()) {
                m.adaptive = Some(curve);
            }
        }
    }
}

// ------------------------------------------------------------------ Diff

/// Change list to bring `cur` to `target` (only within caps' range).
/// Shared by undo / redo, preset application and loading from a file.
pub fn diff_changes(caps: &Capabilities, cur: &DeviceState, target: &DeviceState) -> Vec<Change> {
    let mut out: Vec<Change> = vec![];
    for (id, v) in &target.settings {
        if caps.setting(id).is_some() && cur.settings.get(id) != Some(v) {
            out.push(Change::Setting {
                id: id.clone(),
                value: v.clone(),
            });
        }
    }
    if let (Some(t), Some(c), Some(mc)) = (&target.mouse, &cur.mouse, &caps.mouse) {
        if (t.stages != c.stages || t.active_stage != c.active_stage)
            && !t.stages.is_empty()
            && t.stages.len() <= mc.max_stages
        {
            if t.stages == c.stages {
                out.push(Change::ActiveStage(t.active_stage));
            } else {
                out.push(Change::DpiStages {
                    stages: t.stages.clone(),
                    active: t.active_stage,
                });
            }
        }
        for (i, b) in t.buttons.iter().enumerate().take(mc.buttons.len()) {
            if c.buttons.get(i) != Some(b) {
                out.push(Change::Button {
                    slot: i,
                    action: b.clone(),
                });
            }
        }
        for (i, d) in t.macros.iter().enumerate() {
            if i < c.macros.len() && c.macros.get(i) != Some(d) {
                out.push(Change::Macro {
                    slot: i,
                    def: d.clone(),
                });
            }
        }
    }
    if let (Some(t), Some(c), Some(kc)) = (&target.keyboard, &cur.keyboard, &caps.keyboard) {
        for (l, layer) in t.keymap.iter().enumerate().take(kc.layers.len()) {
            for (i, a) in layer.iter().enumerate().take(kc.layout.len()) {
                if c.keymap.get(l).and_then(|x| x.get(i)) != Some(a) {
                    out.push(Change::KeyMap {
                        layer: l as u8,
                        key: i,
                        action: *a,
                    });
                }
            }
        }
        // Merge keys of the same setting into one entry (to reduce the number of writes).
        let mut groups: Vec<(KeyPerf, Vec<usize>)> = vec![];
        for (i, pf) in t.perf.iter().enumerate().take(kc.layout.len()) {
            if c.perf.get(i) != Some(pf) {
                match groups.iter_mut().find(|g| g.0 == *pf) {
                    Some(g) => g.1.push(i),
                    None => groups.push((*pf, vec![i])),
                }
            }
        }
        for (pf, keys) in groups {
            out.push(Change::KeyPerf { keys, perf: pf });
        }
        for a in &c.advanced {
            if !t
                .advanced
                .iter()
                .any(|b| b.key == a.key && b.layer == a.layer)
            {
                out.push(Change::Advanced {
                    key: a.key,
                    layer: a.layer,
                    binding: None,
                });
            }
        }
        for a in &t.advanced {
            if !c.advanced.contains(a) {
                out.push(Change::Advanced {
                    key: a.key,
                    layer: a.layer,
                    binding: Some(a.binding.clone()),
                });
            }
        }
        for (i, d) in t.macros.iter().enumerate() {
            if i < c.macros.len() && c.macros.get(i) != Some(d) {
                out.push(Change::Macro {
                    slot: i,
                    def: d.clone(),
                });
            }
        }
    }
    for (a, l) in &target.lighting {
        if caps.lighting.iter().any(|x| x.id == *a) && cur.lighting.get(a) != Some(l) {
            out.push(Change::Lighting {
                area: *a,
                state: l.clone(),
            });
        }
    }
    if let (Some(t), Some(c), Some(pc)) = (&target.peq, &cur.peq, &caps.peq) {
        if t != c && t.len() == pc.bands as usize {
            out.push(Change::PeqAll { bands: t.clone() });
        }
    }
    if let (Some(t), Some(c), Some(_)) = (
        target.mouse.as_ref().and_then(|m| m.adaptive.as_ref()),
        cur.mouse.as_ref().and_then(|m| m.adaptive.as_ref()),
        caps.mouse.as_ref().and_then(|m| m.adaptive_dpi.as_ref()),
    ) {
        if t != c {
            out.push(Change::Adaptive(t.clone()));
        }
    }
    // The config switch itself is not emitted here — `Job::Profile` bundles switch + re-read.
    // (so a diff from before the switch is not written into the config after it). Only the name can be restored by a normal write.
    if caps.profiles.as_ref().is_some_and(|pc| pc.renamable) {
        for (i, n) in target.profile.names.iter().enumerate() {
            if cur.profile.names.get(i) != Some(n) {
                out.push(Change::ProfileName {
                    index: i as u8,
                    name: n.clone(),
                });
            }
        }
    }
    out
}

/// The whole current `DeviceState` as a `Change` list (the profile's "record current state").
/// Keymaps, buttons, macros and advanced keys are not included — a full rewrite means many writes and
/// wears the EEPROM, so this is limited to scalar settings, DPI, performance, lighting and EQ.
pub fn snapshot_changes(caps: &Capabilities, st: &DeviceState) -> Vec<Change> {
    let mut out = vec![];
    for (id, v) in &st.settings {
        if caps.setting(id).is_some() {
            out.push(Change::Setting {
                id: id.clone(),
                value: v.clone(),
            });
        }
    }
    if let (Some(m), Some(mc)) = (&st.mouse, &caps.mouse) {
        if !m.stages.is_empty() && m.stages.len() <= mc.max_stages {
            out.push(Change::DpiStages {
                stages: m.stages.clone(),
                active: m.active_stage,
            });
        }
    }
    if let (Some(k), Some(kc)) = (&st.keyboard, &caps.keyboard) {
        // Merge keys of the same setting into one entry (as in diff_changes).
        let mut groups: Vec<(devhub_core::KeyPerf, Vec<usize>)> = vec![];
        for (i, pf) in k.perf.iter().enumerate().take(kc.layout.len()) {
            match groups.iter_mut().find(|g| g.0 == *pf) {
                Some(g) => g.1.push(i),
                None => groups.push((*pf, vec![i])),
            }
        }
        for (perf, keys) in groups {
            out.push(Change::KeyPerf { keys, perf });
        }
    }
    for (a, l) in &st.lighting {
        if caps.lighting.iter().any(|x| x.id == *a) {
            out.push(Change::Lighting {
                area: *a,
                state: l.clone(),
            });
        }
    }
    if let (Some(t), Some(pc)) = (&st.peq, &caps.peq) {
        if t.len() == pc.bands as usize {
            out.push(Change::PeqAll { bands: t.clone() });
        }
    }
    if let (Some(t), Some(_)) = (
        st.mouse.as_ref().and_then(|m| m.adaptive.as_ref()),
        caps.mouse.as_ref().and_then(|m| m.adaptive_dpi.as_ref()),
    ) {
        out.push(Change::Adaptive(t.clone()));
    }
    out
}

/// Whether a loaded `DeviceState` (preset/export JSON) can be applied to this device.
/// `Err` = cannot apply, e.g. it is for another model. `Ok(Some(msg))` = can apply with a warning.
pub fn state_compatible(caps: &Capabilities, st: &DeviceState) -> Result<Option<String>, String> {
    if st.keyboard.is_some() && caps.keyboard.is_none() {
        return Err("This is a settings file for keyboards".into());
    }
    if st.mouse.is_some() && caps.mouse.is_none() {
        return Err("This is a settings file for mice".into());
    }
    if let Some(bands) = &st.peq {
        match &caps.peq {
            Some(pc) if bands.len() == pc.bands as usize => {}
            Some(_) => return Err("The EQ band count does not match".into()),
            None => return Err("This is a settings file for EQ-capable devices".into()),
        }
    }
    // Count items that cannot be used as is (what diff_changes actually drops).
    let mut ignored = 0usize;
    for id in st.settings.keys() {
        if caps.setting(id).is_none() {
            ignored += 1;
        }
    }
    for a in st.lighting.keys() {
        if !caps.lighting.iter().any(|x| x.id == *a) {
            ignored += 1;
        }
    }
    if let (Some(k), Some(kc)) = (&st.keyboard, &caps.keyboard) {
        ignored += k.keymap.len().saturating_sub(kc.layers.len()) * kc.layout.len();
        for layer in k.keymap.iter().take(kc.layers.len()) {
            ignored += layer.len().saturating_sub(kc.layout.len());
        }
    }
    if ignored > 0 {
        Ok(Some(format!(
            "{ignored} unsupported settings will be ignored"
        )))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod worker_queue_tests {
    use super::*;

    fn band(gain_db: f32) -> PeqBand {
        PeqBand {
            freq: 1000,
            q: 1.,
            gain: gain_db,
            filter: PeqFilter::Peak,
        }
    }

    #[test]
    fn one_band_edit_preserves_the_rest_of_a_queued_eq_preset() {
        let (jobs, skipped) = coalesce_jobs([
            Job::Apply(Change::PeqAll {
                bands: vec![band(3.), band(5.)],
            }),
            Job::Apply(Change::Peq {
                band: 0,
                value: band(7.),
            }),
        ]);
        assert_eq!(skipped, 0);
        assert!(matches!(jobs.as_slice(), [
            Job::Apply(Change::PeqAll { bands }),
            Job::Apply(Change::Peq { band: 0, value }),
        ] if bands[1].gain == 5. && value.gain == 7.));
    }

    #[test]
    fn consecutive_writes_are_coalesced_without_crossing_read_or_profile_switch() {
        let stage = |i| Job::Apply(Change::ActiveStage(i));
        let (jobs, skipped) = coalesce_jobs([
            stage(0),
            stage(1),
            Job::Read { gen: 0 },
            stage(2),
            Job::Apply(Change::Profile(1)),
            stage(3),
        ]);
        assert_eq!(skipped, 1);
        assert!(matches!(
            jobs.as_slice(),
            [
                Job::Apply(Change::ActiveStage(1)),
                Job::Read { gen: 0 },
                Job::Apply(Change::ActiveStage(2)),
                Job::Apply(Change::Profile(1)),
                Job::Apply(Change::ActiveStage(3)),
            ]
        ));
    }

    #[test]
    fn direct_drive_frames_respect_stop_and_coalesce_within_a_run() {
        let frame = |i| Job::DirectDrive {
            cells: [(i, Rgb::new(0, 0, 0))].into(),
        };
        let (jobs, skipped) = coalesce_jobs([frame(0), frame(1), Job::DdStop, frame(2)]);
        assert_eq!(skipped, 0);
        assert!(matches!(jobs.as_slice(), [
            Job::DirectDrive { cells: before }, Job::DdStop, Job::DirectDrive { cells: after },
        ] if before.contains_key(&1) && after.contains_key(&2)));
    }

    #[test]
    fn profile_switch_job_is_never_coalesced_into_writes() {
        // A config switch is a dedicated job — it is not mixed up with the writes just before or after it by thinning.
        let stage = |i| Job::Apply(Change::ActiveStage(i));
        let (jobs, skipped) =
            coalesce_jobs([stage(0), Job::Profile { profile: 2, gen: 0 }, stage(1)]);
        assert_eq!(skipped, 0);
        assert!(matches!(
            jobs.as_slice(),
            [
                Job::Apply(Change::ActiveStage(0)),
                Job::Profile { profile: 2, gen: 0 },
                Job::Apply(Change::ActiveStage(1)),
            ]
        ));
    }

    fn demo_kb() -> (Arc<dyn DeviceDriver>, Capabilities, DeviceState) {
        let hub = devhub_core::demo::DemoHub::new();
        let dev = hub
            .scan()
            .unwrap()
            .into_iter()
            .find(|d| d.kind == DeviceKind::Keyboard)
            .unwrap();
        let driver = hub.driver(&dev.id).unwrap();
        let caps = driver.caps();
        let state = driver.read().unwrap();
        (driver, caps, state)
    }

    #[test]
    fn demo_keeps_settings_separate_per_profile() {
        // Contents are separated per config (as on a real device) — a precondition so that undo to P1 does not break P2.
        let (driver, _, _) = demo_kb();
        driver
            .apply(&Change::KeyMap {
                layer: 0,
                key: 0,
                action: KeyAction::Code(0x1234),
            })
            .unwrap();
        driver.apply(&Change::Profile(1)).unwrap();
        let p1 = driver.read().unwrap();
        assert_eq!(p1.profile.active, 1);
        assert_ne!(
            p1.keyboard.as_ref().unwrap().keymap[0][0],
            KeyAction::Code(0x1234),
            "Changes from config 0 leaked into config 1"
        );
        driver.apply(&Change::Profile(0)).unwrap();
        let p0 = driver.read().unwrap();
        assert_eq!(
            p0.keyboard.as_ref().unwrap().keymap[0][0],
            KeyAction::Code(0x1234),
            "Changes of config 0 were lost"
        );
    }

    #[test]
    fn profile_switch_blocks_edits_until_the_reread_lands() {
        let backend: Arc<dyn HubBackend> = Arc::new(devhub_core::demo::DemoHub::new());
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Msg>();
        let mut hub = Hub::new(backend, tx);
        hub.scan();
        let mut kb = None;
        let t = std::time::Instant::now();
        while kb.is_none() && t.elapsed() < Duration::from_secs(3) {
            match rx.try_recv() {
                Ok(m) => {
                    if let Msg::Scanned { res: Ok(ds), .. } = &m {
                        kb = ds
                            .iter()
                            .find(|d| d.kind == DeviceKind::Keyboard)
                            .map(|d| d.id.clone());
                    }
                    hub.handle(m);
                }
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        let id = kb.expect("demo keyboard");
        hub.open(&id, false);
        assert!(drain_until(
            &mut hub,
            &mut rx,
            Duration::from_secs(5),
            |h| h.session(&id).is_some_and(|s| s.ready())
        ));

        // Send a config switch → no editing until switch + re-read finish, and state stays the old config.
        hub.apply(&id, Change::Profile(1));
        assert!(hub.session(&id).unwrap().switching_profile);
        assert_eq!(
            hub.session(&id)
                .unwrap()
                .state
                .as_ref()
                .unwrap()
                .profile
                .active,
            0
        );
        let pending = hub.session(&id).unwrap().pending;
        hub.apply(
            &id,
            Change::ProfileName {
                index: 0,
                name: "x".into(),
            },
        );
        assert_eq!(
            hub.session(&id).unwrap().pending,
            pending,
            "An edit during the switch reached the worker"
        );
        assert!(drain_until(
            &mut hub,
            &mut rx,
            Duration::from_secs(5),
            |h| {
                h.session(&id).is_some_and(|s| {
                    !s.switching_profile
                        && s.state.as_ref().is_some_and(|st| st.profile.active == 1)
                })
            }
        ));
    }

    fn drain_until(
        hub: &mut Hub,
        rx: &mut futures::channel::mpsc::UnboundedReceiver<Msg>,
        wait: Duration,
        pred: impl Fn(&Hub) -> bool,
    ) -> bool {
        let t = std::time::Instant::now();
        while t.elapsed() < wait {
            match rx.try_recv() {
                Ok(m) => {
                    hub.handle(m);
                }
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
            if pred(hub) {
                return true;
            }
        }
        false
    }

    #[test]
    fn diff_changes_emits_profile_names_but_never_a_switch() {
        let (_, caps, cur) = demo_kb();
        let mut target = cur.clone();
        target.profile.names = vec!["A".into(), "B".into(), "C".into(), "D".into()];
        target.profile.active = 2;
        let changes = diff_changes(&caps, &cur, &target);
        assert!(
            changes.iter().all(|c| !matches!(c, Change::Profile(_))),
            "The config switch appeared as a diff — a pre-switch diff would be written into the post-switch config"
        );
        assert_eq!(
            changes
                .iter()
                .filter(|c| matches!(c, Change::ProfileName { .. }))
                .count(),
            4,
            "The profile-name diff did not appear"
        );
    }

    #[test]
    fn state_replays_writes_issued_after_the_read_started() {
        // Changes submitted after the read was issued are not rolled back by the arrival of an old State.
        let backend: Arc<dyn HubBackend> = Arc::new(devhub_core::demo::DemoHub::new());
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Msg>();
        let mut hub = Hub::new(backend, tx);
        hub.scan();
        let mut kb = None;
        let t = std::time::Instant::now();
        while kb.is_none() && t.elapsed() < Duration::from_secs(3) {
            if let Ok(m) = rx.try_recv() {
                if let Msg::Scanned { res: Ok(ds), .. } = &m {
                    kb = ds
                        .iter()
                        .find(|d| d.kind == DeviceKind::Keyboard)
                        .map(|d| d.id.clone());
                }
                hub.handle(m);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let id = kb.expect("demo keyboard");
        hub.open(&id, false);
        assert!(drain_until(
            &mut hub,
            &mut rx,
            Duration::from_secs(5),
            |h| h.session(&id).is_some_and(|s| s.ready())
        ));

        // State before the change (assume the read returned this value).
        let stale = hub.backend.driver(&id).unwrap().read().unwrap();
        hub.apply(
            &id,
            Change::ProfileName {
                index: 0,
                name: "RENAMED".into(),
            },
        );
        // Even if a stale read arrives, pending changes are kept
        // (gen=0 is the response of a read issued before the change was submitted).
        hub.handle(Msg::State {
            id: id.clone(),
            gen: 0,
            state: Box::new(stale),
        });
        assert_eq!(
            hub.session(&id)
                .unwrap()
                .state
                .as_ref()
                .unwrap()
                .profile
                .names[0],
            "RENAMED"
        );
        // A newer read arriving after confirmation is reflected as is.
        drain_until(&mut hub, &mut rx, Duration::from_secs(5), |h| {
            h.session(&id).is_some_and(|s| s.pending_changes.is_empty())
        });
    }

    #[test]
    fn late_read_response_after_newer_read_keeps_edit() {
        // After issuing read A → edit → read B, even if A's late response arrives it must not
        // roll back the edited display (the generation is carried by the response, so it is unaffected by
        // the read_seq advanced by issuing B).
        let backend: Arc<dyn HubBackend> = Arc::new(devhub_core::demo::DemoHub::new());
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Msg>();
        let mut hub = Hub::new(backend, tx);
        hub.scan();
        let mut kb = None;
        let t = std::time::Instant::now();
        while kb.is_none() && t.elapsed() < Duration::from_secs(3) {
            if let Ok(m) = rx.try_recv() {
                if let Msg::Scanned { res: Ok(ds), .. } = &m {
                    kb = ds
                        .iter()
                        .find(|d| d.kind == DeviceKind::Keyboard)
                        .map(|d| d.id.clone());
                }
                hub.handle(m);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let id = kb.expect("demo keyboard");
        hub.open(&id, false);
        assert!(drain_until(
            &mut hub,
            &mut rx,
            Duration::from_secs(5),
            |h| h.session(&id).is_some_and(|s| s.ready())
        ));

        // Issue read A (gen=0) → submit a change → issue read B (gen=1).
        hub.reload(&id);
        let stale = hub.backend.driver(&id).unwrap().read().unwrap();
        hub.apply(
            &id,
            Change::ProfileName {
                index: 0,
                name: "RENAMED".into(),
            },
        );
        hub.reload(&id);

        // A's late response (gen=0) arrives — the change is pending, so it is layered on and kept.
        hub.handle(Msg::State {
            id: id.clone(),
            gen: 0,
            state: Box::new(stale),
        });
        assert_eq!(
            hub.session(&id)
                .unwrap()
                .state
                .as_ref()
                .unwrap()
                .profile
                .names[0],
            "RENAMED",
            "a stale read response rolled back a pending edit"
        );
        drain_until(&mut hub, &mut rx, Duration::from_secs(5), |h| {
            h.session(&id).is_some_and(|s| s.pending_changes.is_empty())
        });
        assert_eq!(
            hub.session(&id)
                .unwrap()
                .state
                .as_ref()
                .unwrap()
                .profile
                .names[0],
            "RENAMED"
        );
    }

    #[test]
    fn state_compatible_rejects_other_devices_and_flags_unknown_keys() {
        let (_, caps, st) = demo_kb();
        // Keyboard state into mouse caps → cannot apply.
        let mouse_caps = devhub_core::demo::mouse_caps(false, false);
        assert!(state_compatible(&mouse_caps, &st).is_err());
        // Same caps → usable as is.
        assert_eq!(state_compatible(&caps, &st).unwrap(), None);
        // Unknown setting ids are counted as warnings (ignored).
        let mut extra = st.clone();
        extra
            .settings
            .insert("nonexistent".into(), SettingValue::Bool(true));
        assert!(matches!(state_compatible(&caps, &extra), Ok(Some(_))));
        // EQ data to a device without EQ support → cannot apply.
        let mut peq = st.clone();
        peq.peq = Some(vec![PeqBand {
            freq: 1000,
            q: 1.,
            gain: 0.,
            filter: PeqFilter::Peak,
        }]);
        assert!(state_compatible(&caps, &peq).is_err());
    }
}
