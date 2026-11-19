//! XSYD protocol (hub.xsyd.top / Sparklink SDK) — Hall effect keyboards such as the SU66 Max.
//!
//! Analyzed on a real device (1ca6:3009, usage_page 0xFFB0 / usage 1):
//! - 64B output report + 64B input report, no report id
//! - Frame: `[group][sub][args...]` → response `[group][sub][data...]`
//! - group: Device=1, Global=2, LayoutAndKey=3, Performance=4, Lighting=5,
//!   HigherKey=6, Macro=7, CustomCommand=10, Displayer=12, ThreeMode=13
//! - A response with `sub==0xFF` means "unsupported command" (confirmed on a real device)
//!
//! # Verification status
//! - **Verified on a real device**: `1,2` device info, `1,3` feature (layer count), `3,1` keymap read,
//!   `3,4` single key write, `5,1`/`5,2` lighting read/write, `4,1`/`4,2` per-key performance,
//!   `4,3` axis data (route/calibrate), `2,3` config, `2,4` OS, `2,5` polling rate,
//!   `2,8` light areas, `2,12` RT precision, `2,13` sleep, `2,14` macro area,
//!   `2,16` debounce, `6,1` advanced key read, `7,1`/`7,3` macro read.
//! - Primary source for the frames: the SDK bundle of dshk.xsyd.top (`xsyd-sdk.js` in the repository).
//!   Every u16 value is little-endian (`q`/`ee`); mm is a u16 scaled by ×1000.

use crate::transport::Channel;
use devhub_core::backend::*;
use devhub_core::model::*;
use devhub_core::registry;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const POLL_MS: i32 = 60;
const DEADLINE_MS: u64 = 700;
/// Declared dead after this many consecutive timeouts (so a full read() does not stall).
const FAIL_FAST_AFTER: u32 = 4;
/// Interval for checking liveness with a real command while declared dead.
const PROBE_MS: u64 = 1500;
/// Timeout for the probe while declared dead.
const PROBE_WAIT_MS: u64 = 120;

mod grp {
    pub const DEVICE: u8 = 1;
    pub const GLOBAL: u8 = 2;
    pub const LAYOUT_KEY: u8 = 3;
    pub const PERFORMANCE: u8 = 4;
    pub const LIGHTING: u8 = 5;
    pub const HIGHER_KEY: u8 = 6;
    pub const MACRO: u8 = 7;
    pub const CUSTOM: u8 = 10;
    pub const THREE_MODE: u8 = 13;
}
mod sub {
    pub const READ: u8 = 1;
    pub const SETTING: u8 = 2;
    pub const DEVICE_INFO: u8 = 2;
    pub const GET_BASIC_INFO: u8 = 1;
    /// device feature `[1,3]` — feature bitmask: r[2]=axisType (model enum),
    /// r[3]=connection type, r[4]={rgb,knob}, r[5]={screen,haptic,numpad,dotMatrix}.
    #[allow(dead_code)]
    pub const FEATURE: u8 = 3;
    pub const GET_KEY_LAYOUT: u8 = 1;
    /// Guess: row-wise keymap write. Mirror of read.
    pub const SET_KEY_LAYOUT: u8 = 2;
    /// group 3: read/write of a single key (SDK GetKeyCode/SetKeyCode).
    #[allow(dead_code)]
    pub const GET_KEY_CODE: u8 = 3;
    pub const SET_KEY_CODE: u8 = 4;
    /// group 3: physical layout description `[3,5,page]` (up to ~15 cells per page of
    /// u16LE {row:5,col:7,ratio:4}). Factory keymap `[3,6,(sys<<4)|fn,row]`.
    #[allow(dead_code)]
    pub const GET_KEY_LAYOUT_STYLE: u8 = 5;
    pub const GET_KEY_LAYOUT_DEFAULT: u8 = 6;
    /// group 4 subs (SDK $a): GetPerformance=1, SetPerformance=2, AxisData=3.
    pub const GET_PERF: u8 = 1;
    pub const SET_PERF: u8 = 2;
    pub const AXIS_DATA: u8 = 3;
    /// group 5 subs: basic lighting read/write shares READ/SETTING.
    /// Custom color: ReadBack=3, Custom=4, DirectDrive=5, Caps=6 (SDK enum confirmed).
    pub const LIGHT_READBACK: u8 = 3;
    pub const LIGHT_CUSTOM: u8 = 4;
    /// DirectDrive — per-LED live write, no response (SDK sendDataNoResponse).
    pub const LIGHT_DIRECTDRIVE: u8 = 5;
    /// group 7 subs (SDK Xa): GetMacroMode=1, SetMacroMode=2, GetMacro=3, SetMacro=4.
    pub const GET_MACRO_MODE: u8 = 1;
    pub const SET_MACRO_MODE: u8 = 2;
    pub const GET_MACRO: u8 = 3;
    pub const SET_MACRO: u8 = 4;
}
/// subs of group 2 (Global) (SDK Wa).
mod gfs {
    pub const RESET_FACTORY: u8 = 1;
    pub const SAVE_PARAM: u8 = 2;
    pub const CONFIG_SWITCH: u8 = 3;
    pub const SYSTEM_SETTING: u8 = 4;
    pub const REPORT_RATE: u8 = 5;
    pub const CALIBRATION: u8 = 6;
    pub const AXIS_LIB: u8 = 7;
    pub const EFFECT_AREA: u8 = 8;
    #[allow(dead_code)]
    pub const DEFAULT_AXIS: u8 = 9;
    #[allow(dead_code)]
    pub const DOUBLE_LIGHT: u8 = 10;
    #[allow(dead_code)]
    pub const SPECIAL_LIGHT: u8 = 11;
    pub const RT_PRECISION: u8 = 12;
    pub const SLEEP_TIME: u8 = 13;
    pub const MACRO_SPACE: u8 = 14;
    pub const SHAKE_OPT: u8 = 16;
}
/// Scope of SaveParam / ResetFactory (SDK qo/Qo).
mod scope {
    pub const ALL: u8 = 0;
    pub const CALIBRATION: u8 = 1;
    pub const PERFORMANCE: u8 = 2;
    pub const LIGHTING: u8 = 3;
    pub const LAYOUT: u8 = 4;
    pub const HIGHER_KEY: u8 = 5;
    pub const MACRO: u8 = 6;
    #[allow(dead_code)]
    pub const AXIS: u8 = 7;
}
/// Second argument of AxisData (4,3) (SDK Qa).
mod axis {
    #[allow(dead_code)]
    pub const ADC: u8 = 0;
    pub const ROUTE: u8 = 1;
    #[allow(dead_code)]
    pub const CALIBRATE: u8 = 2;
    #[allow(dead_code)]
    pub const KEY_STATUS: u8 = 3;
}
/// Advanced key kind (SDK ja). The mode byte on the wire.
mod hk {
    pub const NONE: u8 = 0;
    pub const DKS: u8 = 1;
    pub const MPT: u8 = 2;
    pub const MT: u8 = 3;
    pub const TGL: u8 = 4;
    pub const END: u8 = 5;
    pub const SOCD: u8 = 6;
    pub const RS: u8 = 7;
}

/// Device constants queried once at open.
struct XsydInfo {
    layers: u8,
    /// RT precision (mm). `2,12` response r[3]/1000. 0.01 if the query fails.
    rt_step: f32,
    /// Full stroke (mm). Max axisRangeMax of the route /1000. 4.0 if the query fails.
    travel: f32,
    /// Polling rate options (wire code → Hz).
    polling: Vec<(u8, &'static str)>,
    /// Config (profile) count. `2,3` list response r[3].
    configs: u8,
    /// Lighting area index list (0=keyboard, 1..=decoration).
    light_areas: Vec<u8>,
    /// Macro (slot count, max events per macro).
    macro_space: Option<(u8, u16)>,
    /// Axis library (list of u16 ids). If empty, switch_types is not shown.
    axes: Vec<u16>,
    /// Whether it is DoubleLighting (2 zones, upper and lower). `2,10,0` response r[3]==1.
    double_lighting: bool,
}

/// Cell number → confirmed BGR. None for cells whose application is uncertain because the write failed.
type DdPins = BTreeMap<usize, Option<[u8; 3]>>;

/// Type of the axis frame cache: (kind,row) → (capture time, 30 cells).
type AxisCache = Mutex<HashMap<(u8, u8), (Instant, [u16; 30])>>;

pub struct XsydDriver {
    info: DeviceInfo,
    ch: Channel,
    xs: XsydInfo,
    caps: Capabilities,
    /// Number of consecutive command timeouts. Once it exceeds the threshold, immediately return Disconnected,
    /// and periodically pass one real command through, only once, to detect recovery.
    /// (prevents every command of read() from waiting 700ms each on an unresponsive device)
    fails: AtomicU32,
    dead_since: Mutex<Option<Instant>>,
    /// LED cells currently pinned by DirectDrive and the color last sent
    /// (area → cell → BGR). Cells that drop out of the set in the next frame are returned to
    /// firmware control by clearing isCustom with `[5,4]` (DD's flag=0 means "don't touch",
    /// not release). Cells that keep the same color are not resent either —
    /// pins remain after the stream stops, so the display holds with only the changes.
    dd_pins: Mutex<BTreeMap<u8, DdPins>>,
    /// Cells and colors that were user custom (isCustom=255) before the DD session started
    /// (area → cell → BGR). Saved aside so DD release can restore them with flag=255 + the saved color —
    /// DD and user custom share the same isCustom flag,
    /// so without telling them apart DD would take the user's custom down with it.
    dd_pre: Mutex<BTreeMap<u8, BTreeMap<usize, [u8; 3]>>>,
    /// Axis frame cache: (kind,row) → (capture time, 30 cells).
    /// The input queue fans out to every open handle, so frames triggered by
    /// requests from other tools such as the official HUB can also be picked up (passive piggyback).
    axis_cache: AxisCache,
    /// Time of the `[4,3,kind,row]` I sent most recently — for detecting external traffic.
    axis_self: Mutex<HashMap<(u8, u8), Instant>>,
    /// Time of the command (grp,sub) I sent most recently — used to tell whether a non-axis frame is
    /// "someone else's response" or "my own response".
    cmd_self: Mutex<HashMap<(u8, u8), Instant>>,
    /// Deadline until which requests are held back after detecting external traffic (passive mode).
    axis_passive: Mutex<Instant>,
}

/// `2,5` polling rate wire code → label (SDK Fn).
fn polling_label(code: u8) -> &'static str {
    match code {
        0 => "8000 Hz",
        1 => "4000 Hz",
        2 => "2000 Hz",
        3 => "1000 Hz",
        4 => "500 Hz",
        5 => "250 Hz",
        _ => "125 Hz",
    }
}

pub fn open(dev: hidapi::HidDevice, info: DeviceInfo) -> HubResult<XsydDriver> {
    dev.set_blocking_mode(false).ok();
    let ch = Channel::open(dev, 65, 65);
    let mut drv = XsydDriver {
        caps: Capabilities::empty(),
        xs: XsydInfo {
            layers: 2,
            rt_step: 0.01,
            travel: 4.0,
            polling: vec![
                (0, "8000 Hz"),
                (1, "4000 Hz"),
                (2, "2000 Hz"),
                (3, "1000 Hz"),
            ],
            configs: 0,
            light_areas: vec![0],
            macro_space: None,
            axes: vec![],
            double_lighting: false,
        },
        info,
        ch,
        fails: AtomicU32::new(0),
        dead_since: Mutex::new(None),
        dd_pins: Mutex::new(BTreeMap::new()),
        dd_pre: Mutex::new(BTreeMap::new()),
        axis_cache: Mutex::new(HashMap::new()),
        axis_self: Mutex::new(HashMap::new()),
        cmd_self: Mutex::new(HashMap::new()),
        axis_passive: Mutex::new(Instant::now() - Duration::from_secs(1)),
    };
    // [1,3] DeviceFeature: r[2]=axisType (model variant enum, magnetic=2), r[3]=connection,
    // r[4]={rgb,knob}, r[5]={screen,haptic,numpad,dotMatrix}. Not the layer count.
    // Keymap layers are fixed to fn0..3 as in the SDK (on the real device layer4 returns all 0).
    drv.xs.layers = 4;
    // Read the FW version at open time and put it on the device name tag.
    let mut info = drv.info.clone();
    if let Ok(r) = drv.cmd(grp::DEVICE, sub::DEVICE_INFO, &[]) {
        info.firmware = parse_fw(&r);
        // Arguments of the distribution API: board_id = r[4..8] BE u32, appVersion = r[8..12] joined with ".".
        info.board_id = Some(u32::from_be_bytes([r[4], r[5], r[6], r[7]]));
        info.firmware = Some(format!("{}.{}.{}.{}", r[8], r[9], r[10], r[11]));
    }
    info.supported = true;
    drv.info = info;

    // ---- Batch query of constants (all failures are ignored and defaults are used) ----
    // RT precision: [2,12,0] → r[3]/1000
    if let Ok(r) = drv.cmd(grp::GLOBAL, gfs::RT_PRECISION, &[0]) {
        let step = r[3] as f32 / 1000.0;
        if step > 0.0 {
            drv.xs.rt_step = step;
        }
    }
    // Polling rate options: [2,5,0] → r[3]=total, r[4..] = codes
    if let Ok(r) = drv.cmd(grp::GLOBAL, gfs::REPORT_RATE, &[0]) {
        let n = (r[3] as usize).min(7);
        let v: Vec<_> = (0..n)
            .map(|i| (r[4 + i], polling_label(r[4 + i])))
            .collect();
        if !v.is_empty() {
            drv.xs.polling = v;
        }
    }
    // Config count: [2,3,0] → r[3]=total
    if let Ok(r) = drv.cmd(grp::GLOBAL, gfs::CONFIG_SWITCH, &[0]) {
        drv.xs.configs = r[3].clamp(0, 10);
    }
    // Light areas: [2,8,0] → r[3]=total, r[4+4i]=index
    if let Ok(r) = drv.cmd(grp::GLOBAL, gfs::EFFECT_AREA, &[0]) {
        let n = (r[3] as usize).min(14);
        let areas: Vec<u8> = (0..n).map(|i| r[4 + i * 4]).collect();
        if !areas.is_empty() {
            drv.xs.light_areas = areas;
        }
    }
    // Macro area: [2,14,0] → r[3]=slot count, r[4,5]=max events
    if let Ok(r) = drv.cmd(grp::GLOBAL, gfs::MACRO_SPACE, &[0]) {
        if r[3] > 0 {
            drv.xs.macro_space = Some((r[3], u16::from_le_bytes([r[4], r[5]])));
        }
    }
    // Axis library: [2,7,0] → r[3]=total, r[4..]=u16 id
    if let Ok(r) = drv.cmd(grp::GLOBAL, gfs::AXIS_LIB, &[0]) {
        let n = (r[3] as usize).min(30);
        drv.xs.axes = (0..n)
            .map(|i| u16::from_le_bytes([r[4 + i * 2], r[5 + i * 2]]))
            .collect();
    }
    // DoubleLighting: [2,10,0] → r[3]==1 means 2 zones, upper and lower (SDK GFSDoubleLighting).
    if let Ok(r) = drv.cmd(grp::GLOBAL, gfs::DOUBLE_LIGHT, &[0]) {
        drv.xs.double_lighting = r[3] == 1;
    }
    // Full stroke: r[23,24] (axisRangeMax) /1000 of the perf response of any 1 key.
    // Tail order is v2Id(2) + rangeMax(2) + coef(2) — r[25,26] is the coefficient, so it is not read.
    if let Some(k) = registry::su66max_layout().first() {
        let (row, col) = registry::su66max_matrix_pos(k.id);
        if let Ok(r) = drv.cmd(grp::PERFORMANCE, sub::GET_PERF, &[row, col]) {
            let t = u16::from_le_bytes([r[23], r[24]]) as f32 / 1000.0;
            if t > 0.5 {
                drv.xs.travel = t;
            }
        }
    }
    drv.caps = build_caps(&drv.xs);
    Ok(drv)
}

impl XsydDriver {
    /// group/sub frame → 64B response. sub==0xFF (unsupported) is Unsupported.
    fn cmd(&self, group: u8, sub: u8, args: &[u8]) -> HubResult<[u8; 64]> {
        // fail-fast: while consecutive timeouts continue, return Disconnected immediately. Every PROBE_MS,
        // pass one real command through, only once, to check recovery (do not persist even in the dead state).
        let probing = self.fails.load(Ordering::Relaxed) >= FAIL_FAST_AFTER;
        if probing {
            let mut ds = self.dead_since.lock().unwrap_or_else(|p| p.into_inner());
            match *ds {
                Some(t) if t.elapsed() < Duration::from_millis(PROBE_MS) => {
                    return Err(HubError::Disconnected)
                }
                _ => *ds = Some(Instant::now()),
            }
        }
        let mut w = [0u8; 64];
        w[0] = group;
        w[1] = sub;
        let n = args.len().min(62);
        w[2..2 + n].copy_from_slice(&args[..n]);
        self.ch.send(&w)?;
        // Record my own send — used by drain to tell it apart from "someone else's response".
        self.cmd_self
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((group, sub), Instant::now());

        let wait_ms = if probing { PROBE_WAIT_MS } else { DEADLINE_MS };
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        while Instant::now() < deadline {
            match self.ch.recv(POLL_MS) {
                Ok(b) if b.first().copied() == Some(group) => {
                    let mut out = [0u8; 64];
                    out[..b.len().min(64)].copy_from_slice(&b[..b.len().min(64)]);
                    if out[1] == 0xFF {
                        return Err(HubError::Unsupported("command"));
                    }
                    if group == grp::LIGHTING && sub == sub::LIGHT_READBACK && b.len() < 64 {
                        return Err(HubError::Protocol("xsyd: short LED readback".into()));
                    }
                    self.fails.store(0, Ordering::Relaxed);
                    *self.dead_since.lock().unwrap_or_else(|p| p.into_inner()) = None;
                    return Ok(out);
                }
                Ok(_) => {}
                Err(HubError::Protocol(_)) => {}
                Err(e) => return Err(e),
            }
        }
        let f = self.fails.fetch_add(1, Ordering::Relaxed) + 1;
        if f >= FAIL_FAST_AFTER {
            *self.dead_since.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
            return Err(HubError::Disconnected);
        }
        Err(HubError::Protocol("xsyd: response timeout".into()))
    }

    /// Safety net on driver drop — DD's isCustom pins remain indefinitely (confirmed on a real device:
    /// they stay stuck even after the stream stops), so only the tracked pins are released
    /// before the handle closes. If DD was never used, nothing is sent (it used to
    /// send a zero payload to every chunk, wiping the user's custom colors too).
    /// (`direct_drive_stop` does the complete stop on the normal path,
    /// so here only a best-effort pin release is done)
    fn release_dd_pins(&self) {
        let pins = self.dd_pins.lock().map(|p| p.clone()).unwrap_or_default();
        for (area, cells) in pins {
            if cells.is_empty() {
                continue;
            }
            let set: std::collections::HashSet<usize> = cells.keys().copied().collect();
            let _ = self.release_cells(area, &set);
        }
    }

