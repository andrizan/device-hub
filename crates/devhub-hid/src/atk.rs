//! ATK HUB (hub.atk.pro) family mice — 16B report-8 + EEPROM protocol.
//!
//! Based on analysis of the official ATK HUB (hub-v3); verified on a real F1 V3 ULTIMATE+ (373b:12a7).
//! The wire format is in the same family as mouse.fit:
//!
//! - Settings collection usage_page 0xFF02 / usage 2, output/input report id 8.
//! - Request  `[cmd, 0, addrHi, addrLo, len, data…]` (16B, cksum in the 17th byte)
//! - Response  `[rid=8, cmd echo, status, addrHi, addrLo, len, data…, cksum]` (17B)
//! - cksum = `0x55 - (sum of all bytes & 0xff)` (17B total ≡ 0x55)
//! - EEPROM: read cmd8, write cmd7 (10B chunks). Scalars are (v, 0x55-v) pairs.
//!
//! EEPROM map (real device + official JS `y.Address`):
//! ReportRate block @0 (10B: rate,maxDpi,currentDpi,bhop,btnOpMode + a crc for each),
//! LOD @10, DPI color @44 (4B×8), key assignment @96 (4B×16), MotionSync @171,
//! CloseLedTime @173, angle snapping @175, ripple @177, Sensor group @181/183/185,
//! RfTxTime @187, WheelDebounce @227, Shortcut @256 (32B×16),
//! Macro @768 (384B×16), PAW3955 DPI @6912 (6B×8), DynamicSensitivity @6960.
//!
//! Other cmds: 3=online, 4=battery, 5/6=pair, 9=factory, 14/15=configID,
//! 16=CID/MID (F1 V3 = "1,62"), 18=FW, 22/23=long range, 29=dongle FW.
//! CID=1 models all appear to use this 16B scheme (CID=2 models are a different generation).

use devhub_core::backend::*;
use devhub_core::model::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

const RID: u8 = 8;
const CHUNK: u16 = 10;

#[allow(dead_code)]
mod ee {
    pub const REPORT_RATE: u16 = 0;
    pub const MAX_DPI_STAGE: u16 = 2;
    pub const CURRENT_DPI: u16 = 4;
    pub const LOD: u16 = 10;
    pub const DPI_COLOR: u16 = 44;
    pub const KEY_FUNCTION: u16 = 96;
    pub const DEBOUNCE: u16 = 169;
    pub const MOTION_SYNC: u16 = 171;
    /// Sleep time — official closeLedTime. The wire value is in 10-second units (6 = 60s = 1 min).
    pub const CLOSE_LED: u16 = 173;
    pub const ANGLE_SNAP: u16 = 175;
    pub const RIPPLE: u16 = 177;
    /// LED off while moving (moveCloseLights).
    pub const MOVE_LED: u16 = 179;
    /// Per-button shortcut area — 32B × slot (slot index = physical button).
    pub const SHORTCUT: u16 = 256;
    pub const WHEEL_DEBOUNCE: u16 = 227;
    pub const DPI_3955: u16 = 6912;
}

/// cmd16 response (data[0]=CID, data[1]=MID) → (display name, model id).
/// It is relayed through the dongle too, so don't keep the receiver's name.
fn mid_name(cid: u8, mid: u8) -> Option<(&'static str, &'static str)> {
    match (cid, mid) {
        (1, 62) => Some(("ATK F1 V3 ULTIMATE+", "atk-f1v3u")),
        _ => None,
    }
}

// ==================================================================== structure

pub struct AtkDriver {
    dev: Mutex<hidapi::HidDevice>,
    info: Mutex<DeviceInfo>,
    caps: Capabilities,
    events: Option<std::sync::Arc<dyn Fn(HubEvent) + Send + Sync>>,
    /// Whether the model has been determined from CID/MID (stays undetermined while asleep behind a dongle).
    detected: Mutex<bool>,
}

pub fn open(
    dev: hidapi::HidDevice,
    info: DeviceInfo,
    events: Option<std::sync::Arc<dyn Fn(HubEvent) + Send + Sync>>,
) -> HubResult<AtkDriver> {
    dev.set_blocking_mode(false).ok();
    let d = AtkDriver {
        dev: Mutex::new(dev),
        info: Mutex::new(info),
        caps: build_caps(),
        events,
        detected: Mutex::new(false),
    };
    // Determine the model from CID/MID (replace the name if a response arrives).
    d.maybe_detect();
    Ok(d)
}

