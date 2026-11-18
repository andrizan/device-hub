//! mouse.fit dongle path — 16B report-8 + EEPROM protocol.
//!
//! On Windows, the 8K receiver (33e4:3854, usage_page 0xFF02 / usage 2) has no
//! feature report, so the 64B feature-report protocol (mousefit.rs) cannot be
//! used. Instead use the report-8/EEPROM path verified in legacy
//! (equivalent of the JS `Die` class):
//!
//! - output report id 8: `[op, 0, arg…, cksum]` (16B, sum17 ≡ 0x55)
//! - input report id 8: `[op echo, status, arg…, cksum]` (17B)
//! - EEPROM: read op8 `[8,0,addrHi,addrLo,len]` → data in recv[5..] (≤10B per call)
//!   write op7 `[7,0,addrHi,addrLo,len,data…]` (10B chunks)
//!   Scalars are 2B pairs of `(v, 0x55-v)`.
//! - While the mouse is asleep the response data part is all 0 (only the dongle responds) → info.asleep.
//!
//! EEPROM addresses come from `Ge` (mf-enums.txt):
//! ReportRate=0, maxDpiStage=2, CurrentDPI=4, KeyOperation=8, LOD=10,
//! DPIValue=12 (4B×stage, 3950 family), DPIColor=44 (4B×stage), KeyFunction=96 (4B×slot),
//! ShortcutKey=256 (32B×slot), Macro=768 (384B/slot: name@0, ctx@31),
//! DebounceTime=169, MotionSync=171, SleepTime=173, Angle=175, Ripple=177,
//! PerformanceState=181, Performance=183, AngleTune=189, AngleTuneState=191,
//! SensorFPS20K=225, WheelDebounceTime=227, DebounceReleaseTime=229,
//! LeftTrigger=239, LeftFastTrigger=241, LeftTactileFeedback=243,
//! RightTrigger=245, RightFastTrigger=247, RightTactileFeedback=249,
//! Sensor3955DPI=6912 (6B×stage, 3955 family), SensorVirtualPosEnable=6984,
//! SensorVirtualPos=6986.
//!
//! Other ops: 1=EID/donglePR, 4=battery, 5=enterPairMode, 6=pairResult,
//! 9=ClearSetting, 14=getProfileID, 15=setProfileID, 18=FW, 22/23=long-range mode,
//! 29=FW (per dev), 46=StartCalibr.

use devhub_core::backend::*;
use devhub_core::model::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

const RID: u8 = 8;
/// EEPROM data length readable with one command (recv[5..15]).
const CHUNK: u16 = 10;

#[allow(dead_code)]
mod ee {
    pub const REPORT_RATE: u16 = 0;
    pub const MAX_DPI_STAGE: u16 = 2;
    pub const CURRENT_DPI: u16 = 4;
    pub const KEY_OP: u16 = 8;
    pub const LOD: u16 = 10;
    pub const DPI_VALUE: u16 = 12;
    pub const DPI_COLOR: u16 = 44;
    pub const DPI_EFFECT_MODE: u16 = 76;
    pub const DPI_EFFECT_STATE: u16 = 82;
    pub const KEY_FUNCTION: u16 = 96;
    pub const DEBOUNCE: u16 = 169;
    pub const MOTION_SYNC: u16 = 171;
    pub const SLEEP: u16 = 173;
    pub const ANGLE: u16 = 175;
    pub const RIPPLE: u16 = 177;
    pub const PERF_STATE: u16 = 181;
    pub const PERF: u16 = 183;
    pub const ANGLE_TUNE: u16 = 189;
    pub const ANGLE_TUNE_STATE: u16 = 191;
    pub const SENSOR_FPS20K: u16 = 225;
    pub const WHEEL_DEBOUNCE: u16 = 227;
    pub const DEBOUNCE_RELEASE: u16 = 229;
    pub const LEFT_TRIGGER: u16 = 239;
    pub const LEFT_FAST: u16 = 241;
    pub const LEFT_TACTILE: u16 = 243;
    pub const RIGHT_TRIGGER: u16 = 245;
    pub const RIGHT_FAST: u16 = 247;
    pub const RIGHT_TACTILE: u16 = 249;
    pub const SHORTCUT: u16 = 256;
    pub const MACRO: u16 = 768;
    pub const DPI_3955: u16 = 6912;
    pub const SENSOR_VPOS_EN: u16 = 6984;
    pub const SENSOR_VPOS: u16 = 6986;
}

// ==================================================================== model table

/// Per-model differences behind the dongle (from env-models.json). The inner PID itself cannot be
/// obtained over report-8 (the JS GetDevicePID is a stub), but the op1 EID (= env-models MID) can
/// determine the model — `maybe_detect` settles it on open / after wake-up and, if it differs from the registry default
/// (HTS Plus Pro), swaps the model/caps/display name.
pub struct R8Model {
    /// 3955 → DPI uses 6B records @6912, step=1. Otherwise → 4B @12, step=50.
    /// (The official Die class always uses sensorType "3950" and @12 for every model, but
    /// reads and writes at @6912 are verified to work on a real JM_052_3955 too.)
    pub sensor_3955: bool,
    pub dpi_max: u32,
    pub max_stages: usize,
    /// LOD choices (display mm ×10). The wire codes are in `lod_wire` (same order).
    pub lod_choices: &'static [(i64, &'static str)],
    /// LOD wire codes (arguments of the official setLOD). Reads do a reverse lookup in this table.
    /// JM_056_3950 uses {0.7→3, 1→1, 2→2}, HTS/JM_056_3955 use index+1,
    /// GW_056 uses {0.7→0x87, 1→1, 2→2} (official read: s<128→mm, 128+ → (s-128)/10).
    pub lod_wire: &'static [u8],
    /// Upper limit of click debounce in ms (ButtonDebounceTimeMax).
    pub debounce_max: u8,
    /// Magnetic micro switches (ButtonType=1) → the trigger-related EEPROM is enabled.
    pub magnetic: bool,
    /// SPDT toggle (SPDTStateEnable). The Die family uses EEPROM @KeyOperation bit0/1.
    pub spdt: bool,
    pub long_range: bool,
    /// Sensor angle (AngleTune) — official SensorAngleEnable.
    pub sensor_angle: bool,
    /// Button count (BtnMaxNum).
    pub btn_num: usize,
    /// Profile count (1 if unknown).
    pub profiles: u8,
    /// mouse.fit Config folder name (FWLINK.txt → firmware update notification).
    pub ui_folder: &'static str,
}

const LOD_JM: &[(i64, &str)] = &[
    (7, "0.7mm"),
    (9, "0.9mm"),
    (12, "1.2mm"),
    (14, "1.4mm"),
    (16, "1.6mm"),
];
const LOD_GW: &[(i64, &str)] = &[(7, "0.7mm"), (10, "1.0mm"), (20, "2.0mm")];

/// HTS Plus Pro (JM_052_3955, ButtonType=1).
const HTS_PLUS_PRO: R8Model = R8Model {
    sensor_3955: true,
    dpi_max: 40_000,
    max_stages: 7,
    lod_choices: LOD_JM,
    lod_wire: &[1, 2, 3, 4, 5],
    debounce_max: 30,
    magnetic: true,
    spdt: false,
    long_range: true,
    sensor_angle: true,
    btn_num: 5,
    profiles: 1,
    ui_folder: "JM_052_3955",
};
/// HTX Ultra first generation (GW_056, XVI=1). The official app uses the feature-report (cm) path, so
/// the report-8 settings path is unverified — the rows we expose are limited to the env Enable flags.
const HTX_ULTRA: R8Model = R8Model {
    sensor_3955: false,
    dpi_max: 30_000,
    max_stages: 7,
    lod_choices: LOD_GW,
    lod_wire: &[0x87, 1, 2],
    debounce_max: 50,
    magnetic: false,
    spdt: false, // GW's SPDT is a cm (feature report) command — unconfirmed over report-8
    long_range: false,
    sensor_angle: true,
    btn_num: 5,
    profiles: 1,
    ui_folder: "GW_056",
};
/// HTX Ultra 3950 (JM_056_3950, PIDWired=5618). Verified on a real device over Die/report-8.
/// SPDT @8 bit0/1 and LOD wire {0.7→3, 1→1, 2→2} are taken as-is from the official JS write map.
const HTX_ULTRA_3950: R8Model = R8Model {
    sensor_3955: false,
    dpi_max: 32_000,
    max_stages: 7,
    lod_choices: LOD_GW,
    lod_wire: &[3, 1, 2],
    debounce_max: 30,
    magnetic: false,
    spdt: true,
    long_range: true,
    sensor_angle: true,
    btn_num: 5,
    profiles: 1,
    ui_folder: "JM_056_3950",
};
/// HTX Ultra 3955 (JM_056_3955, PIDWired=5619). Not connected — built from the env flags.
const HTX_ULTRA_3955: R8Model = R8Model {
    sensor_3955: false, // the official Die uses @12/4B/step50 for 3955 too
    dpi_max: 40_000,
    max_stages: 7,
    lod_choices: LOD_JM,
    lod_wire: &[1, 2, 3, 4, 5],
    debounce_max: 30,
    magnetic: false,
    spdt: true,
    long_range: true,
    sensor_angle: true,
    btn_num: 5,
    profiles: 1,
    ui_folder: "JM_056_3955",
};

fn model_for(model: &str) -> &'static R8Model {
    match model {
        "htx-ultra" => &HTX_ULTRA,
        "htx-ultra-3950" => &HTX_ULTRA_3950,
        "htx-ultra-3955" => &HTX_ULTRA_3955,
        _ => &HTS_PLUS_PRO,
    }
}

/// Inner model ID of the dongle (op1 → recv[10] = MID in env-models).
/// The 3854 dongle is shared by 8 models, so resolve the model/display name by MID
/// (the official HUB does the same via `getEID()` → `a(MID)`).
/// EID=0 is the value while the mouse is asleep — it cannot be used to determine the model.
fn mid_lookup(mid: u8) -> Option<(&'static str, &'static str)> {
    Some(match mid {
        5 => ("htx-ultra-3950", "HTX Ultra"),
        7 => ("htx-ultra-3955", "HTX Ultra (3955)"),
        11 => ("hts-plus-pro", "HTS Plus Pro"),
        _ => return None,
    })
}