    /// Saves aside the cells + colors of user custom (isCustom=255) with `[5,3]` before a DD session
    /// starts. Used by `release_cells` to restore with flag=255 + the saved color
    /// when DD is released.
    fn capture_dd_pre(&self, area: u8) -> HubResult<()> {
        let mut pre = BTreeMap::new();
        for c in 0..9u8 {
            let rb = self.read_dd_chunk(area, c)?;
            for i in 0..15usize {
                if rb[7 + i * 4] == 255 {
                    pre.insert(
                        c as usize * 15 + i,
                        [rb[4 + i * 4], rb[5 + i * 4], rb[6 + i * 4]],
                    );
                }
            }
        }
        self.dd_pre
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(area, pre);
        Ok(())
    }

    fn read_dd_chunk(&self, area: u8, chunk: u8) -> HubResult<[u8; 64]> {
        let r = self.cmd(grp::LIGHTING, sub::LIGHT_READBACK, &[area, chunk])?;
        if r[1..4] != [sub::LIGHT_READBACK, area, chunk] {
            return Err(HubError::Protocol(format!(
                "xsyd: unexpected LED chunk {chunk}"
            )));
        }
        Ok(r)
    }

    /// Releases the DD pins of `cells` with `[5,4]`. For each chunk, reads back with `[5,3]`
    /// and fixes only the target cells, so the user's custom and other pins are preserved.
    /// If a target cell is in `dd_pre`, it is restored with the pre-DD custom color + flag=255,
    /// otherwise it is released with flag=0 (the color byte is kept).
    /// Return value: the set of chunks whose write failed.
    fn release_cells(
        &self,
        area: u8,
        cells: &std::collections::HashSet<usize>,
    ) -> std::collections::HashSet<u8> {
        let pre = self
            .dd_pre
            .lock()
            .map(|p| p.get(&area).cloned().unwrap_or_default())
            .unwrap_or_default();
        let chunks: std::collections::BTreeSet<u8> = cells.iter().map(|c| (c / 15) as u8).collect();
        let mut failed = std::collections::HashSet::new();
        for c in chunks {
            let lo = c as usize * 15;
            let ok = if let Ok(rb) = self.read_dd_chunk(area, c) {
                let mut args = vec![area, c];
                for i in 0..15usize {
                    let cell_no = lo + i;
                    let mut cell = [rb[4 + i * 4], rb[5 + i * 4], rb[6 + i * 4], rb[7 + i * 4]];
                    if cells.contains(&cell_no) {
                        if let Some(bgr) = pre.get(&cell_no) {
                            cell = [bgr[0], bgr[1], bgr[2], 255];
                        } else {
                            cell[3] = 0;
                        }
                    }
                    args.extend_from_slice(&cell);
                }
                self.cmd(grp::LIGHTING, sub::LIGHT_CUSTOM, &args)
                    .is_ok_and(|r| r[1..4] == [sub::LIGHT_CUSTOM, area, c])
            } else {
                false
            };
            if !ok {
                failed.insert(c);
            }
        }
        failed
    }

    /// Returns Err(Disconnected) if the device is declared dead. Call it at breakpoints of read()/apply()
    /// to stop the sweep of every command to an unresponsive device early.
    fn alive(&self) -> HubResult<()> {
        if self.fails.load(Ordering::Relaxed) >= FAIL_FAST_AFTER {
            Err(HubError::Disconnected)
        } else {
            Ok(())
        }
    }

    /// Reads the key stream of a layer: `[3,1,layer,row]` → r[4..64] = 30×u16 LE.
    /// Position in the stream is row*30+col.
    fn read_keymap(&self, layer: u8) -> HubResult<Vec<u16>> {
        let mut stream = vec![0u16; 8 * 30];
        let mut got_any = false;
        for row in 0..8u8 {
            match self.cmd(grp::LAYOUT_KEY, sub::GET_KEY_LAYOUT, &[layer, row]) {
                Ok(r) if r[2] == layer && r[3] == row => {
                    for i in 0..30 {
                        stream[row as usize * 30 + i] =
                            u16::from_le_bytes([r[4 + i * 2], r[5 + i * 2]]);
                    }
                    got_any = true;
                }
                Ok(_) => {}
                Err(HubError::Unsupported(_)) | Err(HubError::Protocol(_)) => break,
                Err(e) => return Err(e),
            }
        }
        if !got_any {
            return Err(HubError::Protocol("keymap: no rows".into()));
        }
        Ok(stream)
    }

    /// Writes a single key `[3,4,layer,row,col,codeLE]` (SDK SetKeyCode).
    fn write_key(&self, layer: u8, layout_index: u16, code: u16) -> HubResult<()> {
        let (row, col) = registry::su66max_matrix_pos(layout_index);
        let mut args = vec![layer, row, col];
        args.extend_from_slice(&code.to_le_bytes());
        self.cmd(grp::LAYOUT_KEY, sub::SET_KEY_CODE, &args)?;
        Ok(())
    }

    /// Writes the whole stream row by row.
    fn write_stream(&self, layer: u8, stream: &[u16]) -> HubResult<()> {
        for row in 0..8u8 {
            let mut args = vec![layer, row];
            for i in 0..30 {
                args.extend_from_slice(&stream[row as usize * 30 + i].to_le_bytes());
            }
            self.cmd(grp::LAYOUT_KEY, sub::SET_KEY_LAYOUT, &args)?;
        }
        Ok(())
    }

    /// Factory keymap `[3,6,(system<<4)|fn,row]` → 30×u16 stream.
    /// Gets the factory values of the Fn layer (number row → F1-F12 etc.). Rows that fail stay 0.
    fn factory_stream(&self, layer: u8) -> Option<Vec<u16>> {
        let sys = match self.read_setting("system_type") {
            Some(SettingValue::Choice(v)) => v.clamp(0, 1) as u8,
            _ => 0,
        };
        let sel = (sys << 4) | layer;
        let mut stream = vec![0u16; 8 * 30];
        let mut got_any = false;
        for row in 0..8u8 {
            match self.cmd(grp::LAYOUT_KEY, sub::GET_KEY_LAYOUT_DEFAULT, &[sel, row]) {
                Ok(r) if r[2] == sel && r[3] == row => {
                    for i in 0..30 {
                        stream[row as usize * 30 + i] =
                            u16::from_le_bytes([r[4 + i * 2], r[5 + i * 2]]);
                    }
                    got_any = true;
                }
                Ok(_) => {}
                Err(HubError::Unsupported(_)) | Err(HubError::Protocol(_)) => break,
                Err(_) => return None,
            }
        }
        got_any.then_some(stream)
    }

    /// settings id of caps → current value. None on failure (not shown).
    fn read_setting(&self, id: &str) -> Option<SettingValue> {
        match id {
            // [2,5,1] → r[3] = wire code
            "polling" => self
                .cmd(grp::GLOBAL, gfs::REPORT_RATE, &[1])
                .ok()
                .map(|r| SettingValue::Choice(r[3] as i64)),
            // [2,4,1] → r[3] = 0 Win / 1 Mac
            "system_type" => self
                .cmd(grp::GLOBAL, gfs::SYSTEM_SETTING, &[1])
                .ok()
                .map(|r| SettingValue::Choice(r[3] as i64)),
            // [2,16,1] → r[3] = status (1=enabled)
            "shake_opt" => self
                .cmd(grp::GLOBAL, gfs::SHAKE_OPT, &[1])
                .ok()
                .map(|r| SettingValue::Bool(r[3] == 1)),
            // [2,13,1] → r[3,4] u16 minutes
            "light_sleep" => self
                .cmd(grp::GLOBAL, gfs::SLEEP_TIME, &[1])
                .ok()
                .map(|r| SettingValue::Num(u16::from_le_bytes([r[3], r[4]]) as f32)),
            _ => None,
        }
    }

    fn write_setting(&self, id: &str, v: &SettingValue) -> HubResult<()> {
        match id {
            "polling" => {
                self.cmd(grp::GLOBAL, gfs::REPORT_RATE, &[2, v.as_choice() as u8])?;
            }
            "system_type" => {
                self.cmd(grp::GLOBAL, gfs::SYSTEM_SETTING, &[2, v.as_choice() as u8])?;
            }
            "shake_opt" => {
                self.cmd(grp::GLOBAL, gfs::SHAKE_OPT, &[0, v.as_bool() as u8])?;
            }
            "light_sleep" => {
                let m = (v.as_num().clamp(0.0, 120.0) as u16).to_le_bytes();
                self.cmd(grp::GLOBAL, gfs::SLEEP_TIME, &[0, m[0], m[1]])?;
            }
            _ => return Err(HubError::Unsupported("setting")),
        }
        Ok(())
    }

    /// Current config index `[2,3,1]` → r[3].
    fn read_profile(&self) -> HubResult<ProfileState> {
        let r = self.cmd(grp::GLOBAL, gfs::CONFIG_SWITCH, &[1])?;
        let active = r[3].min(self.xs.configs.saturating_sub(1));
        let mut names = Vec::new();
        for i in 0..self.xs.configs {
            // [2,3,3,i] → r[4..36] UTF-8
            let name = self
                .cmd(grp::GLOBAL, gfs::CONFIG_SWITCH, &[3, i])
                .ok()
                .map(|r| {
                    let end = r[4..36].iter().position(|&b| b == 0).unwrap_or(32);
                    String::from_utf8_lossy(&r[4..4 + end]).trim().to_string()
                })
                .unwrap_or_default();
            names.push(name);
        }
        Ok(ProfileState { active, names })
    }

    /// SaveParam `[2,2,scope]` — flash save after writing.
    /// The real device sometimes returns an old value when read back right after save, so this waits for it to take effect before returning.
    fn save(&self, scope: u8) -> HubResult<()> {
        self.cmd(grp::GLOBAL, gfs::SAVE_PARAM, &[scope])?;
        std::thread::sleep(Duration::from_millis(80));
        Ok(())
    }

    /// Factory reset by scope `[2,1,scope]`.
    fn reset_scope(&self, scope: u8, label: &str) -> HubResult<ActionOutcome> {
        self.cmd(grp::GLOBAL, gfs::RESET_FACTORY, &[scope])?;
        Ok(ActionOutcome {
            message: Some(format!("{label} reset to defaults")),
            reload: true,
        })
    }

    /// perf response (r[4..]) → KeyPerf. u16 values are mm×1000 LE.
    /// mode: 0=Normal, 1=RT (continuous RT), 2=IntervalRT (non-continuous RT) — follows the official enum vn.
    fn decode_perf(r: &[u8; 64]) -> KeyPerf {
        let mm = |i: usize| u16::from_le_bytes([r[i], r[i + 1]]) as f32 / 1000.0;
        KeyPerf {
            mode: if r[4] == 0 {
                TriggerMode::Normal
            } else {
                TriggerMode::Rapid
            },
            actuation: mm(5),
            release: Some(mm(7)),
            rt_first: mm(9),
            rt_press: mm(11),
            rt_release: mm(13),
            top_deadzone: mm(15),
            bottom_deadzone: mm(17),
            continuous: r[4] == 1,
            switch_type: r[19],
        }
    }

    /// `[4,1,row,col]` → KeyPerf + trailing fields (axis/calib/axisV2Id/coef/rangeMax).
    /// Keeps the raw response for fields that must be preserved at write time.
    /// The response echoes r[2]=row, r[3]=col. To avoid mistaking a group-4 response of another sub
    /// (axis data) mixed in by live travel etc., it is matched, and if it is off it is read again once.
    fn read_perf(&self, row: u8, col: u8) -> HubResult<(KeyPerf, [u8; 64])> {
        let mut r = self.cmd(grp::PERFORMANCE, sub::GET_PERF, &[row, col])?;
        if r[2] != row || r[3] != col {
            r = self.cmd(grp::PERFORMANCE, sub::GET_PERF, &[row, col])?;
        }
        Ok((Self::decode_perf(&r), r))
    }

    /// `[4,2,row,col,...]` — SDK layout:
    /// mode, nP, nR, rtFirst, rtPress, rtRelease, pDead, rDead (each u16 LE mm×1000),
    /// axis, calibrate, axisV2Id, axisRangeMax, axisCoefficient (the last 3 keep the read values).
    fn write_perf(&self, row: u8, col: u8, p: &KeyPerf) -> HubResult<()> {
        let (_, cur) = self.read_perf(row, col)?; // read first to preserve the axis fields
        let mm = |v: f32| (v * 1000.0).round().clamp(0.0, 65535.0) as u16;
        let mut args = vec![
            row,
            col,
            // Official vn: Normal=0, RT (continuous)=1, IntervalRT (non-continuous)=2
            match p.mode {
                TriggerMode::Normal => 0,
                TriggerMode::Rapid if p.continuous => 1,
                TriggerMode::Rapid => 2,
            },
        ];
        for v in [
            mm(p.actuation),
            mm(p.release.unwrap_or(p.actuation)),
            mm(p.rt_first),
            mm(p.rt_press),
            mm(p.rt_release),
            mm(p.top_deadzone),
            mm(p.bottom_deadzone),
        ] {
            args.extend_from_slice(&v.to_le_bytes());
        }
        args.push(p.switch_type); // axis
                                  // Official layout: axis + calibrate + axisV2Id(2) + axisRangeMax(2) + axisCoefficient(2).
                                  // The official app always sends calibrate as 0 (no calibration-in-progress flag is left).
                                  // v2Id/rangeMax/coef keep the read values.
        args.push(0);
        args.extend_from_slice(&cur[21..27]);
        self.cmd(grp::PERFORMANCE, sub::SET_PERF, &args)?;
        Ok(())
    }

    /// If it is a `[4,3,kind,row]` frame, puts it in the cache and returns true.
    /// The frame content is 30×u16 LE.
    fn note_axis_frame(&self, b: &[u8]) -> bool {
        if b.len() >= 64 && b[0] == grp::PERFORMANCE && b[1] == sub::AXIS_DATA {
            let mut out = [0u16; 30];
            for i in 0..30 {
                out[i] = u16::from_le_bytes([b[4 + i * 2], b[5 + i * 2]]);
            }
            self.axis_cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert((b[2], b[3]), (Instant::now(), out));
            true
        } else {
            false
        }
    }

    /// Lightly drains the input queue and caches the axis frames that are flowing.
    /// HID input reports fan out to every open handle, so the responses to requests sent by another process
    /// such as the official HUB arrive here too — by piggybacking, we need not issue requests ourselves,
    /// and we do not mix other people's frames into the official HUB's response queue.
    /// If only "axis frames that I have not requested recently" are flowing,
    /// or known non-axis group frames (= responses to someone else's commands — the official app
    /// does not verify the echo and takes the first frame as the response, so mixing is fatal) are flowing,
    /// it is judged as external traffic and passive mode is extended (return value = external detected).
    fn drain_axis(&self, budget_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(budget_ms);
        let mut saw_any = false;
        let mut saw_ours = false;
        let mut saw_foreign = false;
        while let Ok(b) = self.ch.recv(0) {
            if self.note_axis_frame(&b) {
                saw_any = true;
                let ours = self
                    .axis_self
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&(b[2], b[3]))
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(500));
                saw_ours |= ours;
            } else if self.is_foreign_cmd(&b) {
                // Another's command response that is not an axis frame (those matching my recent
                // sends are already excluded).
                saw_foreign = true;
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        // Go passive on external detection. The device-side stream has already been stopped by
        // the other's command (any non-axis command stops it), so we send
        // nothing from our side — sending would mix its response into the official queue, which backfires.
        if saw_foreign || (saw_any && !saw_ours) {
            self.note_foreign();
            return true;
        }
        false
    }