impl AtkDriver {
    /// Notifies the UI of a change to `info`.
    fn emit_info(&self, i: DeviceInfo) {
        if let Some(ev) = &self.events {
            ev(HubEvent::Info(i.id.clone(), Box::new(i)));
        }
    }

    /// If CID/MID is not yet determined, try cmd16 and, if it succeeds, replace the model name.
    /// This fails while the mouse is asleep behind a dongle, so it is settled by a call after wake-up.
    fn maybe_detect(&self) {
        if *self.detected.lock().unwrap_or_else(|p| p.into_inner()) {
            return;
        }
        let Ok(r) = self.cmd_raw(16, 0, &[]) else {
            return;
        };
        let (cid, mid) = (
            r.get(6).copied().unwrap_or(0),
            r.get(7).copied().unwrap_or(0),
        );
        if cid == 0 && mid == 0 {
            return;
        }
        *self.detected.lock().unwrap_or_else(|p| p.into_inner()) = true;
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner());
        let mut dirty = false;
        if let Some((name, model)) = mid_name(cid, mid) {
            if i.name != name {
                i.name = name.into();
                i.model = model.into();
                dirty = true;
            }
        }
        if dirty {
            let info = i.clone();
            drop(i);
            self.emit_info(info);
        }
    }

    /// Updates the asleep flag (only meaningful behind a dongle).
    fn set_asleep(&self, asleep: bool) {
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner());
        if i.transport != Transport::Wireless24G || i.asleep == asleep {
            return;
        }
        i.asleep = asleep;
        let info = i.clone();
        drop(i);
        self.emit_info(info);
    }

    /// 16B command → 17B response (picks up the cmd echo; up to 3 tries).
    fn cmd_raw(&self, cmd: u8, addr: u16, data: &[u8]) -> HubResult<Vec<u8>> {
        let mut f = [0u8; 17];
        f[0] = RID;
        f[1] = cmd;
        f[3] = (addr >> 8) as u8;
        f[4] = addr as u8;
        f[5] = data.len().min(10) as u8;
        f[6..6 + data.len().min(10)].copy_from_slice(&data[..data.len().min(10)]);
        let s: u32 = f[..16].iter().map(|&b| b as u32).sum();
        f[16] = 85u32.wrapping_sub(s) as u8;
        let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
        for _ in 0..3 {
            dev.write(&f)
                .map_err(|e| HubError::Io(format!("atk write: {e}")))?;
            let deadline = Instant::now() + Duration::from_millis(400);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let mut buf = [0u8; 64];
                match dev.read_timeout(&mut buf, left.as_millis().min(120) as i32) {
                    Ok(n) if n >= 17 && buf[1] == cmd => {
                        self.set_asleep(false);
                        return Ok(buf[..17].to_vec());
                    }
                    _ => continue,
                }
            }
            sleep(Duration::from_millis(20));
        }
        // No response behind a dongle = treated as the mouse being asleep.
        self.set_asleep(true);
        Err(HubError::Protocol("atk: response timeout".into()))
    }

    /// EEPROM read (10B chunks, joined internally). Data is resp[6..].
    fn ee_read(&self, addr: u16, len: u16) -> HubResult<Vec<u8>> {
        let mut out = Vec::with_capacity(len as usize);
        let mut a = addr;
        while (out.len() as u16) < len {
            let want = (len - out.len() as u16).min(CHUNK) as u8;
            // GetEEPROM: len is the request's [4] (the len field, not data.len()=0).
            let mut f = [0u8; 17];
            f[0] = RID;
            f[1] = 8;
            f[3] = (a >> 8) as u8;
            f[4] = a as u8;
            f[5] = want;
            let s: u32 = f[..16].iter().map(|&b| b as u32).sum();
            f[16] = 85u32.wrapping_sub(s) as u8;
            let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
            let mut got = None;
            for _ in 0..3 {
                dev.write(&f)
                    .map_err(|e| HubError::Io(format!("atk write: {e}")))?;
                let deadline = Instant::now() + Duration::from_millis(400);
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    let mut buf = [0u8; 64];
                    match dev.read_timeout(&mut buf, left.as_millis().min(120) as i32) {
                        Ok(n) if n >= 17 && buf[1] == 8 => {
                            got = Some(buf);
                            break;
                        }
                        _ => continue,
                    }
                }
                if got.is_some() {
                    break;
                }
                sleep(Duration::from_millis(20));
            }
            drop(dev);
            let buf = got.ok_or_else(|| HubError::Protocol("atk: ee timeout".into()))?;
            let n = (buf[5] as usize).min(10).min(want as usize);
            out.extend_from_slice(&buf[6..6 + n]);
            if n < want as usize {
                break;
            }
            a += want as u16;
        }
        Ok(out)
    }

    /// EEPROM write (SetEEPROM cmd7, 10B chunks).
    fn ee_write(&self, addr: u16, data: &[u8]) -> HubResult<()> {
        for (i, chunk) in data.chunks(CHUNK as usize).enumerate() {
            self.cmd_raw(7, addr + (i * CHUNK as usize) as u16, chunk)?;
            sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Scalar (v, 0x55-v) pair write.
    fn ee_write_val(&self, addr: u16, val: u8) -> HubResult<()> {
        self.ee_write(addr, &[val, 85u8.wrapping_sub(val)])
    }

    /// Scalar read, with pair verification.
    fn ee_u8(&self, addr: u16) -> HubResult<u8> {
        let b = self.ee_read(addr, 2)?;
        if (b[0] as u16 + b[1] as u16) & 0xff == 85 {
            Ok(b[0])
        } else {
            Err(HubError::Protocol(format!("eeprom @{addr}: bad pair")))
        }
    }

    /// Reads the 10B block at @0, replaces byte idx (= even offset) and writes it back.
    fn cfg_byte(&self, idx: usize, val: u8) -> HubResult<()> {
        let mut b = self.ee_read(0, 10)?;
        b[idx] = val;
        b[idx + 1] = 85u8.wrapping_sub(val);
        self.ee_write(0, &b)
    }

    fn crc(b: &[u8]) -> u8 {
        85u8.wrapping_sub(b[..b.len() - 1].iter().map(|&x| x as u32).sum::<u32>() as u8)
    }

    /// 3955 DPI record (6B): [xLo,xHi,yLo,yHi,flags,crc], raw+1 = dpi.
    fn parse_stage(b: &[u8]) -> (u32, u32) {
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
    }

    fn encode_stage(x: u32, y: u32) -> Vec<u8> {
        let l = x.saturating_sub(1);
        let s = y.saturating_sub(1);
        let mut b = vec![
            (l & 0xff) as u8,
            ((l >> 8) & 0xff) as u8,
            (s & 0xff) as u8,
            ((s >> 8) & 0xff) as u8,
            ((((l >> 16) & 3) << 2) | (((s >> 16) & 3) << 6)) as u8,
            0,
        ];
        b[5] = Self::crc(&b);
        b
    }

    /// KeyFunction @96 4B record → ButtonAction. type5=ShortcutKey reads the
    /// per-button area (@256+slot*32) separately.
    fn decode_keyfn(&self, slot: usize, b: &[u8]) -> ButtonAction {
        match b[0] {
            1 => ButtonAction::Mouse(match b[1] {
                2 => MouseFn::Right,
                4 => MouseFn::Middle,
                8 => MouseFn::Back,
                16 => MouseFn::Forward,
                _ => MouseFn::Left,
            }),
            2 => ButtonAction::Dpi(match b[1] {
                2 => DpiFn::Up,
                3 => DpiFn::Down,
                _ => DpiFn::Cycle,
            }),
            4 if b[1] == 100 => ButtonAction::Mouse(MouseFn::DoubleClick),
            4 => ButtonAction::Fire {
                button: MouseFn::Left,
                count: b[2],
                interval_ms: b[1] as u16,
            },
            6 => ButtonAction::Macro {
                slot: b[1].saturating_sub(1),
                run: match b[2] {
                    253 => MacroRun::Toggle,
                    254 => MacroRun::WhileHeld,
                    n => MacroRun::Times(n as u16),
                },
            },
            5 => self.read_shortcut(slot).unwrap_or(ButtonAction::Disabled),
            7 => ButtonAction::PollingCycle,
            10 => ButtonAction::Dpi(DpiFn::Shift(((b[1] as u32) + 1) * 50)),
            11 => ButtonAction::Mouse(if b[1] == 2 {
                MouseFn::ScrollDown
            } else {
                MouseFn::ScrollUp
            }),
            _ => ButtonAction::Disabled,
        }
    }

    /// Reads the ShortcutKey area (@256 + slot*32).
    /// Format: [count, entry…, sum] — entry = {keyState, v1, v2},
    /// keyState = 0x80 (press) | 0x40 (release) + keyType (1=Keyboard 2=Fire 3=Combination 4=Macro).
    /// sum = 0x55 - (count + all entry bytes).
    fn read_shortcut(&self, slot: usize) -> Option<ButtonAction> {
        let b = self.ee_read(ee::SHORTCUT + (slot * 32) as u16, 32).ok()?;
        let n = *b.first()? as usize;
        if n == 0 || n > 10 {
            return None;
        }
        // Use the key of the first "press" entry as the representative display.
        for i in 0..n {
            let e = &b[1 + i * 3..1 + i * 3 + 3];
            let (state, ty) = (e[0], e[0] & 0x3f);
            if state & 0x80 == 0 || ty != 1 {
                continue;
            }
            // keyType=Keyboard: v1 = usage code (the official app uses its own key table). Modifiers are on the v2 side.
            return Some(ButtonAction::Key(KeyStroke {
                mods: e[2],
                usage: e[1] as u16,
            }));
        }
        None
    }

    /// ButtonAction::Key → ShortcutKey area (2 entries: press + release).
    fn encode_shortcut(&self, k: &KeyStroke) -> Vec<u8> {
        let mut b = vec![
            2,
            0x80 | 1,
            (k.usage & 0xff) as u8,
            k.mods,
            0x40 | 1,
            (k.usage & 0xff) as u8,
            k.mods,
        ];
        let s: u8 = b.iter().fold(0u8, |a, &x| a.wrapping_add(x));
        b.push(85u8.wrapping_sub(s));
        b.resize(32, 0);
        b
    }

    /// ButtonAction → KeyFunction 4B record.
    fn encode_keyfn(&self, a: &ButtonAction) -> HubResult<[u8; 4]> {
        use ButtonAction::*;
        let rec = |ty: u8, ph: u8, pl: u8| {
            let mut r = [ty, ph, pl, 0];
            r[3] = Self::crc(&r);
            r
        };
        Ok(match a {
            Disabled => rec(0, 0, 0),
            Mouse(MouseFn::DoubleClick) => rec(4, 100, 2),
            Mouse(MouseFn::ScrollUp) => rec(11, 1, 0),
            Mouse(MouseFn::ScrollDown) => rec(11, 2, 0),
            Mouse(f) => {
                let mask = match f {
                    MouseFn::Left => 1,
                    MouseFn::Right => 2,
                    MouseFn::Middle => 4,
                    MouseFn::Back => 8,
                    MouseFn::Forward => 16,
                    _ => 1,
                };
                rec(1, mask, 0)
            }
            Dpi(DpiFn::Cycle) => rec(2, 1, 0),
            Dpi(DpiFn::Up) => rec(2, 2, 0),
            Dpi(DpiFn::Down) => rec(2, 3, 0),
            Dpi(DpiFn::Shift(d)) => rec(10, 0, ((*d / 50).max(1) - 1) as u8),
            Fire {
                count, interval_ms, ..
            } => rec(4, (*interval_ms).min(255) as u8, *count),
            Macro { slot: s, run } => {
                let t = match run {
                    MacroRun::Times(n) => (*n).min(252) as u8,
                    MacroRun::WhileHeld => 254,
                    MacroRun::Toggle => 253,
                };
                rec(6, *s + 1, t)
            }
            PollingCycle => rec(7, 0, 0),
            // ShortcutKey — the on-device assignment is written to a separate area (@256+slot*32).
            Key(_) => rec(5, 0, 0),
            Media(_) => return Err(HubError::Unsupported("media button")),
            ProfileCycle => return Err(HubError::Unsupported("profile cycle")),
            ScrollModeToggle => return Err(HubError::Unsupported("scroll-mode toggle")),
            Unsupported { .. } => return Err(HubError::Unsupported("raw action")),
        })
    }
}

// ==================================================================== caps

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

fn build_caps() -> Capabilities {
    use section::*;
    Capabilities {
        settings: vec![
            SettingSpec::choice(
                "polling_wireless",
                PERFORMANCE,
                "Polling rate",
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
            .help("Mouse report frequency (Hz)"),
            SettingSpec::choice(
                "lod",
                SENSOR,
                "Lift-off distance",
                opts(&[
                    (7, "0.7mm"),
                    (8, "0.8mm"),
                    (9, "0.9mm"),
                    (10, "1.0mm"),
                    (11, "1.1mm"),
                    (12, "1.2mm"),
                    (13, "1.3mm"),
                    (14, "1.4mm"),
                    (15, "1.5mm"),
                    (16, "1.6mm"),
                    (17, "1.7mm"),
                ]),
            )
            .help("Height at which the mouse stops tracking when lifted — PAW3955 supports 0.7–1.7mm in 0.1mm steps"),
            SettingSpec::toggle("motion_sync", SENSOR, "Motion sync")
                .help("Syncs pointer and mouse movement to smooth the trajectory"),
            SettingSpec::toggle("angle_snap", SENSOR, "Angle snapping")
                .help("Straightens out wobble in movement"),
            SettingSpec::toggle("ripple", SENSOR, "Ripple control")
                .help("Reduces fine jitter at high DPI"),
            SettingSpec::slider("wheel_debounce", CLICK, "Wheel delay", 0., 60., 1., "ms")
                .help("Wheel input delay (device default: 30)"),
            SettingSpec::slider("debounce", CLICK, "Click debounce", 0., 32., 1., "ms")
                .help("Prevents button chatter (@169 stabilizationTime)"),
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
                    (30, "30 min"),
                    (60, "60 min"),
                ]),
            )
            .help("Idle time before the mouse goes to sleep (@173, stored in 10-second units)"),
            SettingSpec::toggle("move_close_led", LIGHTING, "LED off while moving")
                .help("Turns the LED off while you are using the mouse (@179)"),
            SettingSpec::slider("bhop", CLICK, "Wheel misfire prevention", 0., 25., 1., "")
                .help("Suppression time for false scroll detection (0 = off; the official app stores time/10)"),
        ],
        actions: vec![
            ActionSpec::new("factory_reset", SYSTEM, "Restore factory defaults")
                .help("Resets all settings to their defaults (cmd9)")
                .danger("All settings on this device will be erased"),
        ],
        profiles: Some(ProfileCaps {
            count: 4,
            renamable: false,
        }),
        mouse: Some(MouseCaps {
            dpi_min: 50,
            dpi_max: 42000,
            dpi_step: 1,
            min_stages: 1,
            max_stages: 8,
            xy_split: true,
            stage_colors: true,
            stage_toggle: false,
            buttons: [
                "Left",
                "Right",
                "Wheel click",
                "Side (back)",
                "Side (forward)",
            ]
            .iter()
            .map(|n| ButtonSlot { name: (*n).into() })
            .collect(),
            button_kinds: vec![
                ButtonActionKind::Mouse,
                ButtonActionKind::Key,
                ButtonActionKind::Dpi,
                ButtonActionKind::Fire,
                ButtonActionKind::Macro,
                ButtonActionKind::PollingCycle,
                ButtonActionKind::Disabled,
            ],
            require_left_click: true,
            macros: None,
            live_buttons: vec![],
            adaptive_dpi: None,
        }),
        keyboard: None,
        lighting: vec![],
        battery: true,
        peq: None,
        firmware_feed: None,
    }
}