// ==================================================================== driver

/// Command responses and spontaneous notifications are mixed in the same input report, so all reports are
/// read by a single receive thread; op echoes go to `pending` and notifications (op10 / report4) go to
/// notification handling (`docs/NEXT_TASKS.md` §2.3).
struct Shared {
    dev: Mutex<hidapi::HidDevice>,
    /// op → response sink (only one can be outstanding at a time — there is a single worker).
    pending: Mutex<HashMap<u8, mpsc::Sender<Vec<u8>>>>,
    /// Latest op10 magnetic-button notification (L%, R%). Written only by the receive thread.
    depth: Mutex<(u8, u8)>,
    info: Mutex<DeviceInfo>,
    /// Time the last input report was received (link liveness heartbeat).
    last_pkt: Mutex<Instant>,
    /// Time of the most recent spontaneous input report (op10 / report-4 etc., excluding command responses).
    /// For computing the measured polling rate — written only by the receive thread.
    rate: Mutex<std::collections::VecDeque<Instant>>,
    /// Whether the battery re-read thread started from a notification is running (prevents duplicate starts).
    bat_busy: AtomicBool,
    /// Consecutive command timeout count (fail-fast — stops the sweep of
    /// all read() commands against an unresponsive device early).
    fails: std::sync::atomic::AtomicU32,
    dead_since: Mutex<Option<Instant>>,
    stop: AtomicBool,
    events: Option<Arc<dyn Fn(HubEvent) + Send + Sync>>,
}

pub struct MfDongleDriver {
    sh: Arc<Shared>,
    /// May change until the inner model is determined (dongle: registry default while no EID is detected).
    model: Mutex<&'static R8Model>,
    caps: Mutex<Capabilities>,
    /// Whether the EID (MID) has been determined on a dongle.
    detected: AtomicBool,
    /// Shared map that writes the determined (model, display name) back to the Hub side —
    /// keeps it from reverting to the registry default when the driver is recreated.
    detect_sink: Arc<Mutex<HashMap<String, (String, String)>>>,
    /// Cache of the magnetic-button actuation point EEPROM values (L, R).
    /// Do not store (0,0) after a failed read — a false actuation line would appear at the bottom.
    trig: Mutex<Option<(u8, u8)>>,
    /// Cooldown for re-reading `trig` (if it could not be read while asleep, retry on the next live poll).
    trig_retry: Mutex<std::time::Instant>,
    events: Option<Arc<dyn Fn(HubEvent) + Send + Sync>>,
}

impl MfDongleDriver {
    fn model(&self) -> &'static R8Model {
        *self.model.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// op1 (f[4]=8 + nonce) → EID (= env-models MID). No response / 0 means undetermined.
    /// The official `getEID` uses `receivedData[10]` (the array without the report id)
    /// = `r[11]` in our 17B response. Verified on a real device: r[11]=5 for the HTXU-3950 pair.
    fn read_mid(&self) -> Option<u8> {
        let mut f = [0u8; 16];
        f[0] = 1;
        f[4] = 8;
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0xA5);
        f[5] = n as u8;
        f[6] = (n >> 8) as u8;
        f[7] = (n >> 16) as u8;
        f[8] = (n >> 24) as u8;
        let r = self.cmd(&f).ok()?;
        (r[11] != 0).then_some(r[11])
    }

    /// Dongle: once the EID is obtained, determine the model and, if it differs from the registry default,
    /// update model/caps/info and notify the UI with `HubEvent::Info`.
    /// If EID=0 (asleep), stay undetermined and retry next time.
    fn maybe_detect(&self) {
        if self.detected.load(Ordering::Relaxed)
            || self
                .sh
                .info
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .transport
                != Transport::Wireless24G
        {
            return;
        }
        let Some(mid) = self.read_mid() else { return };
        self.detected.store(true, Ordering::Relaxed);
        let Some((model_str, name)) = mid_lookup(mid) else {
            return;
        };
        let new = model_for(model_str);
        // Also record the result on the Hub side — it is restored when the driver is recreated.
        {
            let id = self
                .sh
                .info
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .id
                .0
                .clone();
            self.detect_sink
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(id, (model_str.into(), name.into()));
        }
        if std::ptr::eq(new, self.model()) {
            return;
        }
        *self.model.lock().unwrap_or_else(|p| p.into_inner()) = new;
        let info = {
            let mut info = self.sh.info.lock().unwrap_or_else(|p| p.into_inner());
            info.model = model_str.into();
            info.name = name.into();
            info.clone()
        };
        *self.caps.lock().unwrap_or_else(|p| p.into_inner()) = build_caps(&info, new);
        if new.magnetic {
            let l = self.ee_u8(ee::LEFT_TRIGGER).ok();
            let r = self.ee_u8(ee::RIGHT_TRIGGER).ok();
            *self.trig.lock().unwrap_or_else(|p| p.into_inner()) = l.zip(r);
        } else {
            *self.trig.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
        if let Some(ev) = &self.events {
            ev(HubEvent::Info(info.id.clone(), Box::new(info)));
        }
    }
}

impl Drop for MfDongleDriver {
    fn drop(&mut self) {
        self.sh.stop.store(true, Ordering::Relaxed);
    }
}

pub fn open(
    dev: hidapi::HidDevice,
    info: DeviceInfo,
    events: Option<Arc<dyn Fn(HubEvent) + Send + Sync>>,
    detect_sink: Arc<Mutex<HashMap<String, (String, String)>>>,
) -> HubResult<MfDongleDriver> {
    dev.set_blocking_mode(false).ok();
    let model = model_for(&info.model);
    let is_dongle = info.transport == Transport::Wireless24G;
    let id = info.id.clone();
    let sh = Arc::new(Shared {
        dev: Mutex::new(dev),
        pending: Mutex::new(HashMap::new()),
        depth: Mutex::new((0, 0)),
        info: Mutex::new(info),
        // Treat it as "quiet" from right after startup — so retries can be limited from the first command
        // to a mouse that is already asleep when opened.
        last_pkt: Mutex::new(Instant::now() - Duration::from_secs(60)),
        rate: Mutex::new(std::collections::VecDeque::new()),
        bat_busy: AtomicBool::new(false),
        fails: std::sync::atomic::AtomicU32::new(0),
        dead_since: Mutex::new(None),
        stop: AtomicBool::new(false),
        events: events.clone(),
    });
    let d = MfDongleDriver {
        caps: Mutex::new(build_caps(
            &sh.info.lock().unwrap_or_else(|p| p.into_inner()),
            model,
        )),
        model: Mutex::new(model),
        detected: AtomicBool::new(false),
        detect_sink,
        sh: sh.clone(),
        trig: Mutex::new(None),
        // Let the first attempt through right away (unused if already initialized).
        trig_retry: Mutex::new(std::time::Instant::now() - Duration::from_secs(3)),
        events,
    };
    // Receive thread: routes responses and notifications.
    {
        let sh = sh.clone();
        let events = d.events.clone();
        std::thread::Builder::new()
            .name("mf8-reader".into())
            .spawn(move || reader(sh, events, id))
            .ok();
    }
    // A dongle determines the inner model from the op1 EID (MID) (settled here if a response arrives,
    // while asleep it is retried after wake-up on the read() side).
    if is_dongle {
        d.maybe_detect();
    }
    // Read and cache the initial actuation points (proceed with None even on failure — 0 would produce a false line).
    if d.model().magnetic {
        let l = d.ee_u8(ee::LEFT_TRIGGER).ok();
        let r = d.ee_u8(ee::RIGHT_TRIGGER).ok();
        *d.trig.lock().unwrap_or_else(|p| p.into_inner()) = l.zip(r);
    }
    Ok(d)
}

/// Declare the link dead after this many consecutive timeouts (do not stall the whole read()).
const FAIL_FAST_AFTER: u32 = 4;
/// Interval for checking liveness with a real command while declared dead.
const PROBE_MS: u64 = 1500;