    /// Whether a non-axis frame is "someone else's command response": a known group, not an error response,
    /// and not a response to the same (grp,sub) that I sent recently.
    fn is_foreign_cmd(&self, b: &[u8]) -> bool {
        (1..=13).contains(&b[0])
            && b.get(1) != Some(&0xFF)
            && !(b[0] == grp::PERFORMANCE && b[1] == sub::AXIS_DATA)
            && self
                .cmd_self
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&(b[0], b[1]))
                .is_none_or(|t| t.elapsed() > Duration::from_millis(800))
    }

    /// Records external traffic and extends passive mode.
    fn note_foreign(&self) {
        *self.axis_passive.lock().unwrap_or_else(|p| p.into_inner()) =
            Instant::now() + Duration::from_secs(8);
    }

    /// `[4,3,kind,row]` → 30×u16 LE (mm×1000). kind: axis::* (ADC=0,Route=1,Calibrate=2,KeyStatus=3).
    /// The real device autonomously streams live axis data as `[4,3,kind,row]` frames
    /// (cycling through every kind×row pair in ~16ms). With cmd()'s receive that "treats the first group-4 packet as
    /// the response", a frame of another kind/row is picked up and the values drift (the cause of ADC values
    /// leaking into the status/route fields and the OBS overlay flickering).
    /// → Keep reading until a frame whose echo (r[2]=kind, r[3]=row) matches.
    fn axis_row(&self, row: u8, kind: u8) -> HubResult<[u16; 30]> {
        let probing = self.fails.load(Ordering::Relaxed) >= FAIL_FAST_AFTER;
        if probing {
            let mut ds = self.dead_since.lock().unwrap_or_else(|p| p.into_inner());
            match *ds {
                Some(t) if t.elapsed() < Duration::from_millis(PROBE_MS) => {
                    return Err(HubError::Disconnected)
                }
                _ => *ds = Some(Instant::now()),
            }
        }
        self.ch
            .send(&[grp::PERFORMANCE, sub::AXIS_DATA, kind, row])?;
        // Record my own request — used by drain to tell it apart from external traffic.
        self.axis_self
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((kind, row), Instant::now());
        let wait_ms = if probing { PROBE_WAIT_MS } else { DEADLINE_MS };
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        while Instant::now() < deadline {
            match self.ch.recv(POLL_MS) {
                Ok(b) => {
                    // Axis frames go into the cache even if they are not the echo
                    // (the one requested also makes frames of other rows/kinds flow).
                    if self.note_axis_frame(&b) {
                        if b[2] == kind && b[3] == row {
                            self.fails.store(0, Ordering::Relaxed);
                            *self.dead_since.lock().unwrap_or_else(|p| p.into_inner()) = None;
                            return Ok(self
                                .axis_cache
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .get(&(kind, row))
                                .map(|(_, v)| *v)
                                .unwrap_or([0; 30]));
                        }
                        continue;
                    }
                    if b.first().copied() == Some(grp::PERFORMANCE)
                        && b.get(1).copied() == Some(0xFF)
                    {
                        return Err(HubError::Unsupported("command"));
                    }
                    if self.is_foreign_cmd(&b) {
                        // Someone else's command response → go passive (the device side was already released by
                        // that command; we send nothing).
                        self.note_foreign();
                    }
                }
                Err(HubError::Protocol(_)) => {}
                Err(e) => return Err(e),
            }
        }
        let f = self.fails.fetch_add(1, Ordering::Relaxed) + 1;
        if f >= FAIL_FAST_AFTER {
            *self.dead_since.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
            return Err(HubError::Disconnected);
        }
        Err(HubError::Protocol("xsyd: axis response timeout".into()))
    }

    /// Advanced key read `[6,1,row,col,0]` → decodes Advanced if mode!=NONE.
    fn read_higher(&self, layout_index: usize) -> HubResult<Option<Advanced>> {
        let key = &registry::su66max_layout()[layout_index];
        let (row, col) = registry::su66max_matrix_pos(key.id);
        let r = self.cmd(grp::HIGHER_KEY, sub::READ, &[row, col, hk::NONE])?;
        Ok(decode_higher(&r))
    }

    /// Advanced key write `[6,2,row,col,mode,...]`. SOCD/RS take 2 packets.
    /// `binding=None` clears it (mode=NONE).
    fn write_higher(&self, layout_index: usize, binding: Option<&Advanced>) -> HubResult<()> {
        let key = &registry::su66max_layout()[layout_index];
        let (row, col) = registry::su66max_matrix_pos(key.id);
        for p in higher_packets(row, col, binding)? {
            self.cmd(grp::HIGHER_KEY, sub::SETTING, &p)?;
        }
        Ok(())
    }

    /// Macro mode `[7,1,id]` → (valid, actNum, repNum, mode).
    fn read_macro_mode(&self, id: u8) -> HubResult<(bool, u16, u16, u8)> {
        let r = self.cmd(grp::MACRO, sub::GET_MACRO_MODE, &[id])?;
        Ok((
            r[3] == 1,
            u16::from_le_bytes([r[4], r[5]]),
            u16::from_le_bytes([r[6], r[7]]),
            r[8],
        ))
    }

    /// Macro data `[7,3,id,offset]` — fetches 15 events at a time, by offset.
    fn read_macro_events(&self, id: u8, count: u16) -> HubResult<Vec<MacroEvent>> {
        let mut out = Vec::new();
        let mut offset = 0u8;
        while (out.len() as u16) < count.min(240) {
            let r = self.cmd(grp::MACRO, sub::GET_MACRO, &[id, offset])?;
            for i in 0..15 {
                let w =
                    u32::from_le_bytes([r[4 + i * 4], r[5 + i * 4], r[6 + i * 4], r[7 + i * 4]]);
                let keycode = w & 0xFFFF;
                let delay = (w >> 16) & 0x7FFF;
                let down = (w >> 31) & 1;
                if keycode == 0 && delay == 0 && down == 0 {
                    break;
                }
                out.push(MacroEvent {
                    input: MacroInput::Key(keycode as u16),
                    edge: if down == 1 {
                        MacroEdge::Down
                    } else {
                        MacroEdge::Up
                    },
                    delay_ms: delay,
                });
                if out.len() as u16 >= count {
                    break;
                }
            }
            offset += 1;
            if r[3] != offset - 1 {
                break; // offset mismatch = end of data
            }
        }
        Ok(out)
    }

    /// Macro write. Mode packet → data packets (15 events at a time), in that order.
    fn write_macro(&self, slot: u8, def: Option<&MacroDef>) -> HubResult<()> {
        match def {
            None => {
                self.cmd(grp::MACRO, sub::SET_MACRO_MODE, &[slot, 0, 0, 0, 0, 0, 0])?;
            }
            Some(d) => {
                let act = (d.events.len() as u16).to_le_bytes();
                let rep = d.repeat.to_le_bytes();
                self.cmd(
                    grp::MACRO,
                    sub::SET_MACRO_MODE,
                    &[slot, 1, act[0], act[1], rep[0], rep[1], d.mode],
                )?;
                for (i, chunk) in d.events.chunks(15).enumerate() {
                    let mut args = vec![slot, i as u8];
                    for ev in chunk {
                        let key = match ev.input {
                            MacroInput::Key(k) => k as u32,
                            MacroInput::Mouse(f) => f as u32,
                        };
                        let w = ((ev.edge == MacroEdge::Down) as u32) << 31
                            | ev.delay_ms.min(32767) << 16
                            | (key & 0xFFFF);
                        args.extend_from_slice(&w.to_le_bytes());
                    }
                    self.cmd(grp::MACRO, sub::SET_MACRO, &args)?;
                }
            }
        }
        Ok(())
    }

    /// Lays out the route values (mm) within the full stroke in layout index order.
    fn route_mm(&self) -> HubResult<Vec<f32>> {
        let layout = registry::su66max_layout();
        let mut out = vec![0.0f32; layout.len()];
        for row in 1..=5u8 {
            match self.axis_row(row, axis::ROUTE) {
                Ok(vals) => {
                    for (i, k) in layout.iter().enumerate() {
                        let (kr, kc) = registry::su66max_matrix_pos(k.id);
                        if kr == row {
                            out[i] = vals[kc as usize] as f32 / 1000.0;
                        }
                    }
                }
                Err(HubError::Unsupported(_)) | Err(HubError::Protocol(_)) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// `[5,1,area,0]` → basic lighting (SDK lightingBaseData): r[4]=open, r[5]=mode
    /// (0=solid color, 1..16=dynamic = Y1 enum), r[6]=brightness 0-100,
    /// r[7]=speed 0-100, r[8]=direction (0=Forward,1=Backward), r[9]=selected palette color.
    /// open is the jo enum on DoubleLighting devices (0=Close,1=OpenDown,2=OpenUp,3=Open),
    /// and a boolean on SingleLighting (0=Close, nonzero=Open). Either way nonzero is read as lit.
    /// The solid color is taken from the selected slot of the palette `[5,1,area,1]`.
    fn read_lighting(&self, area: u8) -> HubResult<LightingState> {
        let r = self.cmd(grp::LIGHTING, sub::READ, &[area, 0])?;
        let sel = (r[9] as usize).min(7);
        // Palette: [5,1,area,1] → r[4+4i..] = [B,G,R,H]×8
        let palette: Vec<Rgb> = self
            .cmd(grp::LIGHTING, sub::READ, &[area, 1])
            .map(|p| {
                (0..8)
                    .map(|i| Rgb::new(p[6 + i * 4], p[5 + i * 4], p[4 + i * 4]))
                    .collect()
            })
            .unwrap_or_default();
        let color = palette.get(sel).copied().unwrap_or(Rgb::new(255, 255, 255));
        // Saturation (ColorCorrection): [5,1,area,2] → r[4,5,6] = R,G,B (official LightingSaturation).
        let color_correction = self
            .cmd(grp::LIGHTING, sub::READ, &[area, 2])
            .ok()
            .map(|c| Rgb::new(c[4], c[5], c[6]));
        let mut per_key = BTreeMap::new();
        if area == 0 {
            // [5,3,0,chunk] → 15 cells×4B (B,G,R,isCustom). 9 chunks=135 cells.
            let layout = registry::su66max_layout();
            let mut cell_key = [usize::MAX; 135];
            for (i, k) in layout.iter().enumerate() {
                if let Some(&cell) = led_cells(k).first() {
                    if cell < 135 {
                        cell_key[cell] = i;
                    }
                }
            }
            // The readback of DD pins only has the preview color. The real custom color is read from the saved copy.
            let ddp = self
                .dd_pins
                .lock()
                .map(|p| p.get(&area).cloned().unwrap_or_default())
                .unwrap_or_default();
            let saved = self
                .dd_pre
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&area)
                .cloned()
                .unwrap_or_default();
            // All chunks are required — if even 1 chunk drops, it is a read failure, not "there is no custom color
            // in that range". Returning a partial read as a complete state would make the later write_per_key
            // (unspecified cells are erased with isCustom=0) wipe the colors on the real device.
            for c in 0..9u8 {
                let rb = match self.read_dd_chunk(0, c) {
                    Ok(rb) => rb,
                    Err(HubError::Unsupported(_)) => break,
                    Err(e) => return Err(e),
                };
                for i in 0..15usize {
                    let cell = c as usize * 15 + i;
                    let bgr = if ddp.contains_key(&cell) {
                        saved.get(&cell).copied()
                    } else {
                        (rb[7 + i * 4] == 255)
                            .then(|| [rb[4 + i * 4], rb[5 + i * 4], rb[6 + i * 4]])
                    };
                    if let (Some([b, g, r]), true) = (bgr, cell_key[cell] != usize::MAX) {
                        per_key.insert(cell_key[cell], Rgb::new(r, g, b));
                    }
                }
            }
        }
        // open byte: Keyboard on DoubleLighting devices → jo {0=Close,1=OpenDown,2=OpenUp,3=Open},
        // otherwise boolean. Either way nonzero is read as lit.
        let open = r[4];
        let zone_on = if area == 0 && self.xs.double_lighting {
            vec![open == 2 || open == 3, open == 1 || open == 3]
        } else {
            vec![]
        };
        Ok(LightingState {
            on: open != 0,
            // The mode byte is the index into the mode list as is (0..19).
            effect: r[5].min(19),
            color,
            brightness: r[6].min(100),
            speed: r[7].min(100),
            direction: r[8].min(1),
            per_key,
            zone_on,
            palette,
            palette_sel: r[9].min(7),
            color_correction,
        })
    }

    /// `[5,4,0,chunk,…15 cells×4B]` — writes custom colors.
    /// Cells included in `per_key` are written with isCustom=255, others with isCustom=0
    /// (`per_key` is the absolute custom state — unspecified cells are cleared.
    /// As in the official app, color bytes keep the existing values and only the flag changes).
    fn write_per_key(&self, per_key: &BTreeMap<usize, Rgb>) -> HubResult<()> {
        let layout = registry::su66max_layout();
        // Read the current 135 cells (to keep the color bytes of unspecified cells + detect changes)
        let mut cells = [[0u8; 4]; 135];
        for c in 0..9u8 {
            if let Ok(rb) = self.cmd(grp::LIGHTING, sub::LIGHT_READBACK, &[0, c]) {
                for i in 0..15usize {
                    cells[c as usize * 15 + i].copy_from_slice(&rb[4 + i * 4..8 + i * 4]);
                }
            }
        }
        let prev = cells;
        for c in cells.iter_mut() {
            c[3] = 0;
        }
        for (&idx, rgb) in per_key {
            if let Some(k) = layout.get(idx) {
                for cell in led_cells(k) {
                    if cell < 135 {
                        cells[cell] = [rgb.b, rgb.g, rgb.r, 255];
                    }
                }
            }
        }
        // Send only the chunks that changed
        for c in 0..9u8 {
            let lo = c as usize * 15;
            let hi = lo + 15;
            if cells[lo..hi] == prev[lo..hi] {
                continue;
            }
            let mut args = vec![0u8, c];
            for cell in &cells[lo..hi] {
                args.extend_from_slice(cell);
            }
            self.cmd(grp::LIGHTING, sub::LIGHT_CUSTOM, &args)?;
        }
        Ok(())
    }

    /// `[5,2,area,0, open, mode, brightness, speed, direction, palette selection]` (SDK setLightingBase).
    /// mode is an index into the mode list (0..19). Custom color is not a mode but an overlay of
    /// isCustom cells, so mode is not touched.
    /// open: Keyboard (area 0) is DoubleLighting — the upper/lower zones are opened and closed
    /// individually with the jo enum (`zone_on = [upper, lower]` → both=3, upper only=2, lower only=1, both closed=0).
    /// Without `zone_on`, on → 3 (both zones lit). Other areas use a boolean (0/1).
    /// Color is a palette — writes `palette_sel` as the selected slot, and if `palette` is
    /// passed, applies all slots, otherwise applies `color` to the selected slot.
    /// Builds the open byte from the state. Keyboard (area 0) + DoubleLighting uses
    /// the jo enum (`zone_on=[upper,lower]` → both=3, upper only=2, lower only=1, closed=0),
    /// on→3 if there is no `zone_on`. Other areas use a boolean (0/1).
    fn compose_open(&self, area: u8, s: &LightingState) -> u8 {
        if area == 0 && self.xs.double_lighting {
            if s.zone_on.len() >= 2 {
                match (s.zone_on[0], s.zone_on[1]) {
                    (true, true) => 3,
                    (true, false) => 2,
                    (false, true) => 1,
                    (false, false) => 0,
                }
            } else {
                if s.on {
                    3
                } else {
                    0
                }
            }
        } else {
            u8::from(s.on)
        }
    }

    fn write_lighting(&self, area: u8, s: &LightingState) -> HubResult<()> {
        let cur = self.cmd(grp::LIGHTING, sub::READ, &[area, 0])?;
        let open = self.compose_open(area, s);
        // Do not write if the base is the same as the current one (fewer HID round trips by the amount not written).
        let base = [
            open,
            s.effect.min(19),
            s.brightness.min(100),
            s.speed.min(100),
            s.direction.min(1),
            s.palette_sel.min(7),
        ];
        if cur[4..10] != base {
            let mut args = vec![area, 0];
            args.extend_from_slice(&base);
            self.cmd(grp::LIGHTING, sub::SETTING, &args)?;
        }
        // Palette ([B,G,R,H]×8): s.color goes into the selected slot, and the per-slot colors
        // of s.palette are reflected too if present. Do not write if the same as the current one.
        if let Ok(pal) = self.cmd(grp::LIGHTING, sub::READ, &[area, 1]) {
            let mut bytes = pal[4..36].to_vec();
            for (i, c) in s.palette.iter().take(8).enumerate() {
                bytes[i * 4] = c.b;
                bytes[i * 4 + 1] = c.g;
                bytes[i * 4 + 2] = c.r;
            }
            let sel = s.palette_sel.min(7) as usize;
            bytes[sel * 4] = s.color.b;
            bytes[sel * 4 + 1] = s.color.g;
            bytes[sel * 4 + 2] = s.color.r;
            if bytes != pal[4..36] {
                let mut args = vec![area, 1];
                args.extend_from_slice(&bytes);
                self.cmd(grp::LIGHTING, sub::SETTING, &args)?;
            }
        }
        // Saturation (ColorCorrection): [5,2,area,2,R,G,B]. Do not write if the same as the current one.
        if let Some(cc) = s.color_correction {
            if let Ok(cur) = self.cmd(grp::LIGHTING, sub::READ, &[area, 2]) {
                if cur[4] != cc.r || cur[5] != cc.g || cur[6] != cc.b {
                    self.cmd(
                        grp::LIGHTING,
                        sub::SETTING,
                        &[area, 2, cc.r.min(100), cc.g.min(100), cc.b.min(100)],
                    )?;
                }
            }
        }
        Ok(())
    }

    /// `import_config` — writes the whole DeviceState back (each section uses the existing
    /// write_*, saving per section). Profile switching is not performed.
    fn import_state(&self, st: &DeviceState) -> HubResult<()> {
        let layout = registry::su66max_layout();
        if let Some(kb) = &st.keyboard {
            // Keymap: build the stream per layer and write in bulk.
            for (layer, actions) in kb.keymap.iter().enumerate().take(self.xs.layers.into()) {
                let mut stream = vec![1u16; 8 * 30];
                for (i, a) in actions.iter().enumerate() {
                    let Some(k) = layout.get(i) else { break };
                    let (row, col) = registry::su66max_matrix_pos(k.id);
                    stream[row as usize * 30 + col as usize] = action_to_code(a).unwrap_or(1);
                }
                self.write_stream(layer as u8, &stream)?;
            }
            self.save(scope::LAYOUT)?;
            // Per-key performance.
            for (i, p) in kb.perf.iter().enumerate() {
                let Some(k) = layout.get(i) else { break };
                let (row, col) = registry::su66max_matrix_pos(k.id);
                self.write_perf(row, col, p)?;
            }
            self.save(scope::PERFORMANCE)?;
            // Advanced keys: absolute state (keys absent from the file are cleared).
            let bound: BTreeMap<usize, &Advanced> =
                kb.advanced.iter().map(|a| (a.key, &a.binding)).collect();
            for i in 0..layout.len() {
                self.write_higher(i, bound.get(&i).copied())?;
            }
            self.save(scope::HIGHER_KEY)?;
            // Macros.
            for (i, m) in kb.macros.iter().enumerate() {
                self.write_macro(i as u8, m.as_ref())?;
            }
            self.save(scope::MACRO)?;
        }
        for (area, s) in &st.lighting {
            if !self.xs.light_areas.contains(area) {
                continue;
            }
            if *area == 0 {
                self.write_per_key(&s.per_key)?;
            }
            self.write_lighting(*area, s)?;
        }
        self.save(scope::LIGHTING)?;
        for (id, v) in &st.settings {
            self.write_setting(id, v)?;
        }
        Ok(())
    }

    /// Export `keyboard_config.json` of the official configurator (hub.xsyd.top).
    /// `{"data": "<base64>"}` is CryptoJS `AES.encrypt(json, passphrase)` —
    /// AES-256-CBC in OpenSSL `Salted__` format + EVP_BytesToKey(MD5).
    /// The passphrase is hardcoded in the official SDK bundle.
    /// Plain JSON that has `keyboards` is returned as is.
    fn official_plain(&self, v: &serde_json::Value) -> HubResult<serde_json::Value> {
        use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
        use base64::Engine;
        const PASS: &[u8] = b"fDPy6vvnpPsYm2T0g1bh";
        let Some(b64) = v.get("data").and_then(|d| d.as_str()) else {
            return Ok(v.clone());
        };
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| HubError::Invalid(format!("encrypted data: {e}")))?;
        if raw.len() < 48 || &raw[..8] != b"Salted__" {
            return Err(HubError::Invalid("Encrypted data format is invalid".into()));
        }
        // EVP_BytesToKey: D_i = MD5(D_{i-1} | pass | salt) concatenated → key 32B + iv 16B
        use md5::{Digest, Md5};
        let mut keyiv = Vec::with_capacity(48);
        let mut prev: Vec<u8> = Vec::new();
        while keyiv.len() < 48 {
            let mut h = Md5::new();
            h.update(&prev);
            h.update(PASS);
            h.update(&raw[8..16]);
            prev = h.finalize().to_vec();
            keyiv.extend_from_slice(&prev);
        }
        let mut ct = raw[16..].to_vec();
        let pt = cbc::Decryptor::<aes::Aes256>::new_from_slices(&keyiv[..32], &keyiv[32..48])
            .map_err(|e| HubError::Invalid(format!("AES: {e}")))?
            .decrypt_padded::<Pkcs7>(&mut ct)
            .map_err(|_| HubError::Invalid("Decryption failed".into()))?;
        serde_json::from_slice(pt).map_err(|e| HubError::Invalid(format!("decrypted JSON: {e}")))
    }

    /// Official export → DeviceState (the write is left to `import_state`).
    /// - Keymap: `customKeys.win.fn{L}.bindKeyValue` → layer L (wire code as is)
    /// - Advanced keys: `advancedKeys.advancedType` → Advanced
    /// - Performance: `performance` as is for each key (the official app also writes all keys)
    /// - Lighting: `v2light.{Keyboard,DecorateN}` — unspecified items such as palette colors keep the current values
    /// - Macros: `macro.v2list` (invalid slots are cleared)
    /// - Settings: `system.rateOfReturn`/`currentKeyboardSystem`
    ///
    /// The MAC-side keymap is not written (the file's mac section holds template values).
    fn official_to_state(&self, v: &serde_json::Value) -> HubResult<DeviceState> {
        let layout = registry::su66max_layout();
        // pid/vid check — do not write a file of a different model (the official app does the same check)
        if let Some(s) = v.get("system") {
            let num = |k: &str| s.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let (vid, pid) = (num("vid"), num("pid"));
            if (vid, pid) != (self.info.vid as u32, self.info.pid as u32) {
                return Err(HubError::Invalid(format!(
                    "This file is for a different keyboard model ({vid:04x}:{pid:04x})"
                )));
            }
        }
        // (row, col) → key entry
        let mut grid = std::collections::HashMap::new();
        if let Some(rows) = v.get("keyboards").and_then(|k| k.as_array()) {
            for ko in rows.iter().flat_map(|r| r.as_array().into_iter().flatten()) {
                if let (Some(r), Some(c)) = (
                    ko.get("row").and_then(|x| x.as_u64()),
                    ko.get("col").and_then(|x| x.as_u64()),
                ) {
                    grid.insert((r as u8, c as u8), ko);
                }
            }
        }
        if grid.is_empty() {
            return Err(HubError::Invalid("No keyboard data".into()));
        }
        // ---- Keymap (win.fn0-3 → layers 0-3) ----
        let nlayers = (self.xs.layers as usize).min(4);
        let keymap: Vec<Vec<KeyAction>> = (0..nlayers)
            .map(|l| {
                layout
                    .iter()
                    .map(|k| {
                        let (row, col) = registry::su66max_matrix_pos(k.id);
                        let code = grid
                            .get(&(row, col))
                            .and_then(|ko| official_bind(ko, l))
                            .unwrap_or(1);
                        code_to_action(code)
                    })
                    .collect()
            })
            .collect();
        // ---- Advanced keys ----
        let mut advanced = Vec::new();
        for (i, k) in layout.iter().enumerate() {
            let (row, col) = registry::su66max_matrix_pos(k.id);
            let Some(ak) = grid.get(&(row, col)).and_then(|ko| ko.get("advancedKeys")) else {
                continue;
            };
            if let Some(binding) = official_advanced(ak, &layout) {
                advanced.push(AdvancedKey {
                    key: i,
                    layer: 0,
                    binding,
                });
            }
        }
        // ---- Per-key performance (the official app also writes all keys. The axis fields keep the current values via write_perf) ----
        let perf: Vec<KeyPerf> = layout
            .iter()
            .map(|k| {
                let (row, col) = registry::su66max_matrix_pos(k.id);
                let p = grid.get(&(row, col)).and_then(|ko| ko.get("performance"));
                let num = |n: &str| {
                    p.and_then(|x| x.get(n))
                        .and_then(|x| x.as_f64())
                        .unwrap_or(0.)
                };
                // File values are already in mm (the official app multiplies by ×1000 when sending over the wire)
                let mm = |n: &str| num(n) as f32;
                let mode = num("mode") as u8;
                KeyPerf {
                    mode: if mode == 0 {
                        TriggerMode::Normal
                    } else {
                        TriggerMode::Rapid
                    },
                    continuous: mode == 1,
                    actuation: mm("normalPress"),
                    release: Some(mm("normalRelease")),
                    rt_first: mm("rtFirstTouch"),
                    rt_press: mm("rtPress"),
                    rt_release: mm("rtRelease"),
                    top_deadzone: mm("pressDeadStroke"),
                    bottom_deadzone: mm("releaseDeadStroke"),
                    switch_type: num("axis") as u8,
                }
            })
            .collect();
        // ---- Macros (v2list) ----
        let mut macros = Vec::new();
        if let Some(list) = v
            .get("macro")
            .and_then(|m| m.get("v2list"))
            .and_then(|m| m.as_array())
        {
            for m in list
                .iter()
                .take(self.xs.macro_space.map(|x| x.0).unwrap_or(16).into())
            {
                let valid = m.get("valid").and_then(|x| x.as_bool()).unwrap_or(false);
                let events: Vec<MacroEvent> = if valid {
                    m.get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|e| {
                                    let kc = e
                                        .get("keyCode")
                                        .or_else(|| e.get("keycode"))
                                        .and_then(|x| x.as_u64())?
                                        as u16;
                                    Some(MacroEvent {
                                        input: MacroInput::Key(kc),
                                        edge: if e
                                            .get("status")
                                            .and_then(|x| x.as_u64())
                                            .unwrap_or(0)
                                            == 1
                                        {
                                            MacroEdge::Down
                                        } else {
                                            MacroEdge::Up
                                        },
                                        delay_ms: e
                                            .get("delay")
                                            .and_then(|x| x.as_u64())
                                            .unwrap_or(0)
                                            .min(32767)
                                            as u32,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    vec![]
                };
                macros.push(if valid && !events.is_empty() {
                    Some(MacroDef {
                        name: m.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
                        mode: m.get("mode").and_then(|x| x.as_u64()).unwrap_or(0) as u8,
                        repeat: m.get("repNum").and_then(|x| x.as_u64()).unwrap_or(0) as u16,
                        events,
                    })
                } else {
                    None
                });
            }
        }
        // ---- Lighting (v2light) ----
        let mut lighting = BTreeMap::new();
        if let Some(areas) = v.get("v2light").and_then(|x| x.as_object()) {
            for (name, ad) in areas {
                let area: u8 = match name.as_str() {
                    "Keyboard" => 0,
                    n if n.starts_with("Decorate") => {
                        n.trim_start_matches("Decorate").parse().unwrap_or(255)
                    }
                    _ => continue,
                };
                if !self.xs.light_areas.contains(&area) {
                    continue;
                }
                let light = &ad["light"];
                // Items absent from the file (palette colors, saturation) keep the current values as the base
                let mut s = self.read_lighting(area).unwrap_or_else(|_| LightingState {
                    on: true,
                    effect: 0,
                    color: Rgb::new(255, 255, 255),
                    brightness: 80,
                    speed: 80,
                    direction: 0,
                    per_key: BTreeMap::new(),
                    zone_on: vec![],
                    palette: vec![],
                    palette_sel: 0,
                    color_correction: None,
                });
                let zones: [bool; 2] = match light.get("open").and_then(|x| x.as_str()) {
                    Some("Open") => [true, true],
                    Some("OpenUp") => [true, false],
                    Some("OpenDown") => [false, true],
                    _ => [false, false],
                };
                s.on = zones[0] || zones[1];
                if s.zone_on.len() >= 2 {
                    s.zone_on = zones.to_vec();
                }
                let num = |n: &str| light.get(n).and_then(|x| x.as_u64());
                if let Some(m) = num("mode") {
                    s.effect = m as u8;
                }
                if let Some(m) = num("luminance") {
                    s.brightness = m as u8;
                }
                if let Some(m) = num("speed") {
                    s.speed = m as u8;
                }
                s.direction = match light.get("direction").and_then(|x| x.as_str()) {
                    Some("Backward") => 1,
                    _ => 0,
                };
                if let Some(m) = num("selectStaticColor") {
                    s.palette_sel = (m as u8).min(7);
                    // Changing only sel makes write_lighting overwrite the new slot
                    // with the old slot's color — match the color of the selection target.
                    if let Some(c) = s.palette.get(s.palette_sel as usize).copied() {
                        s.color = c;
                    }
                }
                // Per-key custom colors → per_key (isCustom only)
                if area == 0 {
                    s.per_key = layout
                        .iter()
                        .enumerate()
                        .filter_map(|(i, k)| {
                            let (row, col) = registry::su66max_matrix_pos(k.id);
                            let cell =
                                grid.get(&(row, col)).and_then(|ko| ko.get("customLight"))?;
                            cell.get("isCustom")
                                .and_then(|x| x.as_bool())
                                .unwrap_or(false)
                                .then(|| {
                                    let u = |n: &str| {
                                        cell.get(n).and_then(|x| x.as_u64()).unwrap_or(0) as u8
                                    };
                                    (i, Rgb::new(u("R"), u("G"), u("B")))
                                })
                        })
                        .collect();
                }
                lighting.insert(area, s);
            }
        }
        // ---- General settings ----
        let mut settings = BTreeMap::new();
        if let Some(s) = v.get("system") {
            if let Some(r) = s.get("rateOfReturn").and_then(|x| x.as_u64()) {
                settings.insert("polling".into(), SettingValue::Choice(r as i64));
            }
            if let Some(t) = s.get("currentKeyboardSystem").and_then(|x| x.as_str()) {
                settings.insert(
                    "system_type".into(),
                    SettingValue::Choice(if t == "MAC" { 1 } else { 0 }),
                );
            }
        }
        Ok(DeviceState {
            profile: self.read_profile().unwrap_or_default(),
            settings,
            mouse: None,
            keyboard: Some(KeyboardState {
                keymap,
                perf,
                advanced,
                macros,
            }),
            lighting,
            peq: None,
        })
    }
}

/// Official `customKeys.win.fn{L}.bindKeyValue` → wire code.
/// Numbers as is; for a `"@{...keycode=N}"` template, the keycode is picked.
fn official_bind(ko: &serde_json::Value, layer: usize) -> Option<u16> {
    let b = ko
        .get("customKeys")?
        .get("win")?
        .get(format!("fn{layer}"))?
        .get("bindKeyValue")?;
    match b {
        serde_json::Value::Number(n) => n.as_u64().map(|v| v as u16),
        serde_json::Value::String(s) => s
            .split("keycode=")
            .nth(1)
            .and_then(|t| t.trim_end_matches('}').trim().parse().ok()),
        _ => None,
    }
}

/// Official `advancedKeys` object → Advanced (advancedType 0/missing is None).
fn official_advanced(ak: &serde_json::Value, layout: &[LayoutKey]) -> Option<Advanced> {
    let ty = ak.get("advancedType").and_then(|x| x.as_u64())?;
    let num = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let codes = |v: &serde_json::Value, k: &str| -> Vec<u32> {
        v.get(k)
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_u64().map(|n| n as u32))
            .collect()
    };
    // (row2,col2) of the pair partner → layout index
    let pair = |v: &serde_json::Value| -> usize {
        let pos = (num(v, "row2") as u8, num(v, "col2") as u8);
        layout
            .iter()
            .position(|k| registry::su66max_matrix_pos(k.id) == pos)
            .unwrap_or(0)
    };
    match ty {
        1 => {
            let d = &ak["dks"];
            let rows = codes(d, "dks")
                .iter()
                .enumerate()
                .map(|(i, &code)| DksRow {
                    code,
                    phases: trp_to_phases(
                        d.get("trps")
                            .and_then(|x| x.as_array())
                            .and_then(|a| a.get(i))
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0) as u8,
                    ),
                })
                .collect();
            Some(Advanced::Dks {
                rows,
                bottom: num(d, "db2") as f32 / 1000.0,
            })
        }
        2 => {
            let d = &ak["mpt"];
            let cs = codes(d, "dks");
            let mut depths = [0f32; 3];
            for (i, v) in codes(d, "dbs").iter().take(3).enumerate() {
                depths[i] = *v as f32 / 1000.0;
            }
            let mut out = [0u32; 3];
            for (i, &c) in cs.iter().take(3).enumerate() {
                out[i] = c;
            }
            Some(Advanced::Mpt { depths, codes: out })
        }
        3 => {
            let d = &ak["mt"]["mt"];
            let cs = codes(d, "dksAll");
            Some(Advanced::Mt {
                tap: cs.first().copied().unwrap_or(0),
                hold: cs.get(1).copied().unwrap_or(0),
                hold_ms: num(d, "delay") as u16,
            })
        }
        4 => {
            let d = &ak["tgl"]["tgl"];
            Some(Advanced::Tgl {
                code: num(d, "kc") as u32,
                delay_ms: num(d, "delay") as u16,
            })
        }
        5 => {
            let d = &ak["end"]["end"];
            let cs = codes(d, "dks");
            Some(Advanced::End {
                code: cs.first().copied().unwrap_or(0),
                code_release: cs.get(1).copied().unwrap_or(0),
                delay_ms: num(d, "delay") as u16,
            })
        }
        6 => {
            // The real data is the socd object inside the wrapper (advancedKeys.socd).
            // socdMode is on the wrapper side (same as the official setHigherKeySOCD).
            let w = &ak["socd"];
            let d = &w["socd"];
            let cs = codes(d, "kcs");
            Some(Advanced::Socd {
                other: pair(d),
                code_a: cs.first().copied().unwrap_or(0),
                code_b: cs.get(1).copied().unwrap_or(0),
                delay_ms: num(d, "delay") as u16,
                mode: match num(w, "socdMode") {
                    1 => SocdMode::PriorityA,
                    2 => SocdMode::PriorityB,
                    3 => SocdMode::Neutral,
                    _ => SocdMode::Last,
                },
            })
        }
        7 => {
            let d = &ak["rs"]["rs"];
            let cs = codes(d, "kcs");
            Some(Advanced::Rs {
                other: pair(d),
                code_a: cs.first().copied().unwrap_or(0),
                code_b: cs.get(1).copied().unwrap_or(0),
                delay_ms: num(d, "delay") as u16,
            })
        }
        _ => None,
    }
}

/// Firmware string: picks the trailing ASCII chunk of the device_info response
/// (real device: "... 49142 .. 2026101216:41:03" — the tail is the build time).
fn parse_fw(r: &[u8; 64]) -> Option<String> {
    let tail: String = r[2..]
        .iter()
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                ' '
            }
        })
        .collect();
    tail.split_whitespace().last().map(|s| s.to_string())
}