// ==================================================================== DeviceDriver

impl DeviceDriver for AtkDriver {
    fn info(&self) -> DeviceInfo {
        self.maybe_detect();
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner()).clone();
        // Behind a dongle: cmd29 is answered by the dongle itself (it responds even while the mouse is asleep).
        if i.transport == Transport::Wireless24G {
            if let Ok(r) = self.cmd_raw(29, 0, &[]) {
                if r[2] != 1 && (r[6] != 0 || r[7] != 0) {
                    i.dongle_firmware = Some(format!("{}.{}", r[6], r[7]));
                }
            }
        }
        // If cmd18 does not get through, the mouse is asleep.
        match self.cmd_raw(18, 0, &[]) {
            Ok(r) => {
                i.asleep = false;
                if r[6] != 0 || r[7] != 0 {
                    i.firmware = Some(format!("{}.{}", r[6], r[7]));
                }
            }
            Err(_) => {
                i.asleep = i.transport == Transport::Wireless24G;
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
        self.caps.clone()
    }

    fn read(&self) -> HubResult<DeviceState> {
        self.maybe_detect();
        // @0 block: rate, stage count, active.
        let blk = self.ee_read(0, 10)?;
        let rate_code = blk[0];
        let stage_n = (blk[2] as usize).clamp(1, 8);
        let active = (blk[4] as usize).min(stage_n - 1);
        let stages_raw = self.ee_read(ee::DPI_3955, (stage_n * 6) as u16)?;
        let colors = self.ee_read(ee::DPI_COLOR, (stage_n * 4) as u16)?;
        let mut stages = vec![];
        for i in 0..stage_n {
            let rec = &stages_raw[i * 6..i * 6 + 6];
            let (x, y) = Self::parse_stage(rec);
            let c = &colors[i * 4..i * 4 + 4];
            stages.push(DpiStage {
                x,
                y: if y == x { None } else { Some(y) },
                color: Rgb {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                },
                enabled: true,
            });
        }
        // Buttons key0..4 @96.
        let keys = self.ee_read(ee::KEY_FUNCTION, 20)?;
        let buttons = (0..5)
            .map(|i| self.decode_keyfn(i, &keys[i * 4..i * 4 + 4]))
            .collect();

        let mut settings = BTreeMap::new();
        let rate = match rate_code {
            8 => 125,
            4 => 250,
            2 => 500,
            1 => 1000,
            16 => 2000,
            32 => 4000,
            64 => 8000,
            _ => 1000,
        };
        settings.insert("polling_wireless".into(), SettingValue::Choice(rate));
        if let Ok(v) = self.ee_u8(ee::LOD) {
            // @10 = LOD level 1–11 → 0.7–1.7mm (PAW3955 "Ultimate" family).
            let mm10 = (v.clamp(1, 11) + 6) as i64;
            settings.insert("lod".into(), SettingValue::Choice(mm10));
        }
        for (addr, id) in [
            (ee::MOTION_SYNC, "motion_sync"),
            (ee::ANGLE_SNAP, "angle_snap"),
            (ee::RIPPLE, "ripple"),
        ] {
            if let Ok(v) = self.ee_u8(addr) {
                settings.insert(id.into(), SettingValue::Bool(v != 0));
            }
        }
        if let Ok(v) = self.ee_u8(ee::WHEEL_DEBOUNCE) {
            settings.insert("wheel_debounce".into(), SettingValue::Num(v as f32));
        }
        if let Ok(v) = self.ee_u8(ee::DEBOUNCE) {
            settings.insert("debounce".into(), SettingValue::Num(v as f32));
        }
        if let Ok(v) = self.ee_u8(ee::CLOSE_LED) {
            // wire = 10-second units → convert to minutes (6 → 1 min).
            let min = ((v as u32 * 10) / 60).max(1) as i64;
            settings.insert("sleep_min".into(), SettingValue::Choice(min));
        }
        if let Ok(v) = self.ee_u8(ee::MOVE_LED) {
            settings.insert("move_close_led".into(), SettingValue::Bool(v != 0));
        }
        // @0 byte6 = bhop (wheel misfire prevention, time/10).
        settings.insert("bhop".into(), SettingValue::Num(blk[6] as f32));
        // Note: the cmd22/23 long-range mode is a dongle-side feature — on wired, cmd22 ACKs but
        // cmd23 always returns 0 (confirmed on a real device), so it is not exposed as a setting.
        // Onboard profile: cmd14 GetCurrentConfig → data[0] = index.
        let profile = self
            .cmd_raw(14, 0, &[])
            .ok()
            .and_then(|r| r.get(6).copied())
            .unwrap_or(0);
        Ok(DeviceState {
            profile: ProfileState {
                active: profile.min(3),
                names: vec![],
            },
            settings,
            mouse: Some(MouseState {
                stages,
                active_stage: active,
                buttons,
                macros: vec![],
                adaptive: None,
            }),
            keyboard: None,
            lighting: BTreeMap::new(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        match change {
            Change::Setting { id, value } => match id.as_str() {
                "polling_wireless" => {
                    let code = match value.as_choice() {
                        125 => 8,
                        250 => 4,
                        500 => 2,
                        1000 => 1,
                        2000 => 16,
                        4000 => 32,
                        8000 => 64,
                        _ => return Err(HubError::Invalid("polling rate".into())),
                    };
                    self.cfg_byte(0, code)
                }
                "lod" => self.ee_write_val(ee::LOD, (value.as_choice().clamp(7, 17) - 6) as u8),
                "motion_sync" => self.ee_write_val(ee::MOTION_SYNC, value.as_bool() as u8),
                "angle_snap" => self.ee_write_val(ee::ANGLE_SNAP, value.as_bool() as u8),
                "ripple" => self.ee_write_val(ee::RIPPLE, value.as_bool() as u8),
                "wheel_debounce" => {
                    self.ee_write_val(ee::WHEEL_DEBOUNCE, value.as_num().clamp(0., 255.) as u8)
                }
                "debounce" => self.ee_write_val(ee::DEBOUNCE, value.as_num().clamp(0., 255.) as u8),
                "sleep_min" => {
                    // Minutes → 10-second units (1 min → 6).
                    self.ee_write_val(ee::CLOSE_LED, (value.as_choice().clamp(1, 42) * 6) as u8)
                }
                "move_close_led" => self.ee_write_val(ee::MOVE_LED, value.as_bool() as u8),
                "bhop" => self.cfg_byte(6, value.as_num().clamp(0., 25.) as u8),
                _ => Err(HubError::Unsupported("setting")),
            },
            Change::DpiStages { stages, active } => {
                let n = stages.len().clamp(1, 8);
                self.ee_write_val(ee::MAX_DPI_STAGE, n as u8)?;
                self.ee_write_val(ee::CURRENT_DPI, (*active).min(n - 1) as u8)?;
                for (i, s) in stages[..n].iter().enumerate() {
                    self.ee_write(
                        ee::DPI_3955 + (i * 6) as u16,
                        &Self::encode_stage(s.x, s.y.unwrap_or(s.x)),
                    )?;
                    let mut c = vec![s.color.r, s.color.g, s.color.b, 0];
                    c[3] = Self::crc(&c);
                    self.ee_write(ee::DPI_COLOR + (i * 4) as u16, &c)?;
                }
                Ok(())
            }
            Change::ActiveStage(a) => self.ee_write_val(ee::CURRENT_DPI, *a as u8),
            Change::Button { slot, action } => {
                if *slot >= 5 {
                    return Err(HubError::Invalid("button slot".into()));
                }
                let rec = self.encode_keyfn(action)?;
                // For ShortcutKey, write the on-device area first, then point keyfn at it.
                if let ButtonAction::Key(k) = action {
                    self.ee_write(ee::SHORTCUT + (*slot * 32) as u16, &self.encode_shortcut(k))?;
                }
                self.ee_write(ee::KEY_FUNCTION + (*slot * 4) as u16, &rec)
            }
            Change::Profile(i) => self.cmd_raw(15, 0, &[*i]).map(|_| ()),
            _ => Err(HubError::Unsupported("change")),
        }
    }

    fn run_action(&self, id: &str, _file: Option<&std::path::Path>) -> HubResult<ActionOutcome> {
        match id {
            "factory_reset" => {
                self.cmd_raw(9, 0, &[])?;
                Ok(ActionOutcome {
                    message: Some("Restored factory defaults".into()),
                    reload: true,
                })
            }
            _ => Err(HubError::Unsupported("action")),
        }
    }

    fn battery(&self) -> HubResult<Battery> {
        // cmd4: data[0]=percent, data[1]=chargingStatus.
        let r = self.cmd_raw(4, 0, &[])?;
        Ok(Battery {
            percent: Some(r.get(6).copied().unwrap_or(0).min(100)),
            charging: r.get(7).copied().unwrap_or(0) != 0,
        })
    }
}