/// 16B command → 17B response (op echo + cksum verified). Up to 3 tries.
/// The response itself is delivered by the receive thread via `pending`.
fn link_cmd(sh: &Shared, frame: &[u8; 16]) -> HubResult<[u8; 17]> {
    let op = frame[0];
    // fail-fast: Disconnected immediately during consecutive timeouts. Once every PROBE_MS,
    // let one real command through to check for recovery.
    if sh.fails.load(Ordering::Relaxed) >= FAIL_FAST_AFTER {
        let mut ds = sh.dead_since.lock().unwrap_or_else(|p| p.into_inner());
        match *ds {
            Some(t) if t.elapsed() < Duration::from_millis(PROBE_MS) => {
                return Err(HubError::Disconnected)
            }
            _ => *ds = Some(Instant::now()),
        }
    }
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    sh.pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(op, tx);
    let res = (|| {
        let mut w = [0u8; 17];
        w[0] = RID;
        w[1..17].copy_from_slice(frame);
        let s: u32 = w[..16].iter().map(|&b| b as u32).sum();
        w[16] = 85u32.wrapping_sub(s) as u8;
        // Limit retries to 1 for a link known to be asleep / with no recent reception
        // (a full retry for every setting during read() while asleep takes tens of seconds).
        let quiet = sh
            .last_pkt
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed()
            > Duration::from_secs(5);
        let tries = if quiet || sh.info.lock().unwrap_or_else(|p| p.into_inner()).asleep {
            1
        } else {
            3
        };
        for _ in 0..tries {
            sh.dev
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .write(&w)
                .map_err(|e| HubError::Io(format!("mf8 write: {e}")))?;
            let deadline = Instant::now() + Duration::from_millis(400);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                match rx.recv_timeout(left.min(Duration::from_millis(120))) {
                    Ok(v) if v.len() >= 17 && v[1] == op => {
                        let mut out = [0u8; 17];
                        out.copy_from_slice(&v[..17]);
                        sh.fails.store(0, Ordering::Relaxed);
                        *sh.dead_since.lock().unwrap_or_else(|p| p.into_inner()) = None;
                        return Ok(out);
                    }
                    Ok(_) => continue,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if Instant::now() >= deadline {
                            break;
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            sleep(Duration::from_millis(20));
        }
        // Measured: while asleep it is completely unresponsive rather than returning an "all-0 response".
        // Full command timeout + silence on all packets = judged asleep.
        // (If notifications arrived recently and only one attempt failed, treat it as transient.)
        let quiet = sh
            .last_pkt
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed()
            > Duration::from_secs(5);
        if quiet {
            set_asleep(sh, true);
        }
        let f = sh.fails.fetch_add(1, Ordering::Relaxed) + 1;
        if f >= FAIL_FAST_AFTER {
            *sh.dead_since.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
            return Err(HubError::Disconnected);
        }
        Err(HubError::Protocol("mf8: response timeout".into()))
    })();
    sh.pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&op);
    res
}

/// Receive thread body. Reads all input reports and routes them.
fn reader(sh: Arc<Shared>, events: Option<Arc<dyn Fn(HubEvent) + Send + Sync>>, id: DeviceId) {
    let mut buf = [0u8; 65];
    while !sh.stop.load(Ordering::Relaxed) {
        let r = sh
            .dev
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .read_timeout(&mut buf, 30);
        // If dev is held while read_timeout is blocking,
        // the repeated release → immediate re-acquire starves the link_cmd write
        // (measured: over 8 seconds for a single battery command). Leave a gap between iterations
        // so the lock is always handed to the write side.
        std::thread::sleep(Duration::from_millis(1));
        // Something arrived = the link is alive. Record the heartbeat and mark it awake
        // (for this device, asleep means it stops sending any notifications or responses at all).
        if matches!(r, Ok(n) if n > 0) {
            *sh.last_pkt.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
            set_asleep(&sh, false);
        }
        match r {
            Ok(n) if n >= 17 && buf[0] == RID => {
                let op = buf[1];
                if op == 10 {
                    rate_tick(&sh);
                    notify_op10(&buf[..n], &sh, &events, &id);
                } else if let Some(tx) = sh
                    .pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&op)
                {
                    let _ = tx.send(buf[..17].to_vec());
                }
            }
            // Notification on report id 4. `04 03 <pct>` = battery, `04 06 00/01` = disconnect/connect,
            // others (01/04 DPI, 02 profile, 07 LOD, 08 polling, 0f MotionSync,
            // 18 sensor) = something changed on the device itself → re-read.
            // The battery notification carries the value itself, so apply it directly without sending a command
            // (calling link_cmd on the receive thread would block, as it cannot read its own response).
            Ok(n) if n >= 3 && buf[0] == 4 => {
                rate_tick(&sh);
                match buf[1] {
                    3 => {
                        set_asleep(&sh, false);
                        let b = {
                            let mut i = sh.info.lock().unwrap_or_else(|p| p.into_inner());
                            let b = Battery {
                                percent: Some(buf[2].min(100)),
                                charging: i.battery.is_some_and(|b| b.charging),
                            };
                            i.battery = Some(b);
                            b
                        };
                        if let Some(ev) = &events {
                            ev(HubEvent::Battery(id.clone(), b));
                        }
                    }
                    6 => {
                        let connected = buf[2] == 1;
                        set_asleep(&sh, !connected);
                        if connected {
                            if let Some(ev) = &events {
                                ev(HubEvent::StateChanged(id.clone()));
                            }
                        }
                    }
                    _ => {
                        if let Some(ev) = &events {
                            ev(HubEvent::StateChanged(id.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Counts one spontaneous input report (a deque of the last 1 second).
fn rate_tick(sh: &Arc<Shared>) {
    let mut q = sh.rate.lock().unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    q.push_back(now);
    while q
        .front()
        .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1))
    {
        q.pop_front();
    }
}

/// Measured report rate over the last 1 second (Hz).
fn report_hz(sh: &Arc<Shared>) -> Option<f32> {
    let q = sh.rate.lock().unwrap_or_else(|p| p.into_inner());
    Some(q.len() as f32)
}

/// op10 notification (r[6] = kind). Magnetic-button kinds store the depth; others go to events.
/// Note: this runs on the receive thread. Calling `link_cmd` directly would block, as it cannot read
/// its own response, so always hand off to another thread when a command is needed.
fn notify_op10(
    r: &[u8],
    sh: &Arc<Shared>,
    events: &Option<Arc<dyn Fn(HubEvent) + Send + Sync>>,
    id: &DeviceId,
) {
    match r.get(6).copied().unwrap_or(0xFF) {
        // Magnetic button depth: r[14]&127 = left %, r[15]&127 = right %.
        0 | 4 | 8 | 16 | 32 if r.len() > 15 => {
            *sh.depth.lock().unwrap_or_else(|p| p.into_inner()) = (r[14] & 127, r[15] & 127);
        }
        // Battery change → re-read on another thread and emit an event (duplicates are limited to one).
        64 => {
            if let Some(ev) = events {
                if !sh.bat_busy.swap(true, Ordering::SeqCst) {
                    let (sh, ev, id) = (sh.clone(), ev.clone(), id.clone());
                    std::thread::spawn(move || {
                        if let Ok(b) = battery_of(&sh) {
                            ev(HubEvent::Battery(id, b));
                        }
                        sh.bat_busy.store(false, Ordering::SeqCst);
                    });
                }
            }
        }
        // 1=DPI change / 2=polling change → have the state re-read.
        1 | 2 => {
            if let Some(ev) = events {
                ev(HubEvent::StateChanged(id.clone()));
            }
        }
        _ => {}
    }
}

/// Changes `info.asleep` and sends `HubEvent::Info` only when it changed.
fn set_asleep(sh: &Shared, asleep: bool) {
    let mut i = sh.info.lock().unwrap_or_else(|p| p.into_inner());
    if i.asleep == asleep {
        return;
    }
    i.asleep = asleep;
    let info = i.clone();
    drop(i);
    if let Some(ev) = &sh.events {
        ev(HubEvent::Info(info.id.clone(), Box::new(info)));
    }
}

/// Battery read via the op4 command (also used via the link and from notification handling).
fn battery_of(sh: &Shared) -> HubResult<Battery> {
    let mut f = [0u8; 16];
    f[0] = 4;
    let r = link_cmd(sh, &f)?;
    if r[6] == 0 && r[7] == 0 && r[8] == 0 {
        set_asleep(sh, true);
        return sh
            .info
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .battery
            .ok_or_else(|| HubError::Protocol("mouse asleep".into()));
    }
    set_asleep(sh, false);
    let b = Battery {
        percent: Some(r[6].min(100)),
        charging: r[7] != 0,
    };
    sh.info.lock().unwrap_or_else(|p| p.into_inner()).battery = Some(b);
    Ok(b)
}

impl MfDongleDriver {
    fn cmd(&self, frame: &[u8; 16]) -> HubResult<[u8; 17]> {
        link_cmd(&self.sh, frame)
    }

    /// EEPROM read (10B chunks, joined internally). Data is in recv[5..].
    fn ee_read(&self, addr: u16, len: u16) -> HubResult<Vec<u8>> {
        let mut out = Vec::with_capacity(len as usize);
        let mut a = addr;
        while out.len() < len as usize {
            let want = (len - out.len() as u16).min(CHUNK) as u8;
            let mut f = [0u8; 16];
            f[0] = 8;
            f[2] = (a >> 8) as u8;
            f[3] = (a & 0xff) as u8;
            f[4] = want;
            let r = self.cmd(&f)?;
            if r[2] == 0 && r[6..16].iter().all(|&b| b == 0) {
                // All-0 data = the mouse is asleep. Return the zeros as-is (the caller verifies the pair).
                out.extend_from_slice(&[0u8; CHUNK as usize][..want as usize]);
            } else {
                out.extend_from_slice(&r[6..6 + want as usize]);
            }
            a += want as u16;
        }
        Ok(out)
    }

    /// EEPROM write (10B chunks).
    fn ee_write(&self, addr: u16, data: &[u8]) -> HubResult<()> {
        for (i, chunk) in data.chunks(CHUNK as usize).enumerate() {
            let a = addr + (i * CHUNK as usize) as u16;
            let mut f = [0u8; 16];
            f[0] = 7;
            f[2] = (a >> 8) as u8;
            f[3] = (a & 0xff) as u8;
            f[4] = chunk.len() as u8;
            f[5..5 + chunk.len()].copy_from_slice(chunk);
            self.cmd(&f)?;
            sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Scalar: (v, 0x55-v) pair write.
    fn ee_write_val(&self, addr: u16, val: u8) -> HubResult<()> {
        self.ee_write(addr, &[val, 85u8.wrapping_sub(val)])
    }

    /// Scalar read. Err if the pair is invalid (sum≠85).
    fn ee_u8(&self, addr: u16) -> HubResult<u8> {
        let b = self.ee_read(addr, 2)?;
        if (b[0] as u16 + b[1] as u16) & 0xff == 85 {
            Ok(b[0])
        } else {
            Err(HubError::Protocol(format!("eeprom @{addr}: bad pair")))
        }
    }

    /// CRC of a 4B/6B record: `85 - sum(b[..last])`.
    fn crc(b: &[u8]) -> u8 {
        85u8.wrapping_sub(b[..b.len() - 1].iter().map(|&x| x as u32).sum::<u32>() as u8)
    }

    fn mark_asleep(&self, asleep: bool) {
        set_asleep(&self.sh, asleep);
    }

    // ---- DPI stages ------------------------------------------------
    /// EEPROM address and record length of a stage.
    fn stage_addr(&self, i: usize) -> (u16, usize) {
        if self.model().sensor_3955 {
            (ee::DPI_3955 + (i * 6) as u16, 6)
        } else {
            (ee::DPI_VALUE + (i * 4) as u16, 4)
        }
    }

    fn parse_stage(&self, b: &[u8]) -> (u32, u32) {
        parse_stage(self.model().sensor_3955, b)
    }

    fn encode_stage(&self, x: u32, y: u32) -> Vec<u8> {
        encode_stage(self.model().sensor_3955, x, y)
    }

    // ---- buttons (KeyFunction @96) ------------------------------------
    /// record → ButtonAction. type5 (ShortcutKey) reads a separate area separately.
    fn decode_keyfn(&self, slot: usize, b: &[u8]) -> ButtonAction {
        if b.first() == Some(&5) {
            return self.read_shortcut(slot).unwrap_or(ButtonAction::Disabled);
        }
        decode_keyfn_basic(b)
    }

    /// ButtonAction → KeyFunction record + whether a write to the ShortcutKey area is needed.
    fn encode_keyfn(&self, slot: usize, a: &ButtonAction) -> HubResult<([u8; 4], Option<Vec<u8>>)> {
        use ButtonAction::*;
        let rec = |ty: u8, ph: u8, pl: u8| {
            let mut r = [ty, ph, pl, 0];
            r[3] = Self::crc(&r);
            r
        };
        let _ = slot;
        Ok(match a {
            Disabled => (rec(0, 0, 0), None),
            Mouse(MouseFn::DoubleClick) => (rec(4, 100, 2), None),
            Mouse(MouseFn::ScrollUp) => (rec(11, 1, 0), None),
            Mouse(MouseFn::ScrollDown) => (rec(11, 2, 0), None),
            Mouse(f) => {
                let mask = match f {
                    MouseFn::Left => 1,
                    MouseFn::Right => 2,
                    MouseFn::Middle => 4,
                    MouseFn::Back => 8,
                    MouseFn::Forward => 16,
                    _ => 1,
                };
                (rec(1, mask, 0), None)
            }
            Dpi(DpiFn::Cycle) => (rec(2, 1, 0), None),
            Dpi(DpiFn::Up) => (rec(2, 2, 0), None),
            Dpi(DpiFn::Down) => (rec(2, 3, 0), None),
            Dpi(DpiFn::Shift(d)) => (rec(10, 0, ((*d / 50).max(1) - 1) as u8), None),
            Fire {
                count, interval_ms, ..
            } => (rec(4, (*interval_ms).min(255) as u8, *count), None),
            Key(k) => (rec(5, 0, 0), Some(encode_shortcut(k))),
            Media(m) => (rec(5, 0, 0), Some(encode_media(*m))),
            Macro { slot: s, run } => {
                let t = match run {
                    MacroRun::Times(n) => (*n).min(252) as u8,
                    MacroRun::WhileHeld => 254,
                    MacroRun::Toggle => 253,
                };
                (rec(6, *s + 1, t), None)
            }
            PollingCycle => (rec(7, 0, 0), None),
            ProfileCycle => return Err(HubError::Unsupported("profile cycle (dongle)")),
            ScrollModeToggle => return Err(HubError::Unsupported("scroll-mode toggle")),
            Unsupported { .. } => return Err(HubError::Unsupported("raw action")),
        })
    }

    /// Reads the ShortcutKey area (32B) and restores it as KeyStroke/MediaFn.
    fn read_shortcut(&self, slot: usize) -> Option<ButtonAction> {
        let b = self.ee_read(ee::SHORTCUT + (slot * 32) as u16, 32).ok()?;
        let n = *b.first()? as usize;
        let mut mods = 0u8;
        let mut usage = 0u16;
        let mut media = None;
        for i in 0..n.min(10) {
            let o = 1 + i * 3;
            if o + 2 >= b.len() {
                break;
            }
            // Official Update_Mouse_ShortcutKey: type = state&15, value = v1|v2<<8.
            let t = b[o] & 0x0f;
            let v = b[o + 1] as u16 | ((b[o + 2] as u16) << 8);
            if b[o] & 0x80 != 0 {
                match t {
                    0 => mods |= v as u8,
                    1 => usage = v,
                    2 => media = media_of_code(v),
                    _ => {}
                }
            }
        }
        if let Some(m) = media {
            Some(ButtonAction::Media(m))
        } else if usage != 0 || mods != 0 {
            Some(ButtonAction::Key(KeyStroke { mods, usage }))
        } else {
            None
        }
    }

    // ---- macros (EEPROM @768, 384B/slot) -------------------------------
    fn macro_addr(slot: u8) -> u16 {
        ee::MACRO + (slot as u16) * 384
    }
    /// Reads the events of a slot. None if there are none.
    fn read_macro(&self, slot: u8) -> HubResult<Option<MacroDef>> {
        let base = Self::macro_addr(slot);
        let hdr = self.ee_read(base, 10)?;
        let name_len = hdr[0] as usize;
        let ctx0 = self.ee_read(base + 31, 10)?;
        let n = ctx0[0] as usize;
        if n == 0 || n > 70 {
            return Ok(None);
        }
        let need = (n * 5 + 2) as u16;
        let ctx = self.ee_read(base + 31, need)?;
        let mut events = Vec::with_capacity(n);
        for i in 0..n {
            let o = 1 + i * 5;
            if o + 4 >= ctx.len() {
                break;
            }
            let s = ctx[o];
            let (ty, status) = (s & 15, s >> 6);
            let value = ctx[o + 1] as u16 | ((ctx[o + 2] as u16) << 8);
            let delay = ((ctx[o + 3] as u32) << 8) | ctx[o + 4] as u32;
            let edge = if status == 2 {
                MacroEdge::Down
            } else {
                MacroEdge::Up
            };
            let input = match ty {
                1 => MacroInput::Mouse(match value & 0xff {
                    1 => MouseFn::Left,
                    2 => MouseFn::Right,
                    4 => MouseFn::Middle,
                    8 => MouseFn::Back,
                    16 => MouseFn::Forward,
                    _ => MouseFn::Left,
                }),
                9 | 10 => MacroInput::Key(0xE0 + (value as u8).trailing_zeros().min(7) as u16),
                _ => MacroInput::Key(value),
            };
            // type 9=mod down, 10=mod up: the type includes the edge
            let edge = if ty == 10 {
                MacroEdge::Up
            } else if ty == 9 {
                MacroEdge::Down
            } else {
                edge
            };
            events.push(MacroEvent {
                input,
                edge,
                delay_ms: delay,
            });
        }
        let name = if name_len > 0 && name_len <= 30 {
            let nb = self
                .ee_read(base + 1, name_len.min(30) as u16)
                .unwrap_or_default();
            String::from_utf8_lossy(&nb).into_owned()
        } else {
            format!("Macro {}", slot + 1)
        };
        Ok(Some(MacroDef {
            name,
            events,
            mode: 0,
            repeat: 1,
        }))
    }

    fn write_macro(&self, slot: u8, def: &MacroDef) -> HubResult<()> {
        let base = Self::macro_addr(slot);
        let mut a = vec![def.events.len().min(70) as u8];
        for ev in def.events.iter().take(70) {
            let (ty, value, edge) = match ev.input {
                MacroInput::Key(u) if (0xE0..=0xE7).contains(&u) => {
                    (9, 1u16 << (u - 0xE0), ev.edge)
                }
                MacroInput::Key(u) => (2, u, ev.edge),
                MacroInput::Mouse(f) => {
                    let mask = match f {
                        MouseFn::Left => 1,
                        MouseFn::Right => 2,
                        MouseFn::Middle => 4,
                        MouseFn::Back => 8,
                        MouseFn::Forward => 16,
                        _ => 1,
                    };
                    (1, mask, ev.edge)
                }
            };
            // type 9/10 are mod down/up, with the edge embedded in the type
            let ty = if ty == 9 && edge == MacroEdge::Up {
                10
            } else {
                ty
            };
            let status: u8 = if ty == 9 || ty == 10 {
                // 9/10 have the edge built in — keep the status bit consistent too
                if ty == 9 {
                    2
                } else {
                    1
                }
            } else if edge == MacroEdge::Down {
                2
            } else {
                1
            };
            a.push((status << 6) | ty);
            a.push((value & 0xff) as u8);
            a.push((value >> 8) as u8);
            a.push((ev.delay_ms >> 8) as u8);
            a.push((ev.delay_ms & 0xff) as u8);
        }
        a.push(0);
        let last = a.len() - 1;
        a[last] = 85u8.wrapping_sub(a[..last].iter().map(|&x| x as u32).sum::<u32>() as u8);
        self.ee_write(base + 31, &a)
    }

    fn clear_macro(&self, slot: u8) -> HubResult<()> {
        let mut a = vec![0u8, 0];
        a[1] = 85; // crc of [0]
        self.ee_write(Self::macro_addr(slot) + 31, &a)
    }
}

// ==================================================================== encoding helpers

/// ShortcutKey: the official app writes `[count][press(type|0x80)][release(reverse order, type|0x40)][crc]`
/// compactly (crc = `85 - sum(everything before it)`, @`1+3*count`). Real-device dump:
/// `02 81 1f 00 41 1f 00 53` (key usage31). Modifiers are type0/value=mod flags,
/// keys are type1/usage, and the modifier press order is the same as the official UI: [Ctrl,Alt,Shift,Win,(right side)].
fn encode_shortcut(k: &KeyStroke) -> Vec<u8> {
    let mut ev: Vec<[u8; 3]> = vec![];
    for bit in [0u8, 2, 1, 3, 4, 5, 6, 7] {
        if k.mods >> bit & 1 == 1 {
            ev.push([0x80, 1 << bit, 0]);
        }
    }
    // The key itself only when usage!=0 (official: a modifier-only binding has modifier entries only).
    if k.usage != 0 {
        ev.push([0x81, (k.usage & 0xff) as u8, (k.usage >> 8) as u8]);
    }
    // release is the reverse order of press (official JS: |0x40 with h[len-1-u]).
    let mut b = vec![(ev.len() * 2) as u8];
    for e in &ev {
        b.extend_from_slice(e);
    }
    for e in ev.iter().rev() {
        b.extend_from_slice(&[e[0] & !0x80 | 0x40, e[1], e[2]]);
    }
    let crc = 85u32.wrapping_sub(b.iter().map(|&x| x as u32).sum::<u32>()) as u8;
    b.push(crc);
    b
}

fn media_code(m: MediaFn) -> u16 {
    match m {
        MediaFn::PlayPause => 0xCD,
        MediaFn::Stop => 0xB7,
        MediaFn::Next => 0xB5,
        MediaFn::Prev => 0xB6,
        MediaFn::VolumeUp => 0xE9,
        MediaFn::VolumeDown => 0xEA,
        MediaFn::Mute => 0xE2,
        MediaFn::Calculator => 0x192,
        MediaFn::MyComputer => 0x194,
        MediaFn::Browser => 0x196,
        MediaFn::Mail => 0x18A,
        MediaFn::BrightnessUp => 0x6F,
        MediaFn::BrightnessDown => 0x70,
        MediaFn::Refresh => 0x7A,
    }
}
fn media_of_code(c: u16) -> Option<MediaFn> {
    Some(match c {
        0xCD => MediaFn::PlayPause,
        0xB7 => MediaFn::Stop,
        0xB5 => MediaFn::Next,
        0xB6 => MediaFn::Prev,
        0xE9 => MediaFn::VolumeUp,
        0xEA => MediaFn::VolumeDown,
        0xE2 => MediaFn::Mute,
        0x192 => MediaFn::Calculator,
        0x194 => MediaFn::MyComputer,
        0x196 => MediaFn::Browser,
        0x18A => MediaFn::Mail,
        _ => return None,
    })
}
fn encode_media(m: MediaFn) -> Vec<u8> {
    let c = media_code(m);
    // Official: compact 8B write of [2, 0x82,lo,hi, 0x42,lo,hi, crc@7].
    let mut b = vec![
        2,
        0x82,
        (c & 0xff) as u8,
        (c >> 8) as u8,
        0x42,
        (c & 0xff) as u8,
        (c >> 8) as u8,
    ];
    let crc = 85u32.wrapping_sub(b.iter().map(|&x| x as u32).sum::<u32>()) as u8;
    b.push(crc);
    b
}

// ==================================================================== DPI records

/// KeyFunction record [type, paramHi, paramLo, crc] → ButtonAction (other than type5).
fn decode_keyfn_basic(b: &[u8]) -> ButtonAction {
    use ButtonAction::*;
    if b.len() < 3 {
        return Disabled;
    }
    let (ty, ph, pl) = (b[0], b[1], b[2]);
    match ty {
        1 => Mouse(match ph {
            1 => MouseFn::Left,
            2 => MouseFn::Right,
            4 => MouseFn::Middle,
            8 => MouseFn::Back,
            16 => MouseFn::Forward,
            _ => MouseFn::Left,
        }),
        2 => Dpi(match ph {
            2 => DpiFn::Up,
            3 => DpiFn::Down,
            _ => DpiFn::Cycle,
        }),
        4 => {
            if ph == 100 && pl == 2 {
                Mouse(MouseFn::DoubleClick)
            } else {
                Fire {
                    button: MouseFn::Left,
                    count: pl,
                    interval_ms: ph as u16,
                }
            }
        }
        6 => Macro {
            slot: ph.max(1) - 1,
            run: match pl {
                254 => MacroRun::WhileHeld,
                253 => MacroRun::Toggle,
                n => MacroRun::Times(n.max(1) as u16),
            },
        },
        7 => PollingCycle,
        10 => Dpi(DpiFn::Shift(((pl as u32) + 1) * 50)),
        11 => Mouse(if ph == 2 {
            MouseFn::ScrollDown
        } else {
            MouseFn::ScrollUp
        }),
        _ => Disabled,
    }
}

fn crc_rec(b: &[u8]) -> u8 {
    85u8.wrapping_sub(b[..b.len() - 1].iter().map(|&x| x as u32).sum::<u32>() as u8)
}

/// 3955 (6B): [xLo,xHi,yLo,yHi,flags,crc]. flags: bit2-3=x high bits, bit6-7=y high bits,
/// bit0/4 = ×2. The value is (raw+1)*step (step=1 for 3955).
/// Non-3955 (4B): [x,y,flags,crc], raw is 10bit + ×2 flag, step=50.
fn parse_stage(sensor_3955: bool, b: &[u8]) -> (u32, u32) {
    if sensor_3955 {
        let f = b[4] as u32;
        let mut x = b[0] as u32 | ((b[1] as u32) << 8) | (((f >> 2) & 3) << 16);
        let mut y = b[2] as u32 | ((b[3] as u32) << 8) | ((f >> 6) << 16);
        x += 1;
        y += 1;
        if f & 1 == 1 {
            x *= 2;
        }
        if f & 2 == 2 {
            y *= 2;
        }
        (x, y)
    } else {
        let f = b[2] as u32;
        let mut x = ((b[0] as u32 | (((f >> 2) & 3) << 8)) + 1) * 50;
        let mut y = ((b[1] as u32 | ((f >> 6) << 8)) + 1) * 50;
        if f & 1 == 1 {
            x *= 2;
        }
        if f & 2 == 2 {
            y *= 2;
        }
        (x, y)
    }
}

fn encode_stage(sensor_3955: bool, x: u32, y: u32) -> Vec<u8> {
    if sensor_3955 {
        let l = x.saturating_sub(1);
        let s = y.saturating_sub(1);
        let mut b = vec![
            (l & 0xff) as u8,
            ((l >> 8) & 0xff) as u8,
            (s & 0xff) as u8,
            ((s >> 8) & 0xff) as u8,
            (((l >> 16) & 3) << 2 | ((s >> 16) & 3) << 6) as u8,
            0,
        ];
        b[5] = crc_rec(&b);
        b
    } else {
        let mut lx = x / 50;
        let mut sx = y / 50;
        let (mut cx, mut cy) = (0u8, 0u8);
        if lx > 1024 {
            lx /= 2;
            cx = 1;
        }
        if sx > 1024 {
            sx /= 2;
            cy = 1;
        }
        let l = lx.saturating_sub(1).min(1023);
        let s = sx.saturating_sub(1).min(1023);
        let mut b = vec![
            (l & 0xff) as u8,
            (s & 0xff) as u8,
            (((l >> 8) & 3) << 2 | ((s >> 8) & 3) << 6 | cx as u32 | ((cy as u32) << 1)) as u8,
            0,
        ];
        b[3] = crc_rec(&b);
        b
    }
}

// ==================================================================== caps

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

fn build_caps(info: &DeviceInfo, m: &R8Model) -> Capabilities {
    use section::*;
    // "Polling rate" when wired directly; behind a dongle it means the wireless segment.
    let wired = matches!(info.transport, Transport::Wired);
    let mut settings = vec![
        SettingSpec::choice(
            "polling_wireless",
            PERFORMANCE,
            if wired {
                "Polling rate"
            } else {
                "Polling rate (wireless)"
            },
            opts(&[
                (125, "125"),
                (250, "250"),
                (500, "500"),
                (1000, "1K"),
                (2000, "2K"),
                (4000, "4K"),
                (8000, "8K"),
            ]),
        )
        .help("Report frequency between mouse and receiver (Hz)"),
        SettingSpec::toggle("competitive", PERFORMANCE, "Competitive mode")
            .help("Runs the sensor at maximum performance at all times (Sensor FPS 20K)"),
        SettingSpec::choice("lod", SENSOR, "Lift-off distance", opts(m.lod_choices))
            .help("Height at which the mouse stops tracking when lifted"),
        SettingSpec::toggle("motion_sync", SENSOR, "Motion sync")
            .help("Syncs pointer and mouse movement to smooth the trajectory"),
        SettingSpec::toggle("angle_snap", SENSOR, "Angle snapping")
            .help("Straightens out wobble in movement"),
        SettingSpec::toggle("ripple", SENSOR, "Ripple control")
            .help("Reduces fine jitter at high DPI"),
    ];
    if m.sensor_angle {
        settings.push(
            SettingSpec::slider("sensor_angle", SENSOR, "Sensor angle", -30., 30., 1., "°")
                .help("Rotates the coordinate axes to match your grip"),
        );
    }
    settings.extend([
        SettingSpec::slider(
            "debounce_press",
            CLICK,
            "Click debounce (press)",
            0.,
            m.debounce_max as f32,
            1.,
            "ms",
        )
        .help("Shorter responds faster; longer prevents accidental clicks")
        .confirm("Setting 0ms may cause double clicks. Continue?"),
        SettingSpec::slider(
            "debounce_release",
            CLICK,
            "Click debounce (release)",
            0.,
            m.debounce_max as f32,
            1.,
            "ms",
        ),
        SettingSpec::slider(
            "wheel_debounce",
            CLICK,
            "Wheel debounce",
            0.,
            245.,
            5.,
            "ms",
        ),
    ]);
    if m.magnetic {
        settings.extend([
            // The official UI shows 1-20 (the EEPROM gets the displayed value - 1).
            SettingSpec::slider(
                "ms_trigger_left",
                MAGNETIC,
                "L trigger position",
                1.,
                20.,
                1.,
                "",
            )
            .help("Depth at which it actuates (each step = 5% depth)"),
            SettingSpec::slider(
                "ms_trigger_right",
                MAGNETIC,
                "R trigger position",
                1.,
                20.,
                1.,
                "",
            ),
            // The official app uses an "enable toggle + sensitivity 1-10". EEPROM byte = enable<<7 | value.
            SettingSpec::toggle("ms_fast_left_en", MAGNETIC, "L Rapid Trigger"),
            SettingSpec::slider(
                "ms_fast_left",
                MAGNETIC,
                "L RT sensitivity",
                1.,
                10.,
                1.,
                "",
            ),
            SettingSpec::toggle("ms_fast_right_en", MAGNETIC, "R Rapid Trigger"),
            SettingSpec::slider(
                "ms_fast_right",
                MAGNETIC,
                "R RT sensitivity",
                1.,
                10.,
                1.,
                "",
            ),
        ]);
    }
    if m.spdt {
        settings.extend([
            SettingSpec::toggle("spdt_left", CLICK, "Left SPDT")
                .help("Detects clicks on both switch contacts to reduce latency"),
            SettingSpec::toggle("spdt_right", CLICK, "Right SPDT")
                .help("Detects clicks on both switch contacts to reduce latency"),
        ]);
    }
    settings.push(
        SettingSpec::choice(
            "sleep_min",
            POWER,
            "Time until sleep",
            opts(&[
                (1, "1 min"),
                (2, "2 min"),
                (3, "3 min"),
                (5, "5 min"),
                (10, "10 min"),
                (15, "15 min"),
                (30, "30 min"),
            ]),
        )
        .help("Goes to sleep after this much time without movement"),
    );
    if m.long_range {
        settings.push(
            SettingSpec::toggle("long_range", CONNECTION, "Long-range mode").help(
                "Keeps the connection stable even when far from the receiver (uses more power)",
            ),
        );
    }

    Capabilities {
        settings,
        actions: vec![ActionSpec::new("pair", CONNECTION, "Pairing mode")
            .help("Puts the receiver into pairing standby. The mouse also needs a pairing action")
            .confirm("Start pairing mode?")],
        profiles: (m.profiles > 1).then_some(ProfileCaps {
            count: m.profiles,
            renamable: false,
        }),
        mouse: Some(MouseCaps {
            dpi_min: 50,
            dpi_max: m.dpi_max,
            dpi_step: if m.sensor_3955 { 1 } else { 50 },
            min_stages: 1,
            max_stages: m.max_stages,
            xy_split: true,
            stage_colors: true,
            stage_toggle: false,
            buttons: [
                "Left",
                "Right",
                "Wheel click",
                "Side (back)",
                "Side (forward)",
            ][..m.btn_num.min(5)]
                .iter()
                .map(|n| ButtonSlot { name: (*n).into() })
                .collect(),
            button_kinds: vec![
                ButtonActionKind::Mouse,
                ButtonActionKind::Dpi,
                ButtonActionKind::Fire,
                ButtonActionKind::Key,
                ButtonActionKind::Media,
                ButtonActionKind::Macro,
                ButtonActionKind::PollingCycle,
                ButtonActionKind::Disabled,
            ],
            require_left_click: true,
            live_buttons: if m.magnetic { vec![0, 1] } else { vec![] },
            adaptive_dpi: None,
            macros: Some(MacroCaps {
                slots: 3,
                max_events: 70,
                max_delay_ms: 0xFFFF,
                modes: opts(&[
                    (0, "Run a set number of times"),
                    (1, "Repeat until released"),
                    (2, "Repeat until pressed again"),
                ]),
                repeat_modes: vec![0],
                names: false,
                mouse_events: true,
            }),
        }),
        keyboard: None,
        lighting: vec![],
        battery: true,
        peq: None,
        firmware_feed: Some(FirmwareFeed::MouseFit {
            ui_folder: m.ui_folder.into(),
        }),
    }
}

// ==================================================================== DeviceDriver

impl DeviceDriver for MfDongleDriver {
    fn info(&self) -> DeviceInfo {
        let mut i = self
            .sh
            .info
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        // FW: op18 → recv[5].[6] (=r[6].r[7], hex). All 0 means asleep.
        // Real device [8,18,0,0,0,2,5,48] → recv[5]=5, recv[6]=48 → "5.30".
        let mut f = [0u8; 16];
        f[0] = 18;
        if let Ok(r) = self.cmd(&f) {
            if r[6] != 0 || r[7] != 0 {
                i.firmware = Some(format!("{:x}.{:02x}", r[6], r[7]));
                i.asleep = false;
            } else {
                i.asleep = true;
            }
        }
        // Dongle FW: op29 (also recv[5].[6] = r[6].r[7])
        let mut f = [0u8; 16];
        f[0] = 29;
        if let Ok(r) = self.cmd(&f) {
            if r[6] != 0 || r[7] != 0 {
                i.dongle_firmware = Some(format!("{:x}.{:02x}", r[6], r[7]));
            }
        }
        if let Ok(b) = self.battery() {
            i.battery = Some(b);
            i.asleep = false;
        }
        i.supported = true;
        i
    }

    fn caps(&self) -> Capabilities {
        self.caps.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn read(&self) -> HubResult<DeviceState> {
        // When it is known to be asleep, send just one probe.
        // If a response arrives, read everything as usual (the reader has already returned to awake),
        // otherwise return the minimal state immediately without waiting for timeouts on every setting.
        if self
            .sh
            .info
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .asleep
        {
            let mut f = [0u8; 16];
            f[0] = 4;
            if self.cmd(&f).is_err() {
                self.mark_asleep(true);
                return Ok(DeviceState {
                    profile: ProfileState {
                        active: 0,
                        names: vec![],
                    },
                    settings: BTreeMap::new(),
                    mouse: Some(MouseState {
                        stages: vec![],
                        active_stage: 0,
                        buttons: vec![],
                        macros: vec![],
                        adaptive: None,
                    }),
                    keyboard: None,
                    lighting: BTreeMap::new(),
                    peq: None,
                });
            }
        }
        // If the model is undetermined on a dongle (e.g. just after waking from sleep), try to determine it here.
        self.maybe_detect();
        let m = self.model();
        let mut settings: BTreeMap<String, SettingValue> = BTreeMap::new();
        macro_rules! get {
            ($id:literal, $r:expr $(,)?) => {
                if let Ok(v) = $r {
                    settings.insert($id.to_string(), v);
                }
            };
        }

        let mut zero_hits = 0u32;

        // Polling: EEPROM @0 is the new protocol's code table (1=1000, 128=8K…). Verified on a real device.
        match self.ee_u8(ee::REPORT_RATE) {
            Ok(code) => {
                let h = crate::mousefit::poll_from_code(code);
                settings.insert("polling_wireless".into(), SettingValue::Choice(h as i64));
            }
            Err(_) => zero_hits += 1,
        }
        get!(
            "competitive",
            self.ee_u8(ee::SENSOR_FPS20K)
                .map(|v| SettingValue::Bool(v == 1))
        );
        match self.ee_u8(ee::LOD) {
            Ok(w) => {
                // Reverse lookup in the wire code table → if not found, try reading mm*10 directly.
                let tenth = m
                    .lod_wire
                    .iter()
                    .position(|&c| c == w)
                    .and_then(|i| m.lod_choices.get(i).map(|(v, _)| *v))
                    .or_else(|| {
                        m.lod_choices
                            .iter()
                            .find(|(v, _)| *v == w as i64 * 10)
                            .map(|(v, _)| *v)
                    });
                if let Some(t) = tenth {
                    settings.insert("lod".into(), SettingValue::Choice(t));
                }
            }
            Err(_) => zero_hits += 1,
        }
        get!(
            "motion_sync",
            self.ee_u8(ee::MOTION_SYNC)
                .map(|v| SettingValue::Bool(v == 1))
        );
        get!(
            "angle_snap",
            self.ee_u8(ee::ANGLE).map(|v| SettingValue::Bool(v == 1))
        );
        get!(
            "ripple",
            self.ee_u8(ee::RIPPLE).map(|v| SettingValue::Bool(v == 1))
        );
        if m.sensor_angle {
            get!(
                "sensor_angle",
                self.ee_u8(ee::ANGLE_TUNE).map(|v| {
                    let v = if v >= 128 { v as i16 - 256 } else { v as i16 };
                    SettingValue::Num(v as f32)
                })
            );
        }
        get!(
            "debounce_press",
            self.ee_u8(ee::DEBOUNCE)
                .map(|v| SettingValue::Num(v as f32))
        );
        get!(
            "debounce_release",
            self.ee_u8(ee::DEBOUNCE_RELEASE)
                .map(|v| SettingValue::Num(v as f32))
        );
        get!(
            "wheel_debounce",
            self.ee_u8(ee::WHEEL_DEBOUNCE)
                .map(|v| SettingValue::Num(v as f32))
        );
        // Sleep: wire = min*6 (JS: sec/10). 0 is treated as unset and omitted.
        if let Ok(w) = self.ee_u8(ee::SLEEP) {
            if w > 0 {
                let min = ((w as f32) / 6.).round() as i64;
                settings.insert("sleep_min".into(), SettingValue::Choice(min.max(1)));
            }
        }
        if m.magnetic {
            for (id, addr) in [
                ("ms_trigger_left", ee::LEFT_TRIGGER),
                ("ms_trigger_right", ee::RIGHT_TRIGGER),
            ] {
                if let Ok(v) = self.ee_u8(addr) {
                    // The EEPROM holds 0-19; shown as +1 (1-20), same as the official UI.
                    settings.insert(id.into(), SettingValue::Num(v.min(19) as f32 + 1.));
                }
            }
            for (en_id, id, addr) in [
                ("ms_fast_left_en", "ms_fast_left", ee::LEFT_FAST),
                ("ms_fast_right_en", "ms_fast_right", ee::RIGHT_FAST),
            ] {
                if let Ok(v) = self.ee_u8(addr) {
                    // bit7=enable, low7=value (official get: byte>>7 / byte&15).
                    // The value remains even when disabled, so return both as-is.
                    settings.insert(en_id.into(), SettingValue::Bool(v & 0x80 != 0));
                    settings.insert(id.into(), SettingValue::Num((v & 15) as f32));
                }
            }
        }
        if m.spdt {
            if let Ok(v) = self.ee_u8(ee::KEY_OP) {
                settings.insert("spdt_left".into(), SettingValue::Bool(v & 1 == 1));
                settings.insert("spdt_right".into(), SettingValue::Bool(v & 2 == 2));
            }
        }
        if m.long_range {
            let mut f = [0u8; 16];
            f[0] = 23;
            if let Ok(r) = self.cmd(&f) {
                // Data starts at recv[5] (=r[6]) — r[5] is the length field (always 10).
                if !(r[6] == 0 && r[7] == 0 && r[8] == 0) {
                    settings.insert("long_range".into(), SettingValue::Bool(r[6] == 1));
                }
            }
        }

        // DPI stages
        let n_stages = self
            .ee_u8(ee::MAX_DPI_STAGE)
            .unwrap_or(0)
            .clamp(1, m.max_stages as u8) as usize;
        if self.ee_u8(ee::MAX_DPI_STAGE).is_err() {
            zero_hits += 1;
        }
        let active = self.ee_u8(ee::CURRENT_DPI).unwrap_or(0) as usize;
        let rec_len = if m.sensor_3955 { 6 } else { 4 };
        let dpi_base = if m.sensor_3955 {
            ee::DPI_3955
        } else {
            ee::DPI_VALUE
        };
        let raw = self
            .ee_read(dpi_base, (n_stages * rec_len) as u16)
            .unwrap_or_default();
        let colors_raw = self
            .ee_read(ee::DPI_COLOR, (n_stages * 4) as u16)
            .unwrap_or_default();
        let mut stages = Vec::with_capacity(n_stages);
        for i in 0..n_stages {
            let lo = i * rec_len;
            if lo >= raw.len() {
                break;
            }
            let b = &raw[lo..(lo + rec_len).min(raw.len())];
            if b.len() < rec_len || b.iter().all(|&x| x == 0) {
                break;
            }
            let (x, y) = self.parse_stage(b);
            let clo = i * 4;
            let c = if clo < colors_raw.len() {
                &colors_raw[clo..(clo + 3).min(colors_raw.len())]
            } else {
                &[]
            };
            stages.push(DpiStage {
                x,
                y: if y == x { None } else { Some(y) },
                color: if c.len() >= 3 {
                    Rgb::new(c[0], c[1], c[2])
                } else {
                    Rgb::hex(0xffffff)
                },
                enabled: true,
            });
        }
        if stages.is_empty() {
            stages.push(DpiStage {
                x: 800,
                y: None,
                color: Rgb::hex(0xffffff),
                enabled: true,
            });
        }
        let active_stage = active.min(stages.len() - 1);

        // Buttons (KeyFunction @96, 4B×slot)
        let nbtn = m.btn_num.min(5);
        let kf = self
            .ee_read(ee::KEY_FUNCTION, (nbtn * 4) as u16)
            .unwrap_or_default();
        let mut buttons = Vec::with_capacity(nbtn);
        for i in 0..nbtn {
            let lo = i * 4;
            let b = if lo < kf.len() {
                &kf[lo..(lo + 4).min(kf.len())]
            } else {
                &[]
            };
            buttons.push(if b.len() >= 4 {
                self.decode_keyfn(i, b)
            } else {
                ButtonAction::Disabled
            });
        }

        // Macros (3 slots)
        let mut macros = Vec::with_capacity(3);
        for s in 0..3u8 {
            macros.push(self.read_macro(s).ok().flatten());
        }

        let asleep = zero_hits >= 3 && settings.is_empty();
        self.mark_asleep(asleep);

        Ok(DeviceState {
            profile: ProfileState {
                active: 0,
                names: vec![],
            },
            settings,
            mouse: Some(MouseState {
                stages,
                active_stage,
                buttons,
                macros,
                adaptive: None,
            }),
            keyboard: None,
            lighting: BTreeMap::new(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        match change {
            Change::Setting { id, value } => self.apply_setting(id, value),
            Change::DpiStages { stages, active } => {
                let n = stages.len().clamp(1, self.model().max_stages);
                // JS: setDPIStageNum — maxDpiStage → active (clamp) → stages → colors
                self.ee_write_val(ee::MAX_DPI_STAGE, n as u8)?;
                self.ee_write_val(ee::CURRENT_DPI, (*active).min(n - 1) as u8)?;
                for (i, s) in stages[..n].iter().enumerate() {
                    let (addr, _) = self.stage_addr(i);
                    self.ee_write(addr, &self.encode_stage(s.x, s.y.unwrap_or(s.x)))?;
                    let mut c = vec![s.color.r, s.color.g, s.color.b, 0];
                    c[3] = Self::crc(&c);
                    self.ee_write(ee::DPI_COLOR + (i * 4) as u16, &c)?;
                }
                Ok(())
            }
            Change::ActiveStage(a) => self.ee_write_val(ee::CURRENT_DPI, *a as u8),
            Change::Button { slot, action } => {
                if *slot >= self.model().btn_num.min(5) {
                    return Err(HubError::Invalid("button slot".into()));
                }
                let (rec, shortcut) = self.encode_keyfn(*slot, action)?;
                if let Some(buf) = shortcut {
                    self.ee_write(ee::SHORTCUT + (*slot * 32) as u16, &buf)?;
                    sleep(Duration::from_millis(50));
                }
                self.ee_write(ee::KEY_FUNCTION + (*slot * 4) as u16, &rec)
            }
            Change::Macro { slot, def } => {
                if *slot >= 3 {
                    return Err(HubError::Invalid("macro slot".into()));
                }
                match def {
                    None => self.clear_macro(*slot as u8),
                    Some(d) if d.events.is_empty() => self.clear_macro(*slot as u8),
                    Some(d) => self.write_macro(*slot as u8, d),
                }
            }
            Change::Profile(_) => Err(HubError::Unsupported("profiles (dongle)")),
            Change::Lighting { .. } => Err(HubError::Unsupported("lighting")),
            _ => Err(HubError::Unsupported("change")),
        }
    }

    fn battery(&self) -> HubResult<Battery> {
        // JS getBatPer: percent=recv[5] (=r[6]), recv[6]=state, recv[7..8]=voltage BE.
        // Measured: percent=85, voltage=4031mV.
        battery_of(&self.sh)
    }

    /// Magnetic button press depth. Only returns the latest value stored by the receive thread (does not touch HID).
    fn live_buttons(&self) -> HubResult<Vec<ButtonDepth>> {
        if !self.model().magnetic {
            return Err(HubError::Unsupported("live_buttons"));
        }
        let (l, r) = *self.sh.depth.lock().unwrap_or_else(|p| p.into_inner());
        // Actuation point not yet obtained (e.g. opened while asleep) → re-read with a 2-second cooldown. Do not show a false bottom line.
        let mut trig = *self.trig.lock().unwrap_or_else(|p| p.into_inner());
        if trig.is_none() {
            let mut at = self.trig_retry.lock().unwrap_or_else(|p| p.into_inner());
            if at.elapsed() >= Duration::from_secs(2) {
                *at = std::time::Instant::now();
                drop(at);
                if let (Ok(l), Ok(r)) =
                    (self.ee_u8(ee::LEFT_TRIGGER), self.ee_u8(ee::RIGHT_TRIGGER))
                {
                    *self.trig.lock().unwrap_or_else(|p| p.into_inner()) = Some((l, r));
                    trig = Some((l, r));
                }
            }
        }
        let mk = |raw: u8, ee: Option<u8>| {
            let depth = (raw as f32 / 100.).clamp(0., 1.);
            // JS: calibrData/5 >= getMSButtonTrigger()+1 → actuated (5% steps).
            let trigger = ee.map(|t| ((t as f32 + 1.) * 0.05).min(1.));
            ButtonDepth {
                depth,
                trigger,
                pressed: trigger.map(|t| depth >= t),
            }
        };
        Ok(vec![mk(l, trig.map(|t| t.0)), mk(r, trig.map(|t| t.1))])
    }

    /// Measured report rate. Only returns the spontaneous packets/second counted by the receive thread.
    fn live_button_fast(&self) -> HubResult<Vec<Option<u8>>> {
        if !self.model().magnetic {
            return Err(HubError::Unsupported("live_button_fast"));
        }
        Ok([ee::LEFT_FAST, ee::RIGHT_FAST]
            .into_iter()
            .map(|addr| {
                self.ee_u8(addr)
                    .ok()
                    .map(|v| if v & 0x80 != 0 { v & 15 } else { 0 })
            })
            .collect())
    }

    fn live_report_hz(&self) -> Option<f32> {
        report_hz(&self.sh)
    }

    fn run_action(&self, id: &str, _file: Option<&std::path::Path>) -> HubResult<ActionOutcome> {
        match id {
            "pair" => {
                let mut f = [0u8; 16];
                f[0] = 5;
                f[4] = 2;
                f[7] = 11;
                self.cmd(&f)?;
                sleep(Duration::from_secs(2));
                let mut f = [0u8; 16];
                f[0] = 6;
                let r = self.cmd(&f)?;
                Ok(ActionOutcome {
                    message: Some(format!("Pairing result: {}", r[6])),
                    reload: false,
                })
            }
            _ => Err(HubError::Unsupported("action")),
        }
    }
}

impl MfDongleDriver {
    fn apply_setting(&self, id: &str, v: &SettingValue) -> HubResult<()> {
        match id {
            "polling_wireless" => {
                let code = crate::mousefit::poll_to_code(v.as_choice() as u32)?;
                self.ee_write_val(ee::REPORT_RATE, code)
            }
            "competitive" => self.ee_write_val(ee::SENSOR_FPS20K, v.as_bool() as u8),
            "lod" => {
                let tenth = v.as_choice();
                let m = self.model();
                let pos = m
                    .lod_choices
                    .iter()
                    .position(|(c, _)| *c == tenth)
                    .ok_or_else(|| HubError::Invalid("lod".into()))?;
                let wire = m
                    .lod_wire
                    .get(pos)
                    .copied()
                    .ok_or_else(|| HubError::Invalid("lod".into()))?;
                self.ee_write_val(ee::LOD, wire)
            }
            "motion_sync" => self.ee_write_val(ee::MOTION_SYNC, v.as_bool() as u8),
            "angle_snap" => self.ee_write_val(ee::ANGLE, v.as_bool() as u8),
            "ripple" => self.ee_write_val(ee::RIPPLE, v.as_bool() as u8),
            "sensor_angle" => {
                let d = v.as_num().round() as i16;
                // JS: AngleTuneState=1 first, then AngleTune.
                self.ee_write_val(ee::ANGLE_TUNE_STATE, 1)?;
                sleep(Duration::from_millis(50));
                let enc = if d > 0 {
                    d as u8
                } else {
                    (255 - d.unsigned_abs() + 1) as u8
                };
                self.ee_write_val(ee::ANGLE_TUNE, enc)
            }
            "debounce_press" => self.ee_write_val(
                ee::DEBOUNCE,
                v.as_num().clamp(0., self.model().debounce_max as f32) as u8,
            ),
            "debounce_release" => self.ee_write_val(
                ee::DEBOUNCE_RELEASE,
                v.as_num().clamp(0., self.model().debounce_max as f32) as u8,
            ),
            "wheel_debounce" => {
                self.ee_write_val(ee::WHEEL_DEBOUNCE, v.as_num().clamp(0., 245.) as u8)
            }
            "sleep_min" => {
                // wire = min*6 (JS: r>=60?r/60*6:r/10 = sec/10).
                let min = v.as_choice().clamp(0, 42);
                self.ee_write_val(ee::SLEEP, (min * 6) as u8)
            }
            "ms_trigger_left" | "ms_trigger_right" => {
                // The UI value is 1-20 (same as official) → EEPROM gets -1 (0-19). The trig cache holds the raw value.
                let n = (v.as_num() as i64 - 1).clamp(0, 19) as u8;
                let addr = if id == "ms_trigger_left" {
                    ee::LEFT_TRIGGER
                } else {
                    ee::RIGHT_TRIGGER
                };
                self.ee_write_val(addr, n)?;
                let mut t = self.trig.lock().unwrap_or_else(|p| p.into_inner());
                *t = Some(if id == "ms_trigger_left" {
                    (n, t.map(|x| x.1).unwrap_or(0))
                } else {
                    (t.map(|x| x.0).unwrap_or(0), n)
                });
                Ok(())
            }
            "ms_fast_left_en" | "ms_fast_right_en" | "ms_fast_left" | "ms_fast_right" => {
                // Official setMSButtonFastTrigger: byte = enable<<7 | (value&127).
                // When changing only one half, preserve the other half with a read-modify-write.
                let addr = if id.starts_with("ms_fast_left") {
                    ee::LEFT_FAST
                } else {
                    ee::RIGHT_FAST
                };
                let cur = self.ee_u8(addr).unwrap_or(0);
                let wire = if id.ends_with("_en") {
                    (cur & 0x7f) | ((v.as_bool() as u8) << 7)
                } else {
                    (cur & 0x80) | (v.as_num().clamp(0., 127.) as u8)
                };
                self.ee_write_val(addr, wire)
            }
            "spdt_left" | "spdt_right" => {
                // Die: EEPROM @KeyOperation bit0=left, bit1=right.
                // The official app overwrites the whole byte, but we use a read-modify-write to preserve unknown bits.
                let cur = self.ee_u8(ee::KEY_OP).unwrap_or(0);
                let bit = if id == "spdt_left" { 1 } else { 2 };
                let next = if v.as_bool() { cur | bit } else { cur & !bit };
                self.ee_write_val(ee::KEY_OP, next)
            }
            "long_range" => {
                let mut f = [0u8; 16];
                f[0] = 22;
                f[4] = 10;
                f[5] = v.as_bool() as u8;
                self.cmd(&f)?;
                Ok(())
            }
            other => Err(HubError::Invalid(format!("unknown setting {other}"))),
        }
    }
}

// ==================================================================== tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_3955_roundtrip() {
        let enc = encode_stage(true, 1600, 3200);
        assert_eq!(enc.len(), 6);
        assert_eq!(&enc[..4], &[0x3F, 0x06, 0x7F, 0x0C]); // 1599 / 3199 LE
        let (x, y) = parse_stage(true, &enc);
        assert_eq!((x, y), (1600, 3200));
        // crc: sum of all bytes ≡ 85
        assert_eq!(enc.iter().map(|&b| b as u32).sum::<u32>() & 0xff, 85);
        // High bits (18-bit value)
        let enc = encode_stage(true, 40000, 40000);
        let (x, y) = parse_stage(true, &enc);
        assert_eq!((x, y), (40000, 40000));
    }

    #[test]
    fn stage_3950_roundtrip() {
        for (x, y) in [(800, 800), (1600, 3200), (26000, 26000)] {
            let enc = encode_stage(false, x, y);
            assert_eq!(enc.len(), 4);
            assert_eq!(enc.iter().map(|&b| b as u32).sum::<u32>() & 0xff, 85);
            let (px, py) = parse_stage(false, &enc);
            assert_eq!((px, py), (x, y));
        }
    }

    #[test]
    fn htx3950_eeprom_fixture() {
        // EEPROM dump of a real 5618 (JM_056_3950) (htxprobe):
        //   @0 ReportRate=0x20(4K) @4 CurrentDPI=1 @8 KeyOperation=1 @10 LOD=3
        //   @12 4B stages 07/1f/1f/3f = 400/1600/1600/3200
        let m = &HTX_ULTRA_3950;
        assert_eq!(m.lod_choices.len(), m.lod_wire.len());
        // LOD wire=3 → 0.7mm (official write map {0.7:3, 1:1, 2:2})
        let pos = m.lod_wire.iter().position(|&c| c == 3).unwrap();
        assert_eq!(m.lod_choices[pos].0, 7);
        // DPI 4B record measured values
        assert_eq!(parse_stage(false, &[0x07, 0x07, 0x00, 0x47]), (400, 400));
        assert_eq!(parse_stage(false, &[0x1f, 0x1f, 0x00, 0x17]), (1600, 1600));
        assert_eq!(parse_stage(false, &[0x3f, 0x3f, 0x00, 0xd7]), (3200, 3200));
        // ReportRate 0x20 = 4K
        assert_eq!(crate::mousefit::poll_from_code(0x20), 4000);
        // SPDT @8 = 1 → left ON, right OFF
        assert!(1u8 & 1 == 1 && !(1u8 & 2 == 2));
        // model_for branches
        assert_eq!(model_for("htx-ultra-3950").dpi_max, 32_000);
        assert_eq!(model_for("htx-ultra").dpi_max, 30_000);
        assert_eq!(model_for("htx-ultra-3955").dpi_max, 40_000);
        assert!(model_for("htx-ultra-3950").spdt);
        assert!(!model_for("htx-ultra").spdt);
        // EID of the 3854 dongle (env-models MID) → inner model
        assert_eq!(mid_lookup(5).unwrap().0, "htx-ultra-3950");
        assert_eq!(mid_lookup(7).unwrap().0, "htx-ultra-3955");
        assert_eq!(mid_lookup(11).unwrap().0, "hts-plus-pro");
        assert!(mid_lookup(0).is_none());
    }

    #[test]
    fn keyfn_decode() {
        // type1 mouse back (paramHi = mask 8)
        let r = [1u8, 8, 0, 0];
        assert_eq!(decode_keyfn_basic(&r), ButtonAction::Mouse(MouseFn::Back));
        // type6 macro slot2 whileheld
        let r = [6u8, 3, 254, 0];
        assert_eq!(
            decode_keyfn_basic(&r),
            ButtonAction::Macro {
                slot: 2,
                run: MacroRun::WhileHeld
            }
        );
        // type11 scroll
        let r = [11u8, 2, 0, 0];
        assert_eq!(
            decode_keyfn_basic(&r),
            ButtonAction::Mouse(MouseFn::ScrollDown)
        );
    }

    #[test]
    fn shortcut_roundtrip() {
        let k = KeyStroke {
            mods: mods::LCTRL | mods::LSHIFT,
            usage: 0x04,
        };
        let b = encode_shortcut(&k);
        // Official format: a compact sequence of [count][press×k][release reverse order][crc].
        // press order: LCtrl, LShift, key (the official order of mods is Ctrl,Alt,Shift,Win).
        assert_eq!(b[0], 6);
        assert_eq!(
            &b[1..10],
            &[0x80, 0x01, 0x00, 0x80, 0x02, 0x00, 0x81, 0x04, 0x00]
        );
        // release is the reverse order of press: key, LShift, LCtrl.
        assert_eq!(
            &b[10..19],
            &[0x41, 0x04, 0x00, 0x40, 0x02, 0x00, 0x40, 0x01, 0x00]
        );
        // crc = 85 - sum(everything before it) goes at index 1+3*count.
        assert_eq!(b.len(), 20);
        assert_eq!(
            b[19],
            85u32.wrapping_sub(b[..19].iter().map(|&x| x as u32).sum::<u32>()) as u8
        );
        // Same loop as decode
        let n = b[0] as usize;
        let mut mods = 0u8;
        let mut usage = 0u16;
        for i in 0..n {
            let o = 1 + i * 3;
            let t = b[o] & 0x3f;
            let v = b[o + 1] as u16 | ((b[o + 2] as u16) << 8);
            if b[o] & 0x80 != 0 {
                match t {
                    0 => mods |= v as u8,
                    1 => usage = v,
                    _ => {}
                }
            }
        }
        assert_eq!((mods, usage), (0b11, 4));
    }

    #[test]
    fn shortcut_matches_device_fixture() {
        // HTS Plus Pro real-device dump (after a write by the official Hub): usage31 = `02 81 1f 00 41 1f 00 53`.
        let b = encode_shortcut(&KeyStroke { mods: 0, usage: 31 });
        assert_eq!(b, vec![0x02, 0x81, 0x1f, 0x00, 0x41, 0x1f, 0x00, 0x53]);
    }

    #[test]
    fn media_encode_matches_official() {
        // Official JS: [2, 0x82,lo,hi, 0x42,lo,hi, crc@7] (MediaPlayPause=205=0xCD).
        let b = encode_media(MediaFn::PlayPause);
        assert_eq!(b, vec![2, 0x82, 0xCD, 0x00, 0x42, 0xCD, 0x00, 0xF5]);
    }
}