/// layout key → cell numbers of custom LEDs (normally 1; the split space bar has multiple LEDs).
///
/// The LED grid is 6 rows×21 columns = 126 cells (fixed by visual inspection on the real device, also matches the SDK default).
/// Rows 1-4 use `cell = row*21 + floor(x)`. Row 5 has a compressed right side and mixed dead cells, so it is a
/// measured table: LCtrl=105, LWin=106, LAlt=107, below the space bar=109-113,
/// RAlt=114, Fn=115, ←=117, ↓=118, →=119 (108/116/120-125 are dead cells).
fn led_cells(k: &devhub_core::model::LayoutKey) -> Vec<usize> {
    let row = (k.id >> 4) as usize;
    if row == 5 {
        match k.id & 0x0f {
            0 => vec![105],      // LCtrl
            1 => vec![106],      // LWin
            2 => vec![107],      // LAlt
            4 => vec![109, 110], // Space left
            6 => vec![111],      // Space center
            8 => vec![112, 113], // Space right
            9 => vec![114],      // RAlt
            10 => vec![115],     // Fn
            12 => vec![117],     // ←
            13 => vec![118],     // ↓
            14 => vec![119],     // →
            _ => vec![],
        }
    } else {
        // LED column = matrix column (confirmed on the real device by probe10 split: '=74, Enter=76,
        // Del=98, ↑=97). The visual x drifts with wide keys, so it is not used.
        // The LED of a dead matrix pad really exists under the neighboring wide key, so it is written together:
        //   row3 col12 (=cell75) is under the right half of ', row4 col1 (=cell85) is under LShift.
        let col = (k.id & 0x0f) as usize;
        match (row, col) {
            (3, 11) => vec![74, 75], // ' (+ dead pad at the right end)
            (4, 0) => vec![84, 85],  // LShift (+ dead pad on the right)
            _ => vec![row * 21 + col],
        }
    }
}

/// DKS trps bitmask → the contract's 4 phases.
/// Wire bit order (UI 7 check columns → 8 bits):
///   b0=press-start Tap, b1=press-start Hold, b2=bottom-out Tap, b3|b4=bottom-out Hold,
///   b5=return-start Tap, b6=return-start Hold, b7=full-release Tap.
/// `[6,1]` response r[4]=mode → `Advanced` (follows the official higherKey*Data).
fn decode_higher(r: &[u8; 64]) -> Option<Advanced> {
    let mm = |i: usize| u16::from_le_bytes([r[i], r[i + 1]]) as f32 / 1000.0;
    let le = |i: usize| u16::from_le_bytes([r[i], r[i + 1]]) as u32;
    let col_to_index = |row2: u8, col2: u8| -> usize {
        registry::su66max_layout()
            .iter()
            .position(|k| registry::su66max_matrix_pos(k.id) == (row2, col2))
            .unwrap_or(0)
    };
    match r[4] {
        hk::DKS => Some(Advanced::Dks {
            rows: (0..4)
                .map(|i| DksRow {
                    code: le(5 + i * 2),
                    phases: trp_to_phases(r[13 + i]),
                })
                .collect(),
            // dbs = [db, db2] = [min travel, max travel]. The contract's bottom is the bottom-out side.
            bottom: mm(19),
        }),
        hk::MPT => Some(Advanced::Mpt {
            depths: [mm(11), mm(13), mm(15)],
            codes: [le(5), le(7), le(9)],
        }),
        hk::MT => Some(Advanced::Mt {
            tap: le(5),
            hold: le(7),
            hold_ms: u16::from_le_bytes([r[9], r[10]]),
        }),
        hk::TGL => Some(Advanced::Tgl {
            code: le(5),
            delay_ms: u16::from_le_bytes([r[7], r[8]]),
        }),
        hk::END => Some(Advanced::End {
            code: le(5),
            code_release: le(7),
            delay_ms: u16::from_le_bytes([r[9], r[10]]),
        }),
        hk::SOCD => Some(Advanced::Socd {
            other: col_to_index(r[5], r[6]),
            code_a: le(7),
            code_b: le(9),
            delay_ms: u16::from_le_bytes([r[11], r[12]]),
            mode: match r[13] {
                1 => SocdMode::PriorityA,
                2 => SocdMode::PriorityB,
                3 => SocdMode::Neutral,
                _ => SocdMode::Last,
            },
        }),
        hk::RS => Some(Advanced::Rs {
            other: col_to_index(r[5], r[6]),
            code_a: le(7),
            code_b: le(9),
            delay_ms: u16::from_le_bytes([r[11], r[12]]),
        }),
        _ => None,
    }
}

/// `[6,2,...]` write packet sequence (SOCD/RS take 2 packets, pair reversed).
fn higher_packets(row: u8, col: u8, binding: Option<&Advanced>) -> HubResult<Vec<Vec<u8>>> {
    let index_to_pos = |i: usize| -> HubResult<(u8, u8)> {
        registry::su66max_layout()
            .get(i)
            .map(|k| registry::su66max_matrix_pos(k.id))
            .ok_or_else(|| HubError::Invalid("advanced key index".into()))
    };
    let le = |v: u32| (v as u16).to_le_bytes();
    let mm = |v: f32| ((v * 1000.0).round().clamp(0.0, 65535.0) as u16).to_le_bytes();
    let packets: Vec<Vec<u8>> = match binding {
        None => vec![vec![row, col, hk::NONE]],
        Some(Advanced::Dks { rows, bottom }) => {
            let mut p = vec![row, col, hk::DKS];
            let mut rows4: Vec<DksRow> = rows.clone();
            rows4.resize(
                4,
                DksRow {
                    code: 0,
                    phases: [DksPhase::Off; 4],
                },
            );
            for dr in &rows4 {
                p.extend_from_slice(&le(dr.code));
            }
            for dr in &rows4 {
                p.push(phases_to_trp(&dr.phases));
            }
            // dbs = [db, db2]. db defaults to the half on the bottom-out side.
            p.extend_from_slice(&mm(bottom / 2.0));
            p.extend_from_slice(&mm(*bottom));
            vec![p]
        }
        Some(Advanced::Mpt { depths, codes }) => {
            let mut p = vec![row, col, hk::MPT];
            for c in codes {
                p.extend_from_slice(&le(*c));
            }
            for d in depths {
                p.extend_from_slice(&mm(*d));
            }
            vec![p]
        }
        Some(Advanced::Mt { tap, hold, hold_ms }) => {
            let mut p = vec![row, col, hk::MT];
            p.extend_from_slice(&le(*tap));
            p.extend_from_slice(&le(*hold));
            p.extend_from_slice(&hold_ms.to_le_bytes());
            vec![p]
        }
        Some(Advanced::Tgl { code, delay_ms }) => {
            let mut p = vec![row, col, hk::TGL];
            p.extend_from_slice(&le(*code));
            p.extend_from_slice(&delay_ms.to_le_bytes());
            vec![p]
        }
        Some(Advanced::End {
            code,
            code_release,
            delay_ms,
        }) => {
            let mut p = vec![row, col, hk::END];
            p.extend_from_slice(&le(*code));
            p.extend_from_slice(&le(*code_release));
            p.extend_from_slice(&delay_ms.to_le_bytes());
            vec![p]
        }
        Some(Advanced::Socd {
            other,
            code_a,
            code_b,
            mode,
            delay_ms,
        }) => {
            let (r2, c2) = index_to_pos(*other)?;
            let (x0, x1) = match mode {
                SocdMode::Last => (0, 0),
                SocdMode::PriorityA | SocdMode::First => (1, 2),
                SocdMode::PriorityB => (2, 1),
                SocdMode::Neutral => (3, 3),
            };
            let mut p1 = vec![row, col, hk::SOCD, r2, c2];
            p1.extend_from_slice(&le(*code_a));
            p1.extend_from_slice(&le(*code_b));
            p1.extend_from_slice(&delay_ms.to_le_bytes());
            p1.push(x0);
            let mut p2 = vec![r2, c2, hk::SOCD, row, col];
            p2.extend_from_slice(&le(*code_b));
            p2.extend_from_slice(&le(*code_a));
            p2.extend_from_slice(&delay_ms.to_le_bytes());
            p2.push(x1);
            vec![p1, p2]
        }
        Some(Advanced::Rs {
            other,
            code_a,
            code_b,
            delay_ms,
        }) => {
            let (r2, c2) = index_to_pos(*other)?;
            let mut p1 = vec![row, col, hk::RS, r2, c2];
            p1.extend_from_slice(&le(*code_a));
            p1.extend_from_slice(&le(*code_b));
            p1.extend_from_slice(&delay_ms.to_le_bytes());
            let mut p2 = vec![r2, c2, hk::RS, row, col];
            p2.extend_from_slice(&le(*code_b));
            p2.extend_from_slice(&le(*code_a));
            p2.extend_from_slice(&delay_ms.to_le_bytes());
            vec![p1, p2]
        }
    };
    Ok(packets)
}

fn trp_to_phases(trp: u8) -> [DksPhase; 4] {
    use DksPhase::*;
    let pick = |tap: u8, hold: u8| {
        if trp & hold != 0 {
            Hold
        } else if trp & tap != 0 {
            Tap
        } else {
            Off
        }
    };
    [
        pick(1, 2),
        pick(4, 8 | 16),
        pick(32, 64),
        if trp & 128 != 0 { Tap } else { Off },
    ]
}

fn phases_to_trp(p: &[DksPhase; 4]) -> u8 {
    let mut t = 0u8;
    match p[0] {
        DksPhase::Tap => t |= 1,
        DksPhase::Hold => t |= 2,
        DksPhase::Off => {}
    }
    match p[1] {
        DksPhase::Tap => t |= 4,
        DksPhase::Hold => t |= 8 | 16,
        DksPhase::Off => {}
    }
    match p[2] {
        DksPhase::Tap => t |= 32,
        DksPhase::Hold => t |= 64,
        DksPhase::Off => {}
    }
    if p[3] == DksPhase::Tap {
        t |= 128;
    }
    t
}

/// Default layout legends → HID usage (for reset).
fn default_code(label: &str) -> u16 {
    match label {
        "Esc" => 0x29,
        "Tab" => 0x2B,
        "Caps" => 0x39,
        "Backspace" => 0x2A,
        "Enter" => 0x28,
        "LShift" => 0xE1,
        "RShift" => 0xE5,
        "LCtrl" => 0xE0,
        "LWin" => 0xE3,
        "LAlt" => 0xE2,
        "RAlt" => 0xE6,
        "Space" => 0x2C,
        "Fn" => 0xF101,
        "Del" => 0x4C,
        "↑" => 0x52,
        "↓" => 0x51,
        "←" => 0x50,
        "→" => 0x4F,
        "-" => 0x2D,
        "=" => 0x2E,
        "[" => 0x2F,
        "]" => 0x30,
        "\\" => 0x31,
        ";" => 0x33,
        "'" => 0x34,
        "," => 0x36,
        "." => 0x37,
        "/" => 0x38,
        "`" => 0x35,
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => {
            0x04 + (s.as_bytes()[0] - b'A') as u16
        }
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_digit() => {
            let d = s.as_bytes()[0] - b'0';
            if d == 0 {
                0x27
            } else {
                0x1E + d as u16 - 1
            }
        }
        _ => 0,
    }
}

fn default_stream(layer: u8) -> Vec<u16> {
    let mut stream = vec![0u16; 8 * 30];
    for k in registry::su66max_layout() {
        let (row, col) = registry::su66max_matrix_pos(k.id);
        stream[row as usize * 30 + col as usize] = if layer == 0 {
            default_code(&k.label)
        } else {
            1 // other layers are transparent
        };
    }
    stream
}

fn build_caps(xs: &XsydInfo) -> Capabilities {
    use section::*;
    let mut settings = vec![
        SettingSpec {
            id: "polling".into(),
            section: PERFORMANCE.into(),
            label: "Polling rate".into(),
            help: None,
            kind: SettingKind::Choice {
                options: xs
                    .polling
                    .iter()
                    .map(|(c, l)| ChoiceOpt::new(*c as i64, *l))
                    .collect(),
            },
            visible_when: None,
            confirm: None,
            enabled_when: None,
        },
        SettingSpec {
            id: "system_type".into(),
            section: KEYBOARD.into(),
            label: "OS type".into(),
            help: Some("Switches between the Win and Mac key layout modes".into()),
            kind: SettingKind::Choice {
                options: vec![ChoiceOpt::new(0, "Windows"), ChoiceOpt::new(1, "Mac")],
            },
            visible_when: None,
            confirm: None,
            enabled_when: None,
        },
        SettingSpec::toggle("shake_opt", KEYBOARD, "Debounce"),
        SettingSpec::slider("light_sleep", POWER, "Light sleep", 0.0, 120.0, 1.0, "min"),
    ];
    settings[3].help = Some("Time of inactivity before the lights turn off (in minutes)".into());
    // mode byte 0-19 = L1-L20 of the official UI (confirmed on the real device that writes are clamped to 19).
    // Mapping identified by imaging the real device with an OBS camera (see docs/PROTOCOL_NOTES.md):
    //  SDK enum Y1 is 1..16 (Ripple..Jump), but the real device's wire 1..15 corresponds to Y1[2..16],
    //  and wire0=static palette display, wire16=custom display, wire17-19=key press reaction.
    //  Every mode uses the selected palette color; sel=0 is the firmware's rainbow (multicolor) display.
    let effects = vec![
        LightEffect::new(0, "Solid color").color().still(), // Fixes the selected palette color (sel0=rainbow)
        LightEffect::new(1, "Rainbow flow").color().dir(), // Rainbow gradient scrolls left and right
        LightEffect::new(2, "Slow color change").color(),  // Uniform color + gentle hue rotation
        LightEffect::new(3, "Fast color change").color(),  // Hue rotates quickly
        LightEffect::new(4, "Rainbow checker").color(),    // Rainbow checkerboard
        LightEffect::new(5, "Rainbow cycle").color(),      // Hue-cycling gradient
        LightEffect::new(6, "Rainbow pulse").color(),      // Hue cycling + pulsing
        LightEffect::new(7, "Soft window").color().dir(), // Nearly uniform + a faint light band moves
        LightEffect::new(8, "Gentle morph").color(),      // Color patches morph like stains
        LightEffect::new(9, "Color band shift").color().dir(), // Wide color zones move
        LightEffect::new(10, "Rainbow wave").color().dir(), // Multi-band rainbow wave
        LightEffect::new(11, "Diagonal light band").color().dir(), // A diagonal light band sweeps across
        LightEffect::new(12, "Confetti").color(),                  // Random color dots scattered
        LightEffect::new(13, "Rainbow mosaic").color(),            // Fine rainbow checker
        LightEffect::new(14, "Light rain").color().dir(),          // Color dots fall
        LightEffect::new(15, "Bouncing light").color(),            // Lit blocks jump around
        LightEffect::new(16, "Custom").per_key(),                  // Only isCustom cells are shown
        LightEffect::new(17, "Press: around").color(), // Lights the block around the pressed key
        LightEffect::new(18, "Press: row").color(),    // Lights the row of the pressed key
        LightEffect::new(19, "Press: wide").color(),   // Lights a wide area around the pressed key
    ];
    Capabilities {
        settings,
        actions: vec![
            ActionSpec::new("blackout", LIGHTING, "Toggle blackout")
                .help("Turns off all LEDs temporarily. Press again to restore them."),
            ActionSpec::new("calibrate", PERFORMANCE, "Start magnetic switch calibration")
                .help("Calibrates the key press depth sensors. After starting, press every key hard a few times."),
            ActionSpec::new("calibrate_finish", PERFORMANCE, "Finish calibration")
                .help("Saves the press data to the device and finishes calibration."),
            ActionSpec::new("reset_perf", SYSTEM, "Reset performance settings")
                .danger("Resets the trigger, RT and dead zones of all keys to their defaults."),
            ActionSpec::new("reset_keymap", SYSTEM, "Reset key mapping")
                .danger("Resets the key assignments of all layers to their defaults."),
            ActionSpec::new("reset_lighting", SYSTEM, "Reset lighting")
                .danger("Resets lighting to its defaults."),
            ActionSpec::new("reset_advanced", SYSTEM, "Reset advanced keys")
                .danger("Clears all DKS/MPT/MT/TGL/END/SOCD/RS."),
            ActionSpec::new("export_config", SYSTEM, "Export settings")
                .help("Writes all current settings to a JSON file.")
                .save_file(&["json"]),
            ActionSpec::new("import_config", SYSTEM, "Import settings")
                .help("Writes all settings back from an exported JSON file.")
                .file(&["json"]),
            ActionSpec::new("reset_macro", SYSTEM, "Reset macros")
                .danger("Deletes all macros."),
            ActionSpec::new("factory_reset", SYSTEM, "Restore factory settings")
                .danger("Resets all settings (keys, performance, lighting, macros)."),
        ],
        profiles: (xs.configs > 1).then_some(ProfileCaps {
            count: xs.configs,
            renamable: true,
        }),
        mouse: None,
        keyboard: Some(KeyboardCaps {
            layout: registry::su66max_layout(),
            layers: (0..xs.layers)
                .map(|i| {
                    if i == 0 {
                        "Main".into()
                    } else {
                        format!("Fn{i}")
                    }
                })
                .collect(),
            catalog: xsyd_catalog(),
            // 0x1000|mods<<8|usage (confirmed on the custom-key screen).
            combos: true,
            perf: Some(PerfCaps {
                travel: xs.travel,
                min: xs.rt_step,
                step: xs.rt_step,
                rapid_trigger: true,
                separate_release: true,
                rt_first: true,
                deadzones: true,
                continuous_rt: true,
                switch_types: xs.axes.iter().map(|a| format!("Switch {a}")).collect(),
                switch_ids: xs.axes.clone(),
                per_key: true,
                live_travel: true,
            }),
            advanced: vec![
                AdvancedKind::Dks,
                AdvancedKind::Mpt,
                AdvancedKind::Mt,
                AdvancedKind::Tgl,
                AdvancedKind::End,
                AdvancedKind::Socd,
                AdvancedKind::Rs,
            ],
            macros: xs.macro_space.map(|(slots, max_ev)| MacroCaps {
                slots,
                max_events: max_ev,
                max_delay_ms: 32767,
                modes: vec![
                    ChoiceOpt::new(0, "Run on click (set count)"),
                    ChoiceOpt::new(1, "Run on click"),
                    ChoiceOpt::new(4, "Repeat while held (stops immediately on release)"),
                    ChoiceOpt::new(5, "Repeat while held (stops after completing)"),
                ],
                repeat_modes: vec![0],
                names: false,
                mouse_events: false,
            }),
        }),
        lighting: xs
            .light_areas
            .iter()
            .enumerate()
            .map(|(i, &a)| LightArea {
                id: a,
                name: if a == 0 {
                    "Main keyboard".into()
                } else {
                    format!("Decoration {i}")
                },
                effects: effects.clone(),
                brightness_max: 100,
                speed_max: 100,
                directions: vec!["Forward".into(), "Backward".into()],
                switchable: true,
                // Only Keyboard (area 0) on DoubleLighting devices has upper/lower zone open/close.
                zones: if a == 0 && xs.double_lighting {
                    vec!["Upper light".into(), "Lower light".into()]
                } else {
                    vec![]
                },
                per_key: a == 0,
                // DirectDrive / live readback is only for the keyboard area (per-LED cells).
                direct_drive: a == 0,
                live_readback: a == 0,
            })
            .collect(),
        battery: false,
        peq: None,
        firmware_feed: None, // refilled in caps() from the info's vid/pid
    }
}

/// xsyd key code space (PROTOCOL_NOTES §2.4). Basic keys are HID usage.
fn xsyd_catalog() -> Vec<KeyGroup> {
    let g = |name: &str, v: &[(u32, &str, &str)]| KeyGroup {
        name: name.into(),
        keys: v
            .iter()
            .map(|(c, l, s)| KeyChoice {
                code: *c,
                label: (*l).into(),
                short: Some((*s).into()),
            })
            .collect(),
    };
    vec![
        devhub_core::keys::basic_group(),
        g(
            "Mouse",
            &[
                (16640, "Mouse left", "LMB"),
                (16896, "Mouse right", "RMB"),
                (17152, "Mouse middle", "MMB"),
                (17408, "Mouse forward", "Fwd"),
                (17664, "Mouse back", "Back"),
                (18945, "Wheel up", "W↑"),
                (19201, "Wheel down", "W↓"),
            ],
        ),
        g(
            "Media",
            &[
                (0x20CD, "Play/Pause", "⏯"),
                (8373, "Next track", "⏭"),
                (8374, "Previous track", "⏮"),
                (0x20E9, "Volume +", "🔊"),
                (0x20EA, "Volume −", "🔉"),
                (8418, "Mute", "🔇"),
            ],
        ),
        g(
            "Layers",
            &[
                (61696, "Go to main layer", "L0"),
                (61697, "Go to Fn1 layer", "Fn1"),
                (61698, "Go to Fn2 layer", "Fn2"),
                (61699, "Go to Fn3 layer", "Fn3"),
            ],
        ),
        g(
            "System",
            &[
                (61957, "Switch to profile 1", "P1"),
                (61958, "Switch to profile 2", "P2"),
                (61961, "Win key lock", "WinL"),
                (61955, "Magnetic switch calibration (long press)", "Cal"),
                (61952, "Reset (long press)", "Rst"),
            ],
        ),
        g(
            "Lighting",
            &[
                (62208, "Switch lighting mode", "LMod"),
                (62209, "Switch lighting color", "LCol"),
                (62210, "Brightness +", "L+"),
                (62211, "Brightness −", "L−"),
                (62215, "Light ON/OFF", "L⏻"),
            ],
        ),
    ]
}

/// wire u16 → KeyAction. 0x0000=unassigned, 0x0001=transparent,
/// 0x1000|mods<<8|usage = combination key (confirmed with the SDK custom-key).
fn code_to_action(code: u16) -> KeyAction {
    match code {
        0x0000 => KeyAction::None,
        0x0001 => KeyAction::Transparent,
        c if (0x1001..=0x1FFF).contains(&c) => KeyAction::Combo {
            mods: ((c >> 8) & 0x0F) as u8,
            code: (c & 0xFF) as u32,
        },
        // Macro call: 0xF500+slot (SDK keycode catalog macro:62720-62735).
        c if (0xF500..=0xF50F).contains(&c) => KeyAction::Macro((c - 0xF500) as u8),
        c => KeyAction::Code(c as u32),
    }
}
fn action_to_code(a: &KeyAction) -> HubResult<u16> {
    Ok(match a {
        KeyAction::None => 0,
        KeyAction::Transparent => 1,
        KeyAction::Code(c) => (*c)
            .try_into()
            .map_err(|_| HubError::Invalid("keycode".into()))?,
        KeyAction::Combo { mods, code } => {
            // wire: 0x1000 | mods<<8 | usage (left modifiers bit0-3 only)
            if *code > 0xFF {
                return Err(HubError::Invalid("combo keycode".into()));
            }
            0x1000 | ((*mods as u16 & 0x0F) << 8) | (*code as u16 & 0xFF)
        }
        // 0xF500+slot (up to 16 slots. The real device's slot count is limited on the macro_space side).
        KeyAction::Macro(i) if *i < 16 => 0xF500 + *i as u16,
        KeyAction::Macro(_) => return Err(HubError::Invalid("macro slot".into())),
    })
}

impl Drop for XsydDriver {
    fn drop(&mut self) {
        self.release_dd_pins();
    }
}

impl DeviceDriver for XsydDriver {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn caps(&self) -> Capabilities {
        let mut c = self.caps.clone();
        c.firmware_feed = Some(FirmwareFeed::Sparklink {
            vid: self.info.vid,
            pid: self.info.pid,
            board_id: self.info.board_id,
        });
        c
    }

    fn read(&self) -> HubResult<DeviceState> {
        let layout = registry::su66max_layout();
        let mut keymap = Vec::with_capacity(self.xs.layers as usize);
        for layer in 0..self.xs.layers {
            let stream = self.read_keymap(layer)?;
            keymap.push(
                layout
                    .iter()
                    .map(|k| {
                        let (row, col) = registry::su66max_matrix_pos(k.id);
                        code_to_action(stream[row as usize * 30 + col as usize])
                    })
                    .collect::<Vec<_>>(),
            );
        }
        self.alive()?;
        // Per-key performance — a failure is a read error rather than defaults (treating a partial read
        // as a complete setting would make the diff apply erase the real device's values).
        let mut perf = Vec::with_capacity(layout.len());
        for k in &layout {
            self.alive()?;
            let (row, col) = registry::su66max_matrix_pos(k.id);
            perf.push(self.read_perf(row, col).map(|(p, _)| p)?);
        }
        // Advanced keys (only mode!=NONE) — returning a partial list on a mid-way failure would be read as "nonexistent
        // advanced key = delete", so failures propagate.
        let mut advanced = Vec::new();
        for i in 0..layout.len() {
            self.alive()?;
            if let Some(a) = self.read_higher(i)? {
                advanced.push(AdvancedKey {
                    key: i,
                    layer: 0,
                    binding: a,
                });
            }
        }
        // Macros (only valid slots have their contents read) — failures reading the body propagate too.
        let mut macros = vec![];
        if let Some((slots, _)) = self.xs.macro_space {
            for id in 0..slots {
                let def = match self.read_macro_mode(id) {
                    Ok((true, act, rep, mode)) => Some(MacroDef {
                        name: format!("Macro {}", id + 1),
                        events: self.read_macro_events(id, act)?,
                        mode,
                        repeat: rep,
                    }),
                    Ok(_) => None,
                    Err(e) => return Err(e),
                };
                macros.push(def);
            }
        }
        self.alive()?;
        let mut lighting = BTreeMap::new();
        for &a in &self.xs.light_areas {
            self.alive()?;
            lighting.insert(a, self.read_lighting(a)?);
        }
        let mut settings = BTreeMap::new();
        for spec in &self.caps.settings {
            self.alive()?;
            if let Some(v) = self.read_setting(&spec.id) {
                settings.insert(spec.id.clone(), v);
            }
        }
        let profile = self.read_profile()?;
        Ok(DeviceState {
            profile,
            settings,
            mouse: None,
            keyboard: Some(KeyboardState {
                keymap,
                perf,
                advanced,
                macros,
            }),
            lighting,
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        match change {
            Change::KeyMap { layer, key, action } => {
                let code = action_to_code(action)?;
                let layout_index = self
                    .caps
                    .keyboard
                    .as_ref()
                    .and_then(|k| k.layout.get(*key))
                    .map(|k| k.id)
                    .ok_or_else(|| HubError::Invalid("key index".into()))?;
                self.write_key(*layer, layout_index, code)?;
                self.save(scope::LAYOUT)
            }
            Change::KeyMapReset { layer } => {
                let stream = self
                    .factory_stream(*layer)
                    .unwrap_or_else(|| default_stream(*layer));
                self.write_stream(*layer, &stream)?;
                self.save(scope::LAYOUT)
            }
            Change::Lighting { area, state } if self.xs.light_areas.contains(area) => {
                // per_key is a custom overlay independent of the base mode (the official isCustom
                // semantics). In area 0 it is always written as an absolute state — only
                // changed chunks are sent, so with no change only the readback cost is paid.
                if *area == 0 {
                    self.write_per_key(&state.per_key)?;
                    // write_per_key also wipes DD pins (isCustom) —
                    // discard the tracking so that the next direct_drive re-pins all cells.
                    // dd_pre is discarded too: the "pre-DD custom" has been
                    // superseded by the per_key just applied, so it is re-taken in the next session.
                    if let Ok(mut p) = self.dd_pins.lock() {
                        p.remove(area);
                    }
                    if let Ok(mut m) = self.dd_pre.lock() {
                        m.remove(area);
                    }
                }
                self.write_lighting(*area, state)?;
                self.save(scope::LIGHTING)?;
                // save() waits for the change to take effect before returning, but to cope with real devices that still return old
                // values, verify the base value and rewrite once only if it is off.
                if let Ok(r) = self.cmd(grp::LIGHTING, sub::READ, &[*area, 0]) {
                    let exp = [
                        self.compose_open(*area, state),
                        state.effect.min(19),
                        state.brightness.min(100),
                        state.speed.min(100),
                        state.direction.min(1),
                        state.palette_sel.min(7),
                    ];
                    if r[4..10] != exp {
                        self.write_lighting(*area, state)?;
                        self.save(scope::LIGHTING)?;
                    }
                }
                Ok(())
            }
            Change::Lighting { .. } => Err(HubError::Unsupported("lighting area")),
            Change::KeyPerf { keys, perf } => {
                let layout = registry::su66max_layout();
                for &i in keys {
                    let k = layout
                        .get(i)
                        .ok_or_else(|| HubError::Invalid("key index".into()))?;
                    let (row, col) = registry::su66max_matrix_pos(k.id);
                    self.write_perf(row, col, perf)?;
                }
                self.save(scope::PERFORMANCE)
            }
            Change::Advanced { key, binding, .. } => {
                self.write_higher(*key, binding.as_ref())?;
                self.save(scope::HIGHER_KEY)
            }
            Change::Macro { slot, def } => {
                self.write_macro(*slot as u8, def.as_ref())?;
                self.save(scope::MACRO)
            }
            Change::Setting { id, value } => self.write_setting(id, value),
            Change::Profile(i) => {
                if *i >= self.xs.configs {
                    return Err(HubError::Invalid("profile".into()));
                }
                // The saved colors belong to the pre-switch config. Do not overwrite the switch target with those colors.
                self.direct_drive_stop(0)?;
                self.cmd(grp::GLOBAL, gfs::CONFIG_SWITCH, &[2, *i])?;
                Ok(())
            }
            Change::ProfileName { index, name } => {
                if *index >= self.xs.configs {
                    return Err(HubError::Invalid("profile".into()));
                }
                let mut args = vec![4, *index];
                let bytes = name.as_bytes();
                let n = bytes.len().min(32);
                args.extend_from_slice(&bytes[..n]);
                self.cmd(grp::GLOBAL, gfs::CONFIG_SWITCH, &args)?;
                Ok(())
            }
            _ => Err(HubError::Unsupported("change")),
        }
    }

    fn run_action(&self, id: &str, file: Option<&std::path::Path>) -> HubResult<ActionOutcome> {
        match id {
            // Blackout (official CustomCommand): state [10,1,0] → r[3]
            // (255=lights off/0=normal), toggle is [10,1,1,{255|0}].
            "blackout" => {
                let r = self.cmd(grp::CUSTOM, sub::READ, &[0])?;
                let active = r[3] == 255;
                self.cmd(grp::CUSTOM, sub::READ, &[1, if active { 0 } else { 255 }])?;
                Ok(ActionOutcome {
                    message: Some(
                        if active {
                            "Blackout disabled"
                        } else {
                            "Blackout enabled"
                        }
                        .into(),
                    ),
                    reload: false,
                })
            }
            // Calibration start [2,6,0]. After pressing keys a few times,
            // calling the same action again is expected to end with [2,6,1] — to match the UI flow,
            // only the start is done here (the end is calibrate_finish).
            "calibrate" => {
                self.cmd(grp::GLOBAL, gfs::CALIBRATION, &[0])?;
                Ok(ActionOutcome {
                    message: Some(
                        "Calibration started. Press every key hard a few times, then run \
                         \"Finish calibration\""
                            .into(),
                    ),
                    reload: false,
                })
            }
            "calibrate_finish" => {
                self.cmd(grp::GLOBAL, gfs::CALIBRATION, &[1])?;
                self.save(scope::CALIBRATION)?;
                Ok(ActionOutcome {
                    message: Some("Calibration finished".into()),
                    reload: true,
                })
            }
            "reset_keymap" => {
                for layer in 0..self.xs.layers {
                    let stream = self
                        .factory_stream(layer)
                        .unwrap_or_else(|| default_stream(layer));
                    self.write_stream(layer, &stream)?;
                }
                self.save(scope::LAYOUT)?;
                Ok(ActionOutcome {
                    message: Some("Key assignments reset to defaults".into()),
                    reload: true,
                })
            }
            "reset_lighting" => {
                self.write_lighting(
                    0,
                    &LightingState {
                        on: true,
                        effect: 0,
                        color: Rgb::new(255, 255, 255),
                        brightness: 50,
                        speed: 50,
                        direction: 0,
                        per_key: BTreeMap::new(),
                        zone_on: vec![true, true],
                        palette: vec![Rgb::new(255, 255, 255)],
                        palette_sel: 0,
                        color_correction: None,
                    },
                )?;
                self.save(scope::LIGHTING)?;
                Ok(ActionOutcome {
                    message: Some("Lighting reset to defaults".into()),
                    reload: true,
                })
            }
            "factory_reset" => {
                self.cmd(grp::GLOBAL, gfs::RESET_FACTORY, &[scope::ALL])?;
                Ok(ActionOutcome {
                    message: Some("Factory settings restored".into()),
                    reload: true,
                })
            }
            // Settings export/import (own JSON — not compatible with the official encrypted format).
            // Accepts {app, model, name, state: DeviceState} or a bare DeviceState.
            "export_config" => {
                let path = file.ok_or_else(|| HubError::Invalid("save destination".into()))?;
                let state = self.read()?;
                let doc = serde_json::json!({
                    "app": "device-hub",
                    "model": self.info.model,
                    "name": self.info.name,
                    "state": state,
                });
                let json = serde_json::to_string_pretty(&doc)
                    .map_err(|e| HubError::Invalid(format!("JSON: {e}")))?;
                std::fs::write(path, json)
                    .map_err(|e| HubError::Io(format!("writing file: {e}")))?;
                Ok(ActionOutcome {
                    message: Some(format!("Settings exported to {}", path.display())),
                    reload: false,
                })
            }
            "import_config" => {
                let path = file.ok_or_else(|| HubError::Invalid("file".into()))?;
                let text = std::fs::read_to_string(path)
                    .map_err(|e| HubError::Io(format!("reading file: {e}")))?;
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| HubError::Invalid(format!("JSON: {e}")))?;
                let st: DeviceState = if v.get("keyboards").is_some() || v.get("data").is_some() {
                    // Export of the official configurator (keyboard_config.json).
                    let plain = self.official_plain(&v)?;
                    self.official_to_state(&plain)?
                } else {
                    serde_json::from_value(v.get("state").cloned().unwrap_or(v))
                        .map_err(|e| HubError::Invalid(format!("settings data: {e}")))?
                };
                self.import_state(&st)?;
                Ok(ActionOutcome {
                    message: Some("Settings imported".into()),
                    reload: true,
                })
            }
            "reset_perf" => self.reset_scope(scope::PERFORMANCE, "Performance settings"),
            "reset_advanced" => self.reset_scope(scope::HIGHER_KEY, "Advanced keys"),
            "reset_macro" => self.reset_scope(scope::MACRO, "Macros"),
            _ => Err(HubError::Unsupported("action")),
        }
    }

    fn live_travel(&self) -> HubResult<Vec<f32>> {
        self.route_mm()
    }

    /// `[4,3,kind,row]` × ADC/Route/Calibrate/KeyStatus fetched over rows 1-5 and gathered in layout order.
    /// First drain the queue and cache the frames that are flowing, then send requests
    /// for the missing cells all at once to collect them in bulk (a round trip per cell would exceed the 25ms slot
    /// and make updates sporadic). If another process such as the official HUB is conversing, piggyback on its responses
    /// and hold off sending (passive mode).
    fn live_axis(&self) -> HubResult<Vec<AxisSample>> {
        const KINDS: [u8; 4] = [axis::ADC, axis::ROUTE, axis::CALIBRATE, axis::KEY_STATUS];
        // The 2 kinds that affect visual smoothness are fetched ~every poll; the 2 with little change are thinned out.
        let fresh_for = |kind: u8| match kind {
            k if k == axis::ROUTE || k == axis::KEY_STATUS => Duration::from_millis(60),
            _ => Duration::from_millis(2500),
        };
        let cell_age = |kind: u8, row: u8| -> Option<(Instant, [u16; 30])> {
            self.axis_cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&(kind, row))
                .copied()
        };
        let stale = |kind: u8, row: u8| -> bool {
            match cell_age(kind, row) {
                Some((t, _)) => t.elapsed() >= fresh_for(kind),
                None => true,
            }
        };
        let is_passive =
            || *self.axis_passive.lock().unwrap_or_else(|p| p.into_inner()) > Instant::now();

        self.drain_axis(4);

        // Find the missing cells and request in bulk → collect the echoes together.
        let mut wanted = Vec::with_capacity(20);
        for row in 1..=5u8 {
            for kind in KINDS {
                if stale(kind, row) {
                    wanted.push((kind, row));
                }
            }
        }
        if !wanted.is_empty() && !is_passive() {
            for &(kind, row) in &wanted {
                self.ch
                    .send(&[grp::PERFORMANCE, sub::AXIS_DATA, kind, row])?;
                self.axis_self
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert((kind, row), Instant::now());
            }
            // Wait briefly and leave the rest to the next poll's drain — blocking for long here
            // would far exceed the OBS loop period (25ms) and drop the display FPS.
            let deadline = Instant::now() + Duration::from_millis(14);
            let mut saw_axis = false;
            let mut foreign = false;
            while Instant::now() < deadline {
                match self.ch.recv(5) {
                    Ok(b) => {
                        if self.note_axis_frame(&b) {
                            saw_axis = true;
                        } else if b.first().copied() == Some(grp::PERFORMANCE)
                            && b.get(1).copied() == Some(0xFF)
                        {
                            // Unsupported (kind,row) — advance filling with 0.
                            self.axis_cache
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .insert((b[2], b[3]), (Instant::now(), [0; 30]));
                        } else if self.is_foreign_cmd(&b) {
                            self.note_foreign();
                            foreign = true;
                            break;
                        }
                        if wanted.iter().all(|&(k, r)| !stale(k, r)) {
                            break;
                        }
                    }
                    Err(HubError::Protocol(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            // No axis frame came and no external response either → treat as disconnected
            // (propagate as a failure as before, to keep disconnect detection).
            if !saw_axis && !foreign {
                return Err(HubError::Protocol("axis: no frames".into()));
            }
        }

        let layout = registry::su66max_layout();
        let mut out = vec![AxisSample::default(); layout.len()];
        let cell = |kind: u8, row: u8| -> [u16; 30] {
            cell_age(kind, row).map(|(_, v)| v).unwrap_or([0; 30])
        };
        for (i, k) in layout.iter().enumerate() {
            let (kr, kc) = registry::su66max_matrix_pos(k.id);
            if (1..=5).contains(&kr) && (kc as usize) < 30 {
                let c = kc as usize;
                out[i] = AxisSample {
                    route: cell(axis::ROUTE, kr)[c] as f32 / 1000.0,
                    adc: cell(axis::ADC, kr)[c],
                    status: cell(axis::KEY_STATUS, kr)[c] as u8,
                    calibrated: cell(axis::CALIBRATE, kr)[c] as u8,
                };
            }
        }
        Ok(out)
    }

    /// Per-key actuation point (layout order, mm). RT keys use First touch, normal keys use actuation.
    /// Called only once at device discovery (66 keys' read_perf = 66 round-trip).
    fn live_key_perf(&self) -> HubResult<Vec<KeyPerf>> {
        let layout = registry::su66max_layout();
        let mut out = Vec::with_capacity(layout.len());
        for k in &layout {
            self.alive()?;
            let (row, col) = registry::su66max_matrix_pos(k.id);
            let (p, _) = self.read_perf(row, col)?;
            out.push(p);
        }
        Ok(out)
    }

    fn live_actuation(&self) -> HubResult<Vec<f32>> {
        Ok(self
            .live_key_perf()?
            .into_iter()
            .map(|p| match p.mode {
                TriggerMode::Rapid => p.rt_first,
                TriggerMode::Normal => p.actuation,
            })
            .collect())
    }

    fn battery(&self) -> HubResult<Battery> {
        // [13,1] → [2]=mode [3]=charge [4]=percent. SU66 is wired → should come back as unsupported.
        let r = self.cmd(grp::THREE_MODE, sub::GET_BASIC_INFO, &[])?;
        Ok(Battery {
            percent: Some(r[4]),
            charging: r[3] == 1,
        })
    }

    /// DirectDrive `[5,5,area,chunk,…15 cells×4B(B,G,R,255)]` — no response (SDK
    /// `setLightingDirectDrive` = sendDataNoResponse). Key index → LED cell expansion is
    /// `led_cells` (space bar etc. have multiple cells).
    /// **Cells with flag=255 are pinned as isCustom — flag=0 means "don't touch" and
    /// is not release (confirmed on the real device)**: unspecified cells keep being driven by
    /// the firmware animation as is (P2-b hybrid — confirmed on the real device that
    /// red DD cells work overlaid on a purple effect).
    /// Therefore:
    /// - Chunks containing only flag=0 are not sent (fewer wasted sends — in the hybrid, highlighting
    ///   5 keys takes only 1-2 chunks).
    /// - Cells pinned in the previous frame but not specified this time are released by clearing isCustom with `[5,4]`
    ///   (the driver tracks the pin set — the set specified on each `direct_drive` call
    ///   can be treated as an absolute specification of "the set I want lit now").
    fn direct_drive(&self, area: u8, cells: &BTreeMap<usize, Rgb>) -> HubResult<()> {
        self.alive()?;
        let layout = registry::su66max_layout();
        let mut buf = [[0u8; 4]; 135];
        let mut new_map = BTreeMap::new();
        for (&idx, rgb) in cells {
            if let Some(k) = layout.get(idx) {
                for cell in led_cells(k) {
                    if cell < 135 {
                        buf[cell] = [rgb.b, rgb.g, rgb.r, 255];
                        new_map.insert(cell, [rgb.b, rgb.g, rgb.r]);
                    }
                }
            }
        }
        let old_map = {
            let pins = self.dd_pins.lock().unwrap_or_else(|p| p.into_inner());
            pins.get(&area).cloned().unwrap_or_default()
        };
        // Session start: before the first pin is set, save aside the user custom cells + colors
        // — so that even after DD overwrites them, they can be restored to the original custom on release.
        if !new_map.is_empty() {
            let needs = self
                .dd_pre
                .lock()
                .map(|p| !p.contains_key(&area))
                .unwrap_or(true);
            if needs {
                self.capture_dd_pre(area)?;
            }
        }
        let released: Vec<usize> = old_map
            .keys()
            .filter(|c| !new_map.contains_key(*c))
            .copied()
            .collect();
        // Send only chunks that contain a newly pinned cell or one whose color changed.
        // Unchanged pins remain, so no resend is needed (real device: they stay stuck even after the stream stops).
        let touched: std::collections::BTreeSet<u8> = new_map
            .iter()
            .filter(|(cell, rgb)| old_map.get(*cell).copied().flatten() != Some(**rgb))
            .map(|(cell, _)| (cell / 15) as u8)
            .collect();
        for c in touched {
            let mut w = [0u8; 64];
            w[0] = grp::LIGHTING;
            w[1] = sub::LIGHT_DIRECTDRIVE;
            w[2] = area;
            w[3] = c;
            for (i, cell) in buf[c as usize * 15..c as usize * 15 + 15]
                .iter()
                .enumerate()
            {
                w[4 + i * 4..8 + i * 4].copy_from_slice(cell);
            }
            let result = self.ch.send(&w);
            // Even if a send fails midway, chunks we attempted to send are restored at stop.
            // Even on error it may have reached the device, so track conservatively.
            self.dd_pins
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entry(area)
                .or_default()
                .extend(
                    new_map
                        .range(c as usize * 15..(c as usize + 1) * 15)
                        .map(|(&cell, &rgb)| (cell, result.is_ok().then_some(rgb))),
                );
            result?;
            // The official app is naturally paced by the await USB transfer time (~1ms). Even hammering at 133µs
            // the device survives (probe9), but leave 1ms to match the OUT endpoint's consumption.
            std::thread::sleep(Duration::from_millis(1));
        }
        // Release the cells that dropped out with `[5,4]` — read back per chunk and fix only the target cells
        // (the user's custom and other pins are preserved. Cells that were custom before DD
        // are restored with dd_pre's color + flag=255).
        let rel_failed = self.release_cells(area, &released.iter().copied().collect());
        // Cells of chunks whose release failed stay tracked, and the release is retried on the next frame.
        let mut pending: BTreeMap<_, _> = new_map
            .into_iter()
            .map(|(cell, rgb)| (cell, Some(rgb)))
            .collect();
        for cell in released {
            if rel_failed.contains(&((cell / 15) as u8)) {
                if let Some(rgb) = old_map.get(&cell) {
                    pending.insert(cell, *rgb);
                }
            }
        }
        let mut pins = self.dd_pins.lock().unwrap_or_else(|p| p.into_inner());
        pins.insert(area, pending);
        Ok(())
    }

    /// DirectDrive release — flag=255 in a DD frame pins the cell as isCustom
    /// (flag=0 means "don't touch" and is not release — probe14).
    /// Releases only the tracked DD pins with `[5,4]` (the user's custom is preserved,
    /// cells that were custom before DD are restored with the color from `dd_pre`), then resends
    /// the current lighting settings with `[5,2]` to return to the firmware animation.
    /// Flag operations do not save — the user custom colors on flash are preserved.
    fn direct_drive_stop(&self, area: u8) -> HubResult<()> {
        if !self
            .dd_pre
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&area)
        {
            return Ok(());
        }
        // direct_drive with an empty set = release only the tracked pins (untracked cells are not touched).
        self.direct_drive(area, &BTreeMap::new())?;
        if self
            .dd_pins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&area)
            .is_some_and(|pins| !pins.is_empty())
        {
            return Err(HubError::Protocol(
                "DirectDrive: LED restore did not complete".into(),
            ));
        }
        let r = self.cmd(grp::LIGHTING, sub::READ, &[area, 0])?;
        self.cmd(
            grp::LIGHTING,
            sub::SETTING,
            &[area, 0, r[4], r[5], r[6], r[7], r[8], r[9]],
        )?;
        self.dd_pre
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&area);
        Ok(())
    }

    /// The `[5,3]` response returns **the current drawn color of all cells** (BGR is always the current frame).
    /// flag is a marker, not a lit indicator — 0xFF=user/DD custom,
    /// 0xF0=cell marked by the firmware, 0=unmarked (the color still comes). The official HUB's preview
    /// shows these for all cells as is (key color = the real device's current color. No simulation).
    fn live_lighting(&self, area: u8) -> HubResult<BTreeMap<usize, Rgb>> {
        if area != 0 {
            return Err(HubError::Unsupported("live_lighting"));
        }
        self.alive()?;
        let layout = registry::su66max_layout();
        // cell → layout index. For multi-cell keys such as Space, adopt the color of the first cell.
        let mut cell_key = [usize::MAX; 135];
        for (i, k) in layout.iter().enumerate() {
            for cell in led_cells(k) {
                if cell < 135 && cell_key[cell] == usize::MAX {
                    cell_key[cell] = i;
                }
            }
        }
        let mut out = BTreeMap::new();
        for c in 0..9u8 {
            let rb = self.cmd(grp::LIGHTING, sub::LIGHT_READBACK, &[0, c])?;
            for i in 0..15usize {
                let cell = c as usize * 15 + i;
                // Judge "is it lit" by the color register, not by the flag:
                // the flag (0xF0/0xFF) is set only for cells marked by the firmware/user,
                // and some cells output a color while staying unmarked (confirmed on the real device with litpoll — in a
                // session without writes, the colors flow even when every cell has flag=0).
                // Cells that are off with color 000 are not put into the map (the UI draws unmarked = off).
                let (b, g, r, f) = (rb[4 + i * 4], rb[5 + i * 4], rb[6 + i * 4], rb[7 + i * 4]);
                if (f != 0 || r != 0 || g != 0 || b != 0) && cell_key[cell] != usize::MAX {
                    out.insert(cell_key[cell], Rgb::new(r, g, b));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_roundtrip() {
        assert_eq!(code_to_action(0), KeyAction::None);
        assert_eq!(code_to_action(1), KeyAction::Transparent);
        assert_eq!(code_to_action(0x04), KeyAction::Code(4));
        assert_eq!(action_to_code(&KeyAction::Transparent).unwrap(), 1);
        assert_eq!(action_to_code(&KeyAction::Code(0xF101)).unwrap(), 0xF101);
        // combo: 0x1000 | mods<<8 | usage
        assert_eq!(
            action_to_code(&KeyAction::Combo { mods: 1, code: 4 }).unwrap(),
            0x1104
        );
        assert_eq!(
            code_to_action(0x1104),
            KeyAction::Combo { mods: 1, code: 4 }
        );
        assert!(action_to_code(&KeyAction::Combo {
            mods: 1,
            code: 0x104
        })
        .is_err());
    }

    #[test]
    fn default_stream_layer0() {
        let s = default_stream(0);
        // Esc is (1,0) → stream[30]
        assert_eq!(s[30], 0x29);
        // A is (3,1) → stream[3*30+1]
        assert_eq!(s[91], 0x04);
        // Space (5,4) → stream[154]
        assert_eq!(s[5 * 30 + 4], 0x2C);
        // Fn (5,10) → 0xF101
        assert_eq!(s[5 * 30 + 10], 0xF101);
        // Layer 1 is all transparent
        let s1 = default_stream(1);
        assert_eq!(s1[30], 1);
    }

    #[test]
    fn fw_parse() {
        let mut r = [0u8; 64];
        r[2..9].copy_from_slice(b" 49142 ");
        r[40..56].copy_from_slice(b"2026101216:41:03");
        assert_eq!(parse_fw(&r).as_deref(), Some("2026101216:41:03"));
    }

    // ---- live_axis piggyback / passive mode ----

    /// `[4,3,kind,row]` axis frame. Puts mark in cell0.
    fn axis_frame(kind: u8, row: u8, mark: u16) -> Vec<u8> {
        let mut f = vec![4u8, 3, kind, row];
        for i in 0..30 {
            let v = if i == 0 { mark } else { 0 };
            f.extend_from_slice(&v.to_le_bytes());
        }
        f
    }

    /// Queues all kind×row (20 frames).
    fn seed_all(d: &XsydDriver, mark: u16) {
        for row in 1..=5 {
            for kind in 0..=3 {
                d.ch.mock_reply(&axis_frame(kind, row, mark));
            }
        }
    }

    #[test]
    fn live_axis_piggyback_sends_no_requests() {
        let d = mock_driver();
        // A situation where only frames triggered by requests from the official HUB etc. are flowing.
        seed_all(&d, 1500);
        let out = d.live_axis().unwrap();
        // Everything can be fetched from the cache by piggybacking, so 0 requests.
        assert!(d.ch.mock_sent().is_empty());
        // The values come from the cache (route is /1000 → 1.5mm).
        assert!(out.iter().any(|s| (s.route - 1.5).abs() < 0.001));
        // Passive mode has been set by external traffic detection.
        assert!(
            *d.axis_passive.lock().unwrap() > Instant::now(),
            "only external frames are expected, so passive mode should be entered"
        );
    }

    #[test]
    fn live_axis_passive_never_requests() {
        let d = mock_driver();
        // Passive mode with 1 external frame.
        d.ch.mock_reply(&axis_frame(1, 1, 1000));
        d.drain_axis(5);
        assert!(*d.axis_passive.lock().unwrap() > Instant::now());
        // Even if the queue is empty no request is issued and it returns Ok (missing cells are 0).
        let out = d.live_axis().unwrap();
        assert!(d.ch.mock_sent().is_empty());
        assert_eq!(out.len(), registry::su66max_layout().len());
    }

    #[test]
    fn drain_foreign_command_frame_triggers_passive() {
        let d = mock_driver();
        // A situation where official settings responses ([3,1,...] etc.) are flowing — OBS
        // only issues axis requests, so a non-axis frame = someone else's traffic.
        let mut f = vec![3u8, 1];
        f.resize(64, 0);
        d.ch.mock_reply(&f);
        assert!(d.drain_axis(5));
        assert!(*d.axis_passive.lock().unwrap() > Instant::now());
        // Someone else's command releases the device-side stream,
        // so we send nothing from our side.
        assert!(d.ch.mock_sent().is_empty());
    }

    #[test]
    fn drain_ignores_own_command_responses() {
        let d = mock_driver();
        // A situation where I sent [3,1] just before → that response is not external.
        d.cmd_self.lock().unwrap().insert((3, 1), Instant::now());
        let mut f = vec![3u8, 1];
        f.resize(64, 0);
        d.ch.mock_reply(&f);
        assert!(!d.drain_axis(5));
        assert!(*d.axis_passive.lock().unwrap() < Instant::now());
    }

    #[test]
    fn axis_row_ignores_foreign_frames_and_caches_them() {
        let d = mock_driver();
        // Frames of another kind/row → arrive before my own echo, in that order.
        d.ch.mock_reply(&axis_frame(3, 5, 777));
        d.ch.mock_reply(&axis_frame(1, 1, 2048));
        let v = d.axis_row(1, 1).unwrap();
        assert_eq!(v[0], 2048);
        // Frames outside the echo go into the cache without being discarded (used for piggybacking).
        let c = d.axis_cache.lock().unwrap();
        assert_eq!(c.get(&(3, 5)).unwrap().1[0], 777);
        // My request has been recorded.
        assert!(d.axis_self.lock().unwrap().contains_key(&(1, 1)));
    }

    #[test]
    fn live_axis_requests_when_cache_cold() {
        let d = mock_driver();
        // No cache and no passive mode → issue a request and wait for the response → the mock
        // returns no response, so it is an error. The behavior needed for disconnect detection remains.
        assert!(d.live_axis().is_err());
        let sent = d.ch.mock_sent();
        assert!(sent.iter().any(|s| s.len() >= 4 && s[0] == 4 && s[1] == 3));
    }

    /// Boundary values of the lighting wire layout (matches the official lightingBaseData).
    #[test]
    fn lighting_scale() {
        // caps range = wire range = 0..100 raw values. Direction is only Forward=0/Backward=1.
        let xs = XsydInfo {
            layers: 2,
            rt_step: 0.01,
            travel: 4.0,
            polling: vec![],
            configs: 1,
            light_areas: vec![0],
            macro_space: None,
            axes: vec![],
            double_lighting: true,
        };
        let caps = build_caps(&xs).lighting;
        assert_eq!(caps[0].brightness_max, 100);
        assert_eq!(caps[0].speed_max, 100);
        assert_eq!(caps[0].directions.len(), 2);
    }

    /// Real-device fixture: `fixtures/su66max/perf.resp.bin` (r1c0, captured 2026-11-15).
    /// mode=0, nP=nR=2.0, rtFirst=0.5, rtP=rtR=0.3, pDead=rDead=0.2, axis=0
    #[test]
    fn perf_decode_fixture() {
        let bin = include_bytes!("../fixtures/su66max/perf.resp.bin");
        let mut r = [0u8; 64];
        r.copy_from_slice(&bin[..64]);
        let p = XsydDriver::decode_perf(&r);
        assert_eq!(p.mode, TriggerMode::Normal);
        assert_eq!(p.actuation, 2.0);
        assert_eq!(p.release, Some(2.0));
        assert_eq!(p.rt_first, 0.5);
        assert_eq!(p.rt_press, 0.3);
        assert_eq!(p.rt_release, 0.3);
        assert_eq!(p.top_deadzone, 0.2);
        assert_eq!(p.bottom_deadzone, 0.2);
        assert_eq!(p.switch_type, 0);
    }

    /// rapid mode=1 → TriggerMode::Rapid + continuous (official vn: RT=1).
    /// mode=2 (IntervalRT) → Rapid + non-continuous.
    #[test]
    fn perf_decode_rapid() {
        let mut r = [0u8; 64];
        r[4] = 1;
        let p = XsydDriver::decode_perf(&r);
        assert_eq!(p.mode, TriggerMode::Rapid);
        assert!(p.continuous);
        r[4] = 2;
        let p = XsydDriver::decode_perf(&r);
        assert_eq!(p.mode, TriggerMode::Rapid);
        assert!(!p.continuous);
    }

    /// DKS trps bitmask ↔ phase round trip.
    #[test]
    fn trp_roundtrip() {
        use DksPhase::*;
        for (trp, expect) in [
            (0u8, [Off, Off, Off, Off]),
            (1, [Tap, Off, Off, Off]),
            (2, [Hold, Off, Off, Off]),
            (4, [Off, Tap, Off, Off]),
            (8 | 16, [Off, Hold, Off, Off]),
            (32, [Off, Off, Tap, Off]),
            (64, [Off, Off, Hold, Off]),
            (128, [Off, Off, Off, Tap]),
        ] {
            assert_eq!(trp_to_phases(trp), expect, "trp={trp:#04x}");
            assert_eq!(phases_to_trp(&expect), trp);
        }
        // Hold takes priority: b0+b1 both set → Hold
        assert_eq!(trp_to_phases(3)[0], Hold);
    }

    /// Macro event u32 packing (same as the SDK: bit31=down, bits16-30=delay, low16=keycode).
    #[test]
    fn macro_event_pack() {
        let w = (1u32 << 31) | (100u32 << 16) | 0x04;
        let b = w.to_le_bytes();
        let back = u32::from_le_bytes(b);
        assert_eq!(back & 0xFFFF, 0x04);
        assert_eq!((back >> 16) & 0x7FFF, 100);
        assert_eq!(back >> 31, 1);
    }

    /// Labels of polling rate wire codes.
    #[test]
    fn polling_labels() {
        assert_eq!(polling_label(0), "8000 Hz");
        assert_eq!(polling_label(3), "1000 Hz");
        assert_eq!(polling_label(6), "125 Hz");
    }

    /// LED cell mapping — coordinates fixed by visual inspection on the real device (cell = row*21 + col).
    #[test]
    fn led_cell_map() {
        let layout = registry::su66max_layout();
        let at = |label: &str| layout.iter().find(|k| k.label == label).unwrap().clone();
        assert_eq!(led_cells(&at("Esc")), vec![21]);
        assert_eq!(led_cells(&at("9")), vec![30]);
        assert_eq!(led_cells(&at("W")), vec![44]);
        // The right edge is confirmed by real-device probes — LED column = matrix column. ' also has the dead pad on its right end
        // (cell75) under itself. LShift likewise (cell85).
        assert_eq!(led_cells(&at("'")), vec![74, 75]);
        assert_eq!(led_cells(&at("Enter")), vec![76]);
        assert_eq!(led_cells(&at("↑")), vec![97]);
        assert_eq!(led_cells(&at("Del")), vec![98]);
        assert_eq!(led_cells(&at("LShift")), vec![84, 85]);
        assert_eq!(led_cells(&at("V")), vec![89]);
        assert_eq!(led_cells(&at("B")), vec![90]);
        // Row 5 is the measured table (compressed right side + space bar with multiple LEDs).
        assert_eq!(led_cells(&at("LCtrl")), vec![105]);
        assert_eq!(led_cells(&at("RAlt")), vec![114]);
        assert_eq!(led_cells(&at("Fn")), vec![115]);
        assert_eq!(led_cells(&at("←")), vec![117]);
        assert_eq!(led_cells(&at("→")), vec![119]);
        let spaces: Vec<usize> = layout
            .iter()
            .filter(|k| k.label == "Space")
            .flat_map(led_cells)
            .collect();
        assert_eq!(spaces, vec![109, 110, 111, 112, 113]);
    }

    /// Advanced key wire round trip — can what was written with `higher_packets` be read back
    /// with `decode_higher` as a `[6,1]` response (including delay fields).
    #[test]
    fn higher_wire_roundtrip() {
        let layout = registry::su66max_layout();
        let idx = |label: &str| {
            layout
                .iter()
                .position(|k| k.label == label)
                .unwrap_or_else(|| panic!("{label} not in layout"))
        };
        let roundtrip = |key_label: &str, b: Advanced| {
            let (row, col) = {
                let k = layout.iter().find(|k| k.label == key_label).unwrap();
                registry::su66max_matrix_pos(k.id)
            };
            let packets = higher_packets(row, col, Some(&b)).unwrap();
            // Synthesized with the same layout as the response r[2..]=row,col,mode,... (r[4]=mode).
            let mut r = [0u8; 64];
            r[2..2 + packets[0].len()].copy_from_slice(&packets[0]);
            decode_higher(&r).unwrap_or_else(|| panic!("{key_label} decode failed"))
        };

        // TGL: kc@5..6, time@7..8
        match roundtrip(
            "A",
            Advanced::Tgl {
                code: 0x04,
                delay_ms: 120,
            },
        ) {
            Advanced::Tgl { code, delay_ms } => {
                assert_eq!(code, 0x04);
                assert_eq!(delay_ms, 120);
            }
            other => panic!("TGL → {other:?}"),
        }
        // END: kcs@5..8, delay@9..10
        match roundtrip(
            "A",
            Advanced::End {
                code: 0x04,
                code_release: 0x1E,
                delay_ms: 80,
            },
        ) {
            Advanced::End {
                code,
                code_release,
                delay_ms,
            } => {
                assert_eq!((code, code_release, delay_ms), (0x04, 0x1E, 80));
            }
            other => panic!("END → {other:?}"),
        }
        // MT: kcs@5..8, time@9..10
        match roundtrip(
            "A",
            Advanced::Mt {
                tap: 0x04,
                hold: 0xE1,
                hold_ms: 200,
            },
        ) {
            Advanced::Mt { tap, hold, hold_ms } => {
                assert_eq!((tap, hold, hold_ms), (0x04, 0xE1, 200));
            }
            other => panic!("MT → {other:?}"),
        }
        // SOCD: pair@5..6, kcs@7..10, delay@11..12, socdMode@13 (the structure of the 2nd packet is also checked)
        for (mode, x0, x1) in [
            (SocdMode::Last, 0u8, 0u8),
            (SocdMode::PriorityA, 1, 2),
            (SocdMode::PriorityB, 2, 1),
            (SocdMode::Neutral, 3, 3),
        ] {
            let b = Advanced::Socd {
                other: idx("D"),
                code_a: 0x04,
                code_b: 0x07,
                mode,
                delay_ms: 33,
            };
            let (row, col) = registry::su66max_matrix_pos(layout[idx("A")].id);
            let packets = higher_packets(row, col, Some(&b)).unwrap();
            assert_eq!(packets.len(), 2);
            // Write packet = [row,col,mode,r2,c2,kcA,kcB,delay,x] → socdMode is at index 11
            assert_eq!(packets[0][11], x0);
            assert_eq!(packets[1][11], x1);
            // Packet 2 has the pair and key codes reversed (kcB@5..6, kcA@7..8)
            assert_eq!(u16::from_le_bytes([packets[1][5], packets[1][6]]), 0x07);
            assert_eq!(u16::from_le_bytes([packets[1][7], packets[1][8]]), 0x04);
            let mut r = [0u8; 64];
            r[2..2 + packets[0].len()].copy_from_slice(&packets[0]);
            match decode_higher(&r) {
                Some(Advanced::Socd {
                    other,
                    code_a,
                    code_b,
                    mode: m,
                    delay_ms,
                }) => {
                    assert_eq!(other, idx("D"));
                    assert_eq!((code_a, code_b, delay_ms), (0x04, 0x07, 33));
                    assert_eq!(m, mode);
                }
                other => panic!("SOCD {mode:?} → {other:?}"),
            }
        }
        // RS: same structure as SOCD (no mode byte)
        match roundtrip(
            "A",
            Advanced::Rs {
                other: idx("S"),
                code_a: 0x04,
                code_b: 0x16,
                delay_ms: 17,
            },
        ) {
            Advanced::Rs {
                other,
                code_a,
                code_b,
                delay_ms,
            } => {
                assert_eq!(other, idx("S"));
                assert_eq!((code_a, code_b, delay_ms), (0x04, 0x16, 17));
            }
            other => panic!("RS → {other:?}"),
        }
        // DKS: 4×kc@5..12, 4×trp@13..16, dbs@17..20 — bottom is db2
        match roundtrip(
            "A",
            Advanced::Dks {
                rows: vec![DksRow {
                    code: 0x04,
                    phases: [DksPhase::Hold, DksPhase::Off, DksPhase::Off, DksPhase::Tap],
                }],
                bottom: 3.0,
            },
        ) {
            Advanced::Dks { rows, bottom } => {
                assert_eq!(rows[0].code, 0x04);
                assert!((bottom - 3.0).abs() < 0.001);
            }
            other => panic!("DKS → {other:?}"),
        }
        // MPT: 3×kc@5..10, 3×db@11..16
        match roundtrip(
            "A",
            Advanced::Mpt {
                depths: [0.8, 2.0, 3.2],
                codes: [0x04, 0x16, 0x07],
            },
        ) {
            Advanced::Mpt { depths, codes } => {
                assert_eq!(codes, [0x04, 0x16, 0x07]);
                assert!((depths[0] - 0.8).abs() < 0.001);
                assert!((depths[2] - 3.2).abs() < 0.001);
            }
            other => panic!("MPT → {other:?}"),
        }
        // NONE (cleared) → decode_higher returns None
        let packets = higher_packets(3, 1, None).unwrap();
        assert_eq!(packets[0][2], hk::NONE);
    }

    // ---- Mock Channel fixture tests (replay of real-device response bytes) ----

    /// Builds a 64B response of `[group, ...tail]` (include the sub/argument echo in tail).
    fn reply(group: u8, tail: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(64);
        v.push(group);
        v.extend_from_slice(tail);
        v.resize(64, 0);
        v
    }

    /// Queues the responses to `capture_dd_pre` at DD session start (custom read of all 9 chunks)
    /// — all cells isCustom=0 (no user paint).
    fn mock_dd_pre_empty(d: &XsydDriver) {
        for c in 0..9u8 {
            d.ch.mock_reply(&reply(5, &[3, 0, c]));
        }
    }

    /// A minimal XsydDriver with DoubleLighting + areas=[0,1].
    fn mock_driver() -> XsydDriver {
        XsydDriver {
            info: DeviceInfo {
                id: DeviceId("test".into()),
                kind: DeviceKind::Keyboard,
                name: "mock".into(),
                vendor: "test".into(),
                model: "test".into(),
                vid: 0,
                pid: 0,
                transport: Transport::Wired,
                firmware: None,
                dongle_firmware: None,
                serial: None,
                driver: DriverKind::Xsyd,
                battery: None,
                supported: true,
                asleep: false,
                board_id: None,
            },
            ch: Channel::mock(),
            xs: XsydInfo {
                layers: 2,
                rt_step: 0.01,
                travel: 4.0,
                polling: vec![],
                configs: 1,
                light_areas: vec![0, 1],
                macro_space: None,
                axes: vec![],
                double_lighting: true,
            },
            caps: Capabilities::empty(),
            fails: AtomicU32::new(0),
            dead_since: Mutex::new(None),
            dd_pins: Mutex::new(BTreeMap::new()),
            dd_pre: Mutex::new(BTreeMap::new()),
            axis_cache: Mutex::new(HashMap::new()),
            axis_self: Mutex::new(HashMap::new()),
            cmd_self: Mutex::new(HashMap::new()),
            axis_passive: Mutex::new(Instant::now() - Duration::from_secs(1)),
        }
    }

    /// The same response sequence as the SU66 real device (area0): base=Open/mode15/#slot2/100/75 →
    /// verifies decoding into LightingState.
    #[test]
    fn lighting_read_fixture() {
        let d = mock_driver();
        // base [5,1,0,0] → r[4]=open=3 r[5]=15 r[6]=100 r[7]=75 r[8]=0 r[9]=sel=2
        d.ch.mock_reply(&reply(5, &[1, 0, 0, 3, 15, 100, 75, 0, 2]));
        // palette [5,1,0,1] → r[4..36]=[B,G,R,H]×8, slot2 = #204080
        let mut tail = vec![1, 0, 1];
        tail.resize(3 + 32, 0);
        tail[3 + 8] = 0x80; // B
        tail[3 + 9] = 0x40; // G
        tail[3 + 10] = 0x20; // R
        d.ch.mock_reply(&reply(5, &tail));
        // cc [5,1,0,2] → r[4,5,6] = R,G,B
        d.ch.mock_reply(&reply(5, &[1, 0, 2, 38, 35, 40]));
        // per-key [5,3,0,0..8]: chunk1 (cells 15..29) i=6 = cell21 (Esc) has custom
        for c in 0..9u8 {
            let mut t = vec![3, 0, c];
            t.resize(3 + 60, 0);
            if c == 1 {
                t[3 + 6 * 4] = 3; // B
                t[4 + 6 * 4] = 2; // G
                t[5 + 6 * 4] = 1; // R
                t[6 + 6 * 4] = 255; // isCustom
            }
            d.ch.mock_reply(&reply(5, &t));
        }
        let s = d.read_lighting(0).unwrap();
        assert!(s.on);
        assert_eq!(s.zone_on, vec![true, true]);
        assert_eq!(s.effect, 15);
        assert_eq!((s.brightness, s.speed, s.direction), (100, 75, 0));
        assert_eq!(s.palette_sel, 2);
        assert_eq!(s.color, Rgb::new(0x20, 0x40, 0x80));
        assert_eq!(s.color_correction, Some(Rgb::new(38, 35, 40)));
        assert_eq!(s.per_key.len(), 1);
        assert_eq!(*s.per_key.values().next().unwrap(), Rgb::new(1, 2, 3));
    }

    /// DirectDrive: sends `[5,5,area,chunk,…15 cells×4B(B,G,R,255)]` without waiting for a response.
    /// Chunks with only flag=0 are not sent (flag=0 is "don't touch").
    /// Verifies key index → LED cell expansion (Esc=21, space bar = multiple cells).
    #[test]
    fn direct_drive_wire() {
        let d = mock_driver();
        let layout = registry::su66max_layout();
        let esc = layout.iter().position(|k| k.label == "Esc").unwrap();
        let spc = layout
            .iter()
            .position(|k| led_cells(k).contains(&111))
            .unwrap();
        let mut cells = BTreeMap::new();
        cells.insert(esc, Rgb::new(255, 0, 0)); // red
        cells.insert(spc, Rgb::new(0, 0, 255)); // blue
        mock_dd_pre_empty(&d);
        d.direct_drive(0, &cells).unwrap();
        let sent = d.ch.mock_sent();
        // Only Esc=cell21 (chunk1) and Space=cell111 (chunk7) are included
        // (the 9 reads of capture_dd_pre are also mixed into sent, so pick up only the DD ones).
        let dd: Vec<_> = sent
            .iter()
            .filter(|f| f[1] == sub::LIGHT_DIRECTDRIVE)
            .collect();
        assert_eq!(dd.len(), 2);
        for f in &dd {
            assert_eq!(f[0], grp::LIGHTING);
            assert_eq!(f[2], 0); // area
            assert_eq!(f.len(), 64);
        }
        let c1 = dd.iter().find(|f| f[3] == 1).unwrap();
        // Esc → cell21 = 7th cell of chunk1 → [B,G,R,255]
        assert_eq!(&c1[4 + 6 * 4..8 + 6 * 4], &[0, 0, 255, 255]);
        let c7 = dd.iter().find(|f| f[3] == 7).unwrap();
        // space bar center cell111 = 7th cell of chunk7 (105-119)
        assert_eq!(&c7[4 + 6 * 4..8 + 6 * 4], &[255, 0, 0, 255]);
    }

    /// DirectDrive hybrid release: when a cell pinned in the previous frame
    /// drops out of the next frame's specified set, isCustom is cleared with `[5,4]`.
    /// The release write uses the readback current values, with only the target cell's flag set to 0.
    #[test]
    fn direct_drive_releases_dropped_cells() {
        let d = mock_driver();
        let layout = registry::su66max_layout();
        let esc = layout.iter().position(|k| k.label == "Esc").unwrap();
        let mut cells = BTreeMap::new();
        cells.insert(esc, Rgb::new(255, 0, 0));
        mock_dd_pre_empty(&d);
        d.direct_drive(0, &cells).unwrap();
        // Esc=cell21 → pin to chunk1. Next is empty → release all.
        // Release is chunk1 only — prepare the readback response (cell21 is isCustom=255, the others 0).
        let mut t = vec![3, 0, 1u8];
        t.resize(3 + 60, 0);
        t[3 + 6 * 4] = 0; // B
        t[4 + 6 * 4] = 0;
        t[5 + 6 * 4] = 255; // R
        t[6 + 6 * 4] = 255; // isCustom (pinned)
        t[3 + 7 * 4 + 3] = 255; // the neighboring cell22 is separately pinned by the user → should be preserved
        d.ch.mock_reply(&reply(5, &t));
        // ack to [5,4]
        d.ch.mock_reply(&reply(5, &[4, 0, 1]));
        d.direct_drive(0, &BTreeMap::new()).unwrap();
        let sent = d.ch.mock_sent();
        // Only [5,4] chunk1 is emitted (no DD frame is sent — no specified cells).
        let custom: Vec<_> = sent.iter().filter(|f| f[1] == sub::LIGHT_CUSTOM).collect();
        assert_eq!(custom.len(), 1, "frames: {sent:?}");
        let f = custom[0];
        assert_eq!(f[3], 1); // chunk1
                             // cell21 to release (index 6 in chunk) → flag=0, color keeps the readback value.
        assert_eq!(&f[4 + 6 * 4..8 + 6 * 4], &[0, 0, 255, 0]);
        // The user's pin cell22 (index7) → kept at flag=255.
        assert_eq!(f[7 + 7 * 4], 255);
    }

    /// Even if the same DirectDrive frame is passed repeatedly, cells already pinned and with the same color are
    /// not resent (pins persist on the real device, so avoid congestion at rest).
    #[test]
    fn direct_drive_skips_unchanged_frames() {
        let d = mock_driver();
        let layout = registry::su66max_layout();
        let esc = layout.iter().position(|k| k.label == "Esc").unwrap();
        let cells = BTreeMap::from([(esc, Rgb::new(255, 0, 0))]);
        mock_dd_pre_empty(&d);
        d.direct_drive(0, &cells).unwrap();
        let n = d.ch.mock_sent().len();
        d.direct_drive(0, &cells).unwrap();
        assert_eq!(d.ch.mock_sent().len(), n);
        // If the color changes, the chunk is resent even for the same cell.
        d.direct_drive(0, &BTreeMap::from([(esc, Rgb::new(0, 255, 0))]))
            .unwrap();
        let sent = d.ch.mock_sent();
        assert_eq!(sent.len(), n + 1);
        assert_eq!(sent.last().unwrap()[1], sub::LIGHT_DIRECTDRIVE);
    }

    #[test]
    fn direct_drive_never_writes_with_an_incomplete_backup() {
        let d = mock_driver();
        // If chunk 2 arrives as the response to chunk 1, do not finalize the save.
        d.ch.mock_reply(&reply(5, &[3, 0, 0]));
        d.ch.mock_reply(&reply(5, &[3, 0, 2]));
        let esc = registry::su66max_layout()
            .iter()
            .position(|k| k.label == "Esc")
            .unwrap();
        assert!(d
            .direct_drive(0, &BTreeMap::from([(esc, Rgb::new(255, 0, 0))]))
            .is_err());
        d.direct_drive_stop(0).unwrap();
        assert_eq!(
            d.ch.mock_sent().iter().map(|f| f[1]).collect::<Vec<_>>(),
            [3, 3]
        );
    }

    #[test]
    fn direct_drive_stop_retries_with_the_original_custom_color() {
        let mut d = mock_driver();
        d.xs.configs = 2;
        for c in 0..9 {
            let mut rb = reply(5, &[3, 0, c]);
            if c == 1 {
                rb[28..32].copy_from_slice(&[3, 2, 1, 255]); // Esc user color
            }
            d.ch.mock_reply(&rb);
        }
        let esc = registry::su66max_layout()
            .iter()
            .position(|k| k.label == "Esc")
            .unwrap();
        d.direct_drive(0, &BTreeMap::from([(esc, Rgb::new(255, 0, 0))]))
            .unwrap();
        let mut lit = reply(5, &[3, 0, 1]);
        lit[28..32].copy_from_slice(&[0, 0, 255, 255]);
        d.ch.mock_reply(&lit);
        d.ch.mock_fail_send(&[5, 4, 0, 1]);
        assert!(d.apply(&Change::Profile(1)).is_err());
        assert!(d.ch.mock_sent().iter().all(|f| f[0] != grp::GLOBAL));

        d.ch.mock_reply(&lit);
        d.ch.mock_reply(&reply(5, &[4, 0, 1]));
        d.ch.mock_reply(&reply(5, &[1, 0, 0, 1, 1, 50, 50, 0, 0]));
        d.ch.mock_reply(&reply(5, &[2, 0, 0]));
        d.direct_drive_stop(0).unwrap();
        let sent = d.ch.mock_sent();
        assert_eq!(sent.iter().filter(|f| f[1] == 2).count(), 1);
        assert_eq!(
            &sent.iter().rfind(|f| f[1] == 4).unwrap()[28..32],
            &[3, 2, 1, 255]
        );
        d.direct_drive_stop(0).unwrap();
        assert_eq!(d.ch.mock_sent().len(), sent.len());
    }

    #[test]
    fn direct_drive_stop_restores_chunks_after_a_partial_frame() {
        let d = mock_driver();
        let layout = registry::su66max_layout();
        let esc = layout.iter().position(|k| k.label == "Esc").unwrap();
        let space = layout
            .iter()
            .position(|k| led_cells(k).contains(&111))
            .unwrap();
        let frame = BTreeMap::from([(esc, Rgb::new(1, 2, 3)), (space, Rgb::new(4, 5, 6))]);
        mock_dd_pre_empty(&d);
        d.ch.mock_fail_send(&[5, 5, 0, 7]);
        assert!(d.direct_drive(0, &frame).is_err());
        for c in [1, 7] {
            d.ch.mock_reply(&reply(5, &[3, 0, c]));
            d.ch.mock_reply(&reply(5, &[4, 0, c]));
        }
        d.ch.mock_reply(&reply(5, &[1, 0, 0, 1, 1, 50, 50, 0, 0]));
        d.ch.mock_reply(&reply(5, &[2, 0, 0]));
        d.direct_drive_stop(0).unwrap();
        let restored: Vec<_> =
            d.ch.mock_sent()
                .iter()
                .filter(|f| f[1] == 4)
                .map(|f| f[3])
                .collect();
        assert_eq!(restored, [1, 7]);
    }

    /// Wire of the base write: `[5,2,area,0,open,mode,lum,spd,dir,sel]` and
    /// verifying that "parts identical to the current are not sent".
    #[test]
    fn lighting_write_wire() {
        let d = mock_driver();
        // cur base (changed → only the base frame is emitted)
        d.ch.mock_reply(&reply(5, &[1, 0, 0, 1, 1, 50, 50, 0, 0]));
        // ack echo to base SET
        d.ch.mock_reply(&reply(5, &[2, 0, 0]));
        // cur palette: slot2 equals the target color → no palette write
        let mut tail = vec![1, 0, 1];
        tail.resize(3 + 32, 0);
        tail[3 + 8] = 0x80;
        tail[3 + 9] = 0x40;
        tail[3 + 10] = 0x20;
        d.ch.mock_reply(&reply(5, &tail));
        // cur cc = same as the target → no cc write
        d.ch.mock_reply(&reply(5, &[1, 0, 2, 38, 35, 40]));
        let s = LightingState {
            on: true,
            effect: 15,
            color: Rgb::new(0x20, 0x40, 0x80),
            brightness: 100,
            speed: 75,
            direction: 0,
            per_key: BTreeMap::new(),
            zone_on: vec![true, true],
            palette: vec![],
            palette_sel: 2,
            color_correction: Some(Rgb::new(38, 35, 40)),
        };
        d.write_lighting(0, &s).unwrap();
        let sent = d.ch.mock_sent();
        // sent also contains READs — extract and verify only the SET frames.
        let sets: Vec<_> = sent.iter().filter(|f| f[1] == sub::SETTING).collect();
        assert_eq!(sets.len(), 1, "frames: {sent:?}");
        assert_eq!(&sets[0][..10], &[5, 2, 0, 0, 3, 15, 100, 75, 0, 2]);
    }

    /// jo enum synthesis: both=3 / upper only=2 / lower only=1 / closed=0. Non-DoubleLighting
    /// areas use a boolean (on→1).
    #[test]
    fn lighting_zone_open() {
        let d = mock_driver();
        let mk = |zone_on: Vec<bool>, on: bool| LightingState {
            on,
            effect: 0,
            color: Rgb::new(0, 0, 0),
            brightness: 0,
            speed: 0,
            direction: 0,
            per_key: BTreeMap::new(),
            zone_on,
            palette: vec![],
            palette_sel: 0,
            color_correction: None,
        };
        assert_eq!(d.compose_open(0, &mk(vec![true, true], true)), 3);
        assert_eq!(d.compose_open(0, &mk(vec![true, false], true)), 2);
        assert_eq!(d.compose_open(0, &mk(vec![false, true], true)), 1);
        assert_eq!(d.compose_open(0, &mk(vec![false, false], false)), 0);
        // When zone_on is absent, on→3 (DoubleLighting)
        assert_eq!(d.compose_open(0, &mk(vec![], true)), 3);
        // SingleLighting area is a boolean
        assert_eq!(d.compose_open(1, &mk(vec![], true)), 1);
        assert_eq!(d.compose_open(1, &mk(vec![], false)), 0);
    }

    /// The official export is CryptoJS `AES.encrypt` = OpenSSL `Salted__` +
    /// EVP_BytesToKey(MD5). The vector was made with OpenSSL itself:
    /// `openssl enc -aes-256-cbc -md md5 -salt -pass pass:<PASS> -base64 -A`.
    #[test]
    fn official_export_decrypts_like_openssl() {
        let d = mock_driver();
        let v = serde_json::json!({
            "data": "U2FsdGVkX18ZKqE4GBzY7wDA+mdxVbKnnzSO4cB1jmN2TU5BEFW9B0PmdSt8Og3lAKaTAEJrgf6G8kZegB1NtA=="
        });
        let plain = d.official_plain(&v).unwrap();
        assert_eq!(
            plain,
            serde_json::json!({"keyboards": [{"name": "SU66 Max"}], "v": 1})
        );
        // Plain JSON passes through; a wrong header is refused.
        let raw = serde_json::json!({"keyboards": []});
        assert_eq!(d.official_plain(&raw).unwrap(), raw);
        let bad = serde_json::json!({"data": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"});
        assert!(d.official_plain(&bad).is_err());
    }
}
