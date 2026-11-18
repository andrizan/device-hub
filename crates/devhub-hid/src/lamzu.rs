//! LAMZU Aurora (www.lamzu.net) family mice — 64B feature report protocol.
//!
//! Based on analysis of the official Web HUB JS + verified on a real wired Maya X V2 / LM20 (37b0:0044)
//! (docs/PROTOCOL_NOTES.md §5).
//!
//! - Settings channel: if2 `FFFF:00`, 64B feature reports only (no output report).
//! - Send `[0,0,2,sub,f,cmd,devID,args…]` (devID = profile number 1..4,
//!   lighting commands and some commands without devID use a different layout).
//! - Response: `a[1]`=status (A1=ACK), `a[4]`=payload length, `a[6]`=cmd echo,
//!   `a[7]`=devID, `a[8..]`=value. Writes apply immediately and persist to flash.
//! - Autonomous notifications `[4,type,…]` arrive as input reports on if1 `FFA0:01`.
//! - Uninitialized profile slots are not scanned (p4 may cause an HID error).

use devhub_core::backend::*;
use devhub_core::lamzu::{caps as lamzu_caps, model_by_pid, LamzuModel, MODELS};
use devhub_core::model::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

/// Wait for a feature report round trip. Wired needs only ~10-30ms, but via the dongle relay
/// the response buffer update can lag by ~200ms.
const CMD_DELAY: Duration = Duration::from_millis(120);

// ---------------------------------------------------------------- commands

/// Feature report round trip: sends `[0,0,b2,sub,f,cmd,args…]` and returns the response.
/// The response is 65B including the report ID. `None` means a non-a1 response or a failure.
fn cmd(
    dev: &hidapi::HidDevice,
    b2: u8,
    sub: u8,
    f: u8,
    command: u8,
    args: &[u8],
) -> HubResult<Vec<u8>> {
    // Do not retry reset-type commands (cmd 13 = resetProfile) — avoids running them twice.
    cmd_ex(dev, b2, sub, f, command, args, command != 13)
}

/// Sends without waiting for a response (same as the official `defaultDevice`; a full reset returns no response).
fn send_only(dev: &hidapi::HidDevice, b2: u8, sub: u8, f: u8, command: u8, args: &[u8]) -> HubResult<()> {
    let mut w = [0u8; 65];
    w[3] = b2;
    w[4] = sub;
    w[5] = f;
    w[6] = command;
    let n = args.len().min(56);
    w[7..7 + n].copy_from_slice(&args[..n]);
    dev.send_feature_report(&w)
        .map(|_| ())
        .map_err(|e| HubError::Io(format!("lamzu feature write: {e}")))
}

fn cmd_ex(
    dev: &hidapi::HidDevice,
    b2: u8,
    sub: u8,
    f: u8,
    command: u8,
    args: &[u8],
    resend: bool,
) -> HubResult<Vec<u8>> {
    let mut w = [0u8; 65];
    w[3] = b2;
    w[4] = sub;
    w[5] = f;
    w[6] = command;
    let n = args.len().min(56);
    w[7..7 + n].copy_from_slice(&args[..n]);
    // Send only once — response generation is delayed behind the dongle relay, so repeat get
    // until a fresh response with a[6]==cmd arrives.
    // (Resending resets the device-side processing timer and delays things further.)
    dev.send_feature_report(&w)
        .map_err(|e| HubError::Io(format!("lamzu feature write: {e}")))?;
    let mut last = HubError::Protocol("lamzu: response timeout".into());
    // Some writes through the dongle (polling changes, etc.) need an RF renegotiation, so
    // response generation can take a few seconds — repeat get with plenty of margin.
    // If the send itself is dropped, no response is regenerated and the buffer stays stale
    // and stuck, so resend once midway, as the official retrySetGet does.
    for i in 0..25 {
        // The next command can be dropped while a flash write is being committed —
        // resend up to 2 times (equivalent to retrySetGet).
        if resend && (i == 8 || i == 16) {
            dev.send_feature_report(&w).ok();
        }
        sleep(CMD_DELAY);
        let mut r = [0u8; 65];
        match dev.get_feature_report(&mut r) {
            Ok(len) if len >= 8 => {
                // Response layout (fixtures/lamzu/lm20-wired-active.txt):
                // r[3]=target echo, r[4]=declared payload length, r[5]=f echo,
                // r[6]=cmd echo, r[7..7+r[4]]=payload.
                // cmd alone collides with leftover buffers that differ only in f (button get f3 and
                // polling get f1 both use cmd=0x80), so match on everything.
                if r[6] == command && r[5] == f && r[3] == b2 {
                    // GET commands (cmd>=0x80) echo args[0]
                    // (dev/profile) at the start of the payload. Button get also echoes the slot number
                    // args[1] in r[8] — an identifier so that sequential slot reads do not
                    // mistakenly accept a neighbor's stale response.
                    let arg_ok = command < 0x80 || args.first().is_none_or(|a| r[7] == *a);
                    let btn_ok = !(f == 3 && command == 0x80)
                        || args.get(1).is_none_or(|a| r[8] == *a);
                    let len_ok = (r[4] as usize) <= len.saturating_sub(7);
                    if arg_ok && btn_ok && len_ok {
                        return if r[1] == 0xA1 {
                            Ok(r[..len].to_vec())
                        } else {
                            // A valid response to this command, but not an ACK (a2/a3).
                            Err(HubError::Protocol(format!(
                                "lamzu: cmd {command:#04x} -> tag {:02x}",
                                r[1]
                            )))
                        };
                    }
                    last = HubError::Protocol(format!(
                        "lamzu: cmd {command:#04x} -> mismatched payload f{:02x}/len{}/arg{:02x}",
                        r[5], r[4], r[7]
                    ));
                    continue;
                }
                last = HubError::Protocol(format!(
                    "lamzu: cmd {command:#04x} -> stale f{:02x} echo {:02x}",
                    r[5], r[6]
                ));
            }
            Ok(_) => {}
            Err(e) => last = HubError::Io(format!("lamzu feature read: {e}")),
        }
    }
    Err(last)
}

/// The value `a[8]` of a profile-targeted command (devID=profile 1..4 placed at the start of args).
fn u8_at(resp: &[u8], i: usize) -> HubResult<u8> {
    resp.get(i)
        .copied()
        .ok_or_else(|| HubError::Protocol("lamzu: short payload".into()))
}

// ---------------------------------------------------------------- driver

pub struct LamzuDriver {
    dev: Mutex<hidapi::HidDevice>,
    info: Mutex<DeviceInfo>,
    /// Determined by the model (paired mouse) and sensor. Rebuilt if the paired mouse becomes known later via the dongle.
    caps: Mutex<Capabilities>,
    /// A row of the official model definitions. None = not connected / unknown paired mouse.
    model: Mutex<Option<&'static LamzuModel>>,
    /// Response to the sensor model command (4 = PAW3955).
    sensor: Mutex<u8>,
    stop: Arc<AtomicBool>,
}

pub fn open(
    dev: hidapi::HidDevice,
    info: DeviceInfo,
    events: Option<Arc<dyn Fn(HubEvent) + Send + Sync>>,
) -> HubResult<LamzuDriver> {
    dev.set_blocking_mode(false).ok();
    let stop = Arc::new(AtomicBool::new(false));
    // Notification channel if1 FFA0:01 — opened by us to read input reports.
    if let Some(ev) = events {
        spawn_notify(info.clone(), ev, stop.clone());
    }
    let d = LamzuDriver {
        dev: Mutex::new(dev),
        info: Mutex::new(info.clone()),
        caps: Mutex::new(lamzu_caps(&MODELS[0], info.transport, 4)),
        model: Mutex::new(None),
        sensor: Mutex::new(4),
        stop,
    };
    d.detect();
        Ok(d)
}

impl Drop for LamzuDriver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Reads input reports from `if1 FFA0:01` and converts `[4,type,…]` into HubEvent.
fn spawn_notify(info: DeviceInfo, ev: Arc<dyn Fn(HubEvent) + Send + Sync>, stop: Arc<AtomicBool>) {
    let (vid, pid) = (info.vid, info.pid);
    let serial = info.serial.clone();
    std::thread::spawn(move || {
        let Ok(api) = hidapi::HidApi::new() else {
            return;
        };
        // With several receivers sharing the same VID/PID, first-match binds to the wrong unit —
        // require a match when the serial is available.
        let Some(d) = api
            .device_list()
            .find(|d| {
                d.vendor_id() == vid
                    && d.product_id() == pid
                    && d.interface_number() == 1
                    && d.usage_page() == 0xFFA0
                    && d.usage() == 0x01
                    && serial
                        .as_deref()
                        .is_none_or(|s| s.is_empty() || d.serial_number() == Some(s))
            })
            .and_then(|i| i.open_device(&api).ok())
        else {
            return;
        };
        d.set_blocking_mode(false).ok();
        let mut buf = [0u8; 64];
        while !stop.load(Ordering::Relaxed) {
            match d.read_timeout(&mut buf, 250) {
                Ok(n) if n >= 3 && buf[0] == 4 => match buf[1] {
                    // type3 = battery (b[2]=%, b[3]=charging)
                    3 => {
                        let id = info.id.clone();
                        ev(HubEvent::Battery(
                            id,
                            Battery {
                                percent: Some(buf[2].min(100)),
                                charging: buf[3] == 1,
                            },
                        ));
                    }
                    // type1=DPI, 2=profile, 6=connect/disconnect, 7=LOD,
                    // 8=polling, 13=mode — have the UI re-read.
                    1 | 2 | 6 | 7 | 8 | 13 => {
                        ev(HubEvent::StateChanged(info.id.clone()));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    });
}

impl LamzuDriver {
    /// Determines the model (wired = own PID, dongle = paired mouse from GetDevicePID) and the sensor,
    /// and fixes the product name, model and caps. An unknown paired mouse gets `supported=false`.
    fn detect(&self) {
        let (transport, own_pid) = {
            let i = self.info.lock().unwrap_or_else(|p| p.into_inner());
            (i.transport, i.pid)
        };
        let pid = if transport == Transport::Wireless24G {
            match self.run(1, 6, 0, 139, &[2]) {
                Ok(r) if r.len() >= 13 => {
                    let vid = u16::from_be_bytes([r[9], r[10]]);
                    let pid = u16::from_be_bytes([r[11], r[12]]);
                    (vid == 0x37B0).then_some(pid)
                }
                _ => {
                    // The mouse is asleep / not connected — keep the receiver name.
                    self.info.lock().unwrap_or_else(|p| p.into_inner()).asleep = true;
                    return;
                }
            }
        } else {
            Some(own_pid)
        };
        let model = pid.and_then(model_by_pid);
        // Sensor model [2,1,1,143] → a[7] (1=3395, 2=3950, 4=3955).
        let sensor = self
            .run(2, 1, 1, 143, &[])
            .ok()
            .and_then(|r| r.get(7).copied())
            .unwrap_or(4);
        *self.sensor.lock().unwrap_or_else(|p| p.into_inner()) = sensor;
        *self.model.lock().unwrap_or_else(|p| p.into_inner()) = model;
        let m = model.unwrap_or(&MODELS[0]);
        *self.caps.lock().unwrap_or_else(|p| p.into_inner()) = lamzu_caps(m, transport, sensor);
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner());
        i.asleep = false;
        i.supported = model.is_some();
        if let Some(m) = model {
            i.name = m.name.into();
            i.model = m.model.into();
        } else if transport == Transport::Wireless24G {
            i.name = "LAMZU 8K receiver (unsupported mouse)".into();
            i.model = "lamzu-dongle".into();
        }
    }

    fn caps_now(&self) -> Capabilities {
        self.caps.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn has(&self, id: &str) -> bool {
        self.caps.lock().unwrap_or_else(|p| p.into_inner()).setting(id).is_some()
    }

    fn run(&self, b2: u8, sub: u8, f: u8, command: u8, args: &[u8]) -> HubResult<Vec<u8>> {
        let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
        cmd(&dev, b2, sub, f, command, args)
    }

    /// get: `[2,sub,f,cmd|0x80,dev]` → value in `a[8]`.
    fn get(&self, sub: u8, f: u8, command: u8, dev: u8) -> HubResult<u8> {
        let r = self.run(2, sub, f, command, &[dev])?;
        u8_at(&r, 8)
    }

    /// set: `[2,sub,f,cmd,dev,val]`.
    /// The FW generates no response for a same-value write (the response buffer stays at the previous command
    /// and gets stuck), so read the current value first and skip the send if it is the same.
    /// If it fails because of response delay through the dongle, verify by reading back with get (cmd|0x80).
    fn set(&self, sub: u8, f: u8, command: u8, dev: u8, val: u8) -> HubResult<()> {
        if self.get(sub, f, command | 0x80, dev).ok() == Some(val) {
            return Ok(());
        }
        match self.run(2, sub, f, command, &[dev, val]) {
            Ok(_) => Ok(()),
            Err(e) => {
                let back = self.get(sub, f, command | 0x80, dev);
                if back.as_ref().ok() == Some(&val) {
                    return Ok(());
                }
                Err(HubError::Protocol(format!("{e} [verify {back:?}]")))
            }
        }
    }

    /// Whether it goes through the dongle (37b0:0048 Orbital 8K).
    fn is_dongle(&self) -> bool {
        self.info
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .transport
            == Transport::Wireless24G
    }

    /// The current profile (1..4). Err when unknown —
    /// silently falling back to 1 could overwrite profile 1 while another profile is being edited.
    fn active_profile(&self) -> HubResult<u8> {
        let r = self.run(2, 1, 0, 133, &[])?;
        let p = u8_at(&r, 7)?;
        if (1..=4).contains(&p) {
            Ok(p)
        } else {
            Err(HubError::Protocol(format!(
                "lamzu: bad active profile {p:#04x}"
            )))
        }
    }

    /// Reads one button. An empty entry returns an echo, so it is Disabled.
    fn read_button(&self, profile: u8, btn: u8) -> ButtonAction {
        let Ok(r) = self.run(2, 15, 3, 128, &[profile, btn, 0, 255, 10]) else {
            return ButtonAction::Disabled;
        };
        if r.len() < 12 {
            return ButtonAction::Disabled;
        }
        let (ty, len) = (
            u8_at(&r, 10).unwrap_or(0),
            u8_at(&r, 11).unwrap_or(0) as usize,
        );
        if ty == 0xFF || len == 0x0A {
            // Request echo (uninitialized entry).
            return ButtonAction::Disabled;
        }
        let p = &r[12..(12 + len).min(r.len())];
        decode_button(ty, p)
    }

    fn write_button(&self, profile: u8, btn: u8, a: &ButtonAction) -> HubResult<()> {
        let (ty, params) = encode_button(a)?;
        let mut args = vec![profile, btn, 0, ty, params.len() as u8];
        args.extend_from_slice(&params);
        self.run(2, (5 + params.len()) as u8, 3, 0, &args)
            .map(|_| ())
    }

    /// Reads stage values + colors. count=0, out-of-range and short responses are Err
    /// (abnormal values are not rounded into valid data).
    /// Speed-linked DPI curve (official getAdaptiveDPICurve).
    fn read_adaptive(&self, dev: u8) -> HubResult<AdaptiveCurve> {
        let r = self.run(2, 9, 1, 154, &[dev])?;
        let enabled = u8_at(&r, 8)? == 1;
        let n = u8_at(&r, 12)? as usize;
        if n > 13 || r.len() < 13 + n * 4 {
            return Err(HubError::Protocol(format!("lamzu: bad adaptive curve n={n}")));
        }
        let points = (0..n)
            .map(|i| {
                let b = &r[13 + i * 4..13 + i * 4 + 4];
                AdaptivePoint {
                    speed: u16::from_be_bytes([b[0], b[1]]),
                    ratio: u16::from_be_bytes([b[2], b[3]]) as f32 / 100.,
                }
            })
            .collect();
        Ok(AdaptiveCurve { enabled, points })
    }

    /// Writes the speed-linked DPI curve (official setAdaptiveDPICurve:
    /// `[2,4+4N,1,26,prof,en,0,N,(speed u16BE, multiplier×100 u16BE)×N]`). Input is validated first.
    fn write_adaptive(&self, prof: u8, c: &AdaptiveCurve) -> HubResult<()> {
        let Some(ac) = self.caps_now().mouse.and_then(|m| m.adaptive_dpi) else {
            return Err(HubError::Unsupported("adaptive dpi"));
        };
        let n = c.points.len();
        if n < ac.min_points || n > ac.max_points {
            return Err(HubError::Invalid("speed-linked DPI point count".into()));
        }
        let mut last = None;
        for p in &c.points {
            if p.speed > ac.speed_max
                || !(0.0..=ac.ratio_max + 1e-3).contains(&p.ratio)
                || last.is_some_and(|l| p.speed <= l)
            {
                return Err(HubError::Invalid("speed-linked DPI point value".into()));
            }
            last = Some(p.speed);
        }
        let mut args = vec![prof, c.enabled as u8, 0, n as u8];
        for p in &c.points {
            args.extend_from_slice(&p.speed.to_be_bytes());
            args.extend_from_slice(&((p.ratio * 100.).round() as u16).to_be_bytes());
        }
        match self.run(2, (4 + n * 4) as u8, 1, 26, &args) {
            Ok(_) => Ok(()),
            // When no response arrives (e.g. a same-value write), decide by reading back.
            Err(e) => match self.read_adaptive(prof) {
                Ok(back) if back.enabled == c.enabled && back.points.len() == n => Ok(()),
                _ => Err(e),
            },
        }
    }

    fn read_stages(&self, dev: u8) -> HubResult<Vec<DpiStage>> {
        let r = self.run(2, 10, 1, 129, &[dev, 6])?;
        let n = u8_at(&r, 8)? as usize;
        if !(1..=5).contains(&n) || r.len() < 9 + n * 4 {
            return Err(HubError::Protocol(format!("lamzu: bad dpi table n={n}")));
        }
        let colors = self.run(2, 19, 2, 129, &[dev]).ok();
        Ok((0..n)
            .map(|i| {
                let b = &r[9 + i * 4..9 + i * 4 + 4];
                let (x, y) = (
                    u16::from_be_bytes([b[0], b[1]]) as u32,
                    u16::from_be_bytes([b[2], b[3]]) as u32,
                );
                let (cr, cg, cb) = colors
                    .as_ref()
                    .and_then(|c| c.get(8 + i * 3..8 + i * 3 + 3).map(|s| (s[0], s[1], s[2])))
                    .unwrap_or((255, 255, 255));
                DpiStage {
                    x,
                    y: if y == x { None } else { Some(y) },
                    color: Rgb {
                        r: cr,
                        g: cg,
                        b: cb,
                    },
                    enabled: true,
                }
            })
            .collect())
    }

    /// Writes stage values + colors (does not change the active stage).
    /// Value → color → active stage are separate commands in that order, so validate the input first.
    fn write_stages(&self, prof: u8, stages: &[DpiStage]) -> HubResult<()> {
        if stages.is_empty() || stages.len() > 5 {
            return Err(HubError::Invalid("dpi stage count".into()));
        }
        for s in stages {
            let ok = (50..=50000).contains(&s.x)
                && s.y.is_none_or(|y| (50..=50000).contains(&y));
            if !ok {
                return Err(HubError::Invalid("dpi value".into()));
            }
        }
        let mut args = vec![prof, stages.len() as u8];
        for s in stages {
            args.extend_from_slice(&(s.x as u16).to_be_bytes());
            args.extend_from_slice(&(s.y.unwrap_or(s.x) as u16).to_be_bytes());
        }
        self.run(2, 26, 1, 1, &args)?;
        // Color: [2,19,2,1,dev,RGB×6] — padded to 6 slots.
        let mut cargs = vec![prof];
        for s in stages {
            cargs.extend_from_slice(&[s.color.r, s.color.g, s.color.b]);
        }
        cargs.resize(1 + 18, 0);
        self.run(2, 19, 2, 1, &cargs)?;
        // Wait for the flash commit — prevents the next command from being dropped.
        sleep(Duration::from_millis(300));
        Ok(())
    }
}

// ---------------------------------------------------------------- buttons

fn decode_button(ty: u8, p: &[u8]) -> ButtonAction {
    match ty {
        1 => ButtonAction::Mouse(match p.first().copied().unwrap_or(0) {
            1 => MouseFn::Left,
            2 => MouseFn::Right,
            3 => MouseFn::Middle,
            4 => MouseFn::Back,
            5 => MouseFn::Forward,
            16 => MouseFn::ScrollUp,
            17 => MouseFn::ScrollDown,
            // Old DPI code of the due enum (the official jM uses type7, but earlier
            // writes may have left it, so only reads support it).
            18 => return ButtonAction::Dpi(DpiFn::Up),
            19 => return ButtonAction::Dpi(DpiFn::Down),
            20 | 21 => return ButtonAction::Dpi(DpiFn::Cycle),
            _ => {
                return ButtonAction::Unsupported {
                    ty,
                    params: p.to_vec(),
                }
            }
        }),
        2 if p.len() >= 5 => ButtonAction::Fire {
            button: match p[1] {
                2 => MouseFn::Right,
                3 => MouseFn::Middle,
                4 => MouseFn::Back,
                5 => MouseFn::Forward,
                _ => MouseFn::Left,
            },
            count: p[2],
            interval_ms: u16::from_be_bytes([p[3], p[4]]),
        },
        4 if p.len() >= 2 => ButtonAction::Key(KeyStroke {
            mods: p[0],
            usage: p[1] as u16,
        }),
        5 if p.len() >= 2 => {
            let usage = u16::from_be_bytes([p[0], p[1]]);
            ButtonAction::Media(match usage {
                205 => MediaFn::PlayPause,
                183 => MediaFn::Stop,
                181 => MediaFn::Next,
                182 => MediaFn::Prev,
                233 => MediaFn::VolumeUp,
                234 => MediaFn::VolumeDown,
                226 => MediaFn::Mute,
                402 => MediaFn::Calculator,
                404 => MediaFn::MyComputer,
                406 => MediaFn::Browser,
                394 => MediaFn::Mail,
                111 => MediaFn::BrightnessUp,
                112 => MediaFn::BrightnessDown,
                551 => MediaFn::Refresh,
                _ => {
                    return ButtonAction::Unsupported {
                        ty,
                        params: p.to_vec(),
                    }
                }
            })
        }
        // DPI actions use type7 + the Er enum, same as the official jM()
        // (CycleUp=6/CycleDown=7/Up=1/Down=2/Lock=5+[Xhi,Xlo,Xhi,Xlo]).
        7 => ButtonAction::Dpi(match p.first().copied().unwrap_or(0) {
            1 => DpiFn::Up,
            2 => DpiFn::Down,
            5 if p.len() >= 5 => DpiFn::Shift(u32::from(p[1]) << 8 | u32::from(p[2])),
            6 | 7 => DpiFn::Cycle,
            _ => {
                return ButtonAction::Unsupported {
                    ty,
                    params: p.to_vec(),
                }
            }
        }),
        8 => ButtonAction::ProfileCycle,
        13 => ButtonAction::PollingCycle,
        // Official MacroI/II/III: type16=count [slotHi,slotLo,count],
        // 17=until release, 18=until pressed again (params are the slot only).
        16 if p.len() >= 3 => ButtonAction::Macro {
            slot: u16::from_be_bytes([p[0], p[1]]) as u8,
            run: MacroRun::Times(p[2] as u16),
        },
        17 if p.len() >= 2 => ButtonAction::Macro {
            slot: u16::from_be_bytes([p[0], p[1]]) as u8,
            run: MacroRun::WhileHeld,
        },
        18 if p.len() >= 2 => ButtonAction::Macro {
            slot: u16::from_be_bytes([p[0], p[1]]) as u8,
            run: MacroRun::Toggle,
        },
        // A type we cannot interpret (14=LOD cycle, 20=competitive mode toggle, etc.) is
        // kept as Unsupported instead of collapsing to Disabled — it survives a write-back round trip.
        _ => ButtonAction::Unsupported {
            ty,
            params: p.to_vec(),
        },
    }
}

fn encode_button(a: &ButtonAction) -> HubResult<(u8, Vec<u8>)> {
    use ButtonAction::*;
    Ok(match a {
        Disabled => (0, vec![]),
        Mouse(f) => {
            let c = match f {
                MouseFn::Left => 1,
                MouseFn::Right => 2,
                MouseFn::Middle => 3,
                MouseFn::Back => 4,
                MouseFn::Forward => 5,
                MouseFn::ScrollUp => 16,
                MouseFn::ScrollDown => 17,
                _ => return Err(HubError::Unsupported("mouse fn")),
            };
            (1, vec![c])
        }
        // Official jM(): DPI is type7 + the Er enum, lock is [5,Xhi,Xlo,Xhi,Xlo].
        Dpi(DpiFn::Cycle) => (7, vec![6]),
        Dpi(DpiFn::Up) => (7, vec![1]),
        Dpi(DpiFn::Down) => (7, vec![2]),
        Dpi(DpiFn::Shift(d)) => (
            7,
            vec![5, (*d >> 8) as u8, *d as u8, (*d >> 8) as u8, *d as u8],
        ),
        Fire {
            button,
            count,
            interval_ms,
        } => {
            // The official rapid fire (X-click) is fixed to left click. Other buttons are unverified on a real device, so they are not written.
            if *button != MouseFn::Left {
                return Err(HubError::Unsupported("rapid fire is left click only (same as the official app)"));
            }
            let b = 1;
            let [hi, lo] = interval_ms.to_be_bytes();
            (2, vec![1, b, *count, hi, lo])
        }
        Key(k) => (4, vec![k.mods, (k.usage & 0xff) as u8]),
        Media(m) => {
            let usage: u16 = match m {
                MediaFn::PlayPause => 205,
                MediaFn::Stop => 183,
                MediaFn::Next => 181,
                MediaFn::Prev => 182,
                MediaFn::VolumeUp => 233,
                MediaFn::VolumeDown => 234,
                MediaFn::Mute => 226,
                MediaFn::Calculator => 402,
                MediaFn::MyComputer => 404,
                MediaFn::Browser => 406,
                MediaFn::Mail => 394,
                MediaFn::BrightnessUp => 111,
                MediaFn::BrightnessDown => 112,
                MediaFn::Refresh => 551,
            };
            (5, usage.to_be_bytes().to_vec())
        }
        // Official MacroI/II/III: count=type16 [slot,count], until release=17,
        // until pressed again=18 (params are the slot only, no count field).
        Macro { slot, run } => {
            let [hi, lo] = (*slot as u16).to_be_bytes();
            match run {
                MacroRun::Times(n) => (16, vec![hi, lo, (*n).min(255) as u8]),
                MacroRun::WhileHeld => (17, vec![hi, lo]),
                MacroRun::Toggle => (18, vec![hi, lo]),
            }
        }
        // The official app sends the cycle parameter LoopUp=3.
        PollingCycle => (13, vec![3]),
        ProfileCycle => (8, vec![3]),
        ScrollModeToggle => return Err(HubError::Unsupported("scroll-mode toggle")),
        // Device-side assignments we cannot interpret can be written back unchanged in a round trip.
        Unsupported { ty, params } => (*ty, params.clone()),
    })
}

// ---------------------------------------------------------------- caps

// ---------------------------------------------------------------- DeviceDriver

impl DeviceDriver for LamzuDriver {
    fn info(&self) -> DeviceInfo {
        // If the mouse was asleep / undetermined when opened via the dongle, determine it again here.
        if self.model.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
            self.detect();
        }
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner()).clone();
        // Mouse FW: [2,16,0,129] → a[7..11] = 4B version (audit: the "4th byte fixed at 0" assumption was wrong).
        match self.run(2, 16, 0, 129, &[]) {
            Ok(r) => {
                i.asleep = false;
                if let Some(v) = r.get(7..11) {
                    i.firmware = Some(format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]));
                }
            }
            // Mouse not responding behind the dongle = asleep / not connected.
            Err(_) if i.transport == Transport::Wireless24G => i.asleep = true,
            Err(_) => {}
        }
        // Receiver FW: [0,16,0,129] (target 0).
        if i.transport == Transport::Wireless24G {
            if let Ok(r) = self.run(0, 16, 0, 129, &[]) {
                if let Some(v) = r.get(7..11) {
                    i.dongle_firmware = Some(format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]));
                }
            }
        }
        if let Ok(b) = self.battery() {
            i.battery = Some(b);
        }
        // Serial: EID response a[9..] ASCII
        if let Ok(r) = self.run(2, 1, 0, 130, &[]) {
            if let Some(tail) = r.get(9..) {
                if let Some(end) = tail.iter().position(|&b| b == 0 || b == 0xFF) {
                    if end > 0 {
                        i.serial = Some(String::from_utf8_lossy(&tail[..end]).into_owned());
                    }
                }
            }
        }
        // supported stays as decided by detect() from the model table (false for an unknown paired mouse).
        i
    }

    fn caps(&self) -> Capabilities {
        self.caps_now()
    }

    fn read(&self) -> HubResult<DeviceState> {
        // Continuing with an unknown profile would leave it unclear which
        // profile the values read belong to and would also misdirect writes — report the failure.
        let prof = self.active_profile()?;
        let dev = prof;
        let mut settings = BTreeMap::new();

        // Polling: 8,4,2,1,32,64,128 → Hz.
        // Leave it out if it cannot be read (filling in a default would wrongly show 1000Hz).
        if let Ok(code) = self.get(2, 1, 128, dev) {
            let rate = match code {
                8 => 125,
                4 => 250,
                2 => 500,
                // The official getPollingRate treats 16 as 1 (1000Hz).
                1 | 16 => 1000,
                32 => 2000,
                64 => 4000,
                128 => 8000,
                v => v as i64,
            };
            settings.insert("polling_wireless".into(), SettingValue::Choice(rate));
        }

        // LOD: >=128 is (v-128)/10 mm, BCD 0x1x is 1.xmm, 1=1mm, 2=2mm.
        if let Ok(v) = self.get(2, 1, 136, dev) {
            let mm10 = if v >= 128 {
                (v - 128) as i64
            } else if (0x10..=0x19).contains(&v) {
                (v >> 4) as i64 * 10 + (v & 0x0f) as i64
            } else if v == 3 {
                7
            } else {
                v as i64 * 10
            };
            settings.insert("lod".into(), SettingValue::Choice(mm10.clamp(1, 30)));
        }

        for (sub, f, cmdg, id) in [
            (2u8, 1u8, 137u8, "motion_sync"),
            (2, 1, 132, "angle_snap"),
            (2, 1, 138, "ripple"),
            (2, 1, 139, "hyper_mode"),
            (2, 1, 147, "tracking_mode"),
            (2, 1, 141, "dpi_xy"),
        ] {
            if let Ok(v) = self.get(sub, f, cmdg, dev) {
                settings.insert(id.into(), SettingValue::Bool(v == 1));
            }
        }
        if let Ok(v) = self.get(2, 1, 148, dev) {
            let signed = if v > 100 { v as i64 - 256 } else { v as i64 };
            settings.insert("angle_tune".into(), SettingValue::Num(signed as f32));
        }
        // SensorOffset's value is in a[9] (a[8] is unused).
        if let Ok(r) = self.run(2, 3, 1, 149, &[dev]) {
            let v = u8_at(&r, 9).unwrap_or(0);
            let signed = if v > 100 { v as i64 - 256 } else { v as i64 };
            settings.insert("sensor_offset".into(), SettingValue::Num(signed as f32));
        }
        // Items valid only behind a dongle (a3/ignored on wired FW): receiver 3 lights,
        // glass mode. The DPI indicator is a3 even on wireless, so it is not read.
        if self.is_dongle() {
            // Master switch: the mode at light_idx=255 — the official K0()
            // treats it as ON when a known setting exists and its value is non-zero (the real device returns 0xFF).
            if let Ok(r) = self.run(1, 5, 2, 128, &[dev, 255]) {
                settings.insert(
                    "dongle_led".into(),
                    SettingValue::Bool(u8_at(&r, 9).unwrap_or(0) != 0),
                );
            }
            for (id, idx) in [("dongle_led1", 5u8), ("dongle_led2", 3), ("dongle_led3", 4)] {
                if let Ok(r) = self.run(1, 5, 2, 128, &[dev, idx]) {
                    settings.insert(
                        id.into(),
                        SettingValue::Choice(u8_at(&r, 9).unwrap_or(0) as i64),
                    );
                }
            }
        }
        if let Ok(v) = self.get(2, 0, 136, dev) {
            settings.insert("debounce".into(), SettingValue::Num(v as f32));
        }
        if let Ok(r) = self.run(2, 3, 0, 135, &[dev]) {
            let secs = u16::from_be_bytes([u8_at(&r, 8).unwrap_or(0), u8_at(&r, 9).unwrap_or(0)]);
            // The official app shows 0/0xFF00/0xFFFF as "disabled" (not among the write options).
            // In that case leave the value out — the UI shows no selection, and choosing one writes a valid number of minutes.
            if !matches!(secs, 0 | 0xFF00 | 0xFFFF) {
                settings.insert(
                    "sleep_min".into(),
                    SettingValue::Choice(((secs / 60).max(1)) as i64),
                );
            }
        }
        // SpeedEnable: a[8]=L, a[9]=R.
        if let Ok(r) = self.run(2, 3, 0, 154, &[dev]) {
            settings.insert(
                "rapid_left".into(),
                SettingValue::Bool(u8_at(&r, 8).unwrap_or(0) == 1),
            );
            settings.insert(
                "rapid_right".into(),
                SettingValue::Bool(u8_at(&r, 9).unwrap_or(0) == 1),
            );
        }
        // ScrollHP (B-hop): a[8]=direction, a[9..11]=u16BE ms.
        if let Ok(r) = self.run(2, 4, 0, 153, &[dev]) {
            settings.insert(
                "bhop_dir".into(),
                SettingValue::Choice(u8_at(&r, 8).unwrap_or(0) as i64),
            );
            let ms = u16::from_be_bytes([u8_at(&r, 9).unwrap_or(0), u8_at(&r, 10).unwrap_or(0)]);
            settings.insert("bhop_ms".into(), SettingValue::Num(ms as f32));
        }
        // Speed-linked DPI (AdaDPI): a[8]=enabled, a[12]=point count, a[13..]=(speed u16BE, multiplier×100 u16BE)×N.
        let adaptive = if self
            .caps_now()
            .mouse
            .as_ref()
            .is_some_and(|m| m.adaptive_dpi.is_some())
        {
            self.read_adaptive(dev).ok()
        } else {
            None
        };
        // Glass mode: [2,1,1,142] (no profile argument) → a[7].
        if self.has("glass_mode") {
            if let Ok(r) = self.run(2, 1, 1, 142, &[]) {
                settings.insert(
                    "glass_mode".into(),
                    SettingValue::Bool(u8_at(&r, 7).unwrap_or(0) == 1),
                );
            }
        }

        // DPI stages: get [dev,6] → a[8]=count, a[9..]=4B×N (X,Y u16BE).
        // Stay empty if unreadable (abnormal values are not rounded).
        let stages = self.read_stages(dev).unwrap_or_default();
        let mut active = 0usize;
        if let Ok(v) = self.get(2, 1, 130, dev) {
            active = (v as usize)
                .saturating_sub(1)
                .min(stages.len().saturating_sub(1));
        }

        let buttons = (1..=5u8).map(|b| self.read_button(dev, b)).collect();

        Ok(DeviceState {
            profile: ProfileState {
                active: prof.saturating_sub(1),
                names: vec![],
            },
            settings,
            mouse: Some(MouseState {
                stages,
                active_stage: active,
                buttons,
                macros: vec![],
                adaptive,
            }),
            keyboard: None,
            lighting: BTreeMap::new(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        // Allow a profile switch even when the active profile is unknown (it can be the means of recovery).
        if let Change::Profile(i) = change {
            return self
                .run(2, 1, 0, 5, &[i.saturating_add(1)])
                .map(|_| ());
        }
        let prof = self.active_profile()?;
        match change {
            Change::Setting { id, value } => match id.as_str() {
                "polling_wireless" => {
                    let hz = value.as_choice();
                    let allowed = self
                        .caps_now()
                        .setting("polling_wireless")
                        .is_some_and(|sp| match &sp.kind {
                            SettingKind::Choice { options } => options.iter().any(|o| o.value == hz),
                            _ => false,
                        });
                    if !allowed {
                        return Err(HubError::Invalid("polling rate not available on this connection".into()));
                    }
                    let code = match hz {
                        125 => 8,
                        250 => 4,
                        500 => 2,
                        1000 => 1,
                        2000 => 32,
                        4000 => 64,
                        8000 => 128,
                        _ => return Err(HubError::Invalid("polling rate".into())),
                    };
                    self.set(2, 1, 0, prof, code)
                }
                "lod" => {
                    // Official setLOD: <1mm → v*10|0x80, 1.xmm → BCD 0x1x, otherwise → mm.
                    let mm10 = value.as_choice();
                    let v = match mm10 {
                        1..=9 => (mm10 | 0x80) as u8,
                        10 | 20 => (mm10 / 10) as u8,
                        11..=19 => (((mm10 / 10) << 4) | (mm10 % 10)) as u8,
                        _ => return Err(HubError::Invalid("lod".into())),
                    };
                    self.set(2, 1, 8, prof, v)
                }
                "motion_sync" => self.set(2, 1, 9, prof, value.as_bool() as u8),
                "angle_snap" => self.set(2, 1, 4, prof, value.as_bool() as u8),
                "ripple" => self.set(2, 1, 10, prof, value.as_bool() as u8),
                "hyper_mode" => {
                    let on = value.as_bool();
                    self.set(2, 1, 11, prof, on as u8)?;
                    // Official s2(): when competitive mode is OFF, also turn 20K FPS OFF.
                    if !on {
                        self.set(2, 1, 19, prof, 0)?;
                    }
                    Ok(())
                }
                "tracking_mode" => {
                    let on = value.as_bool();
                    // Official: 20K FPS can be enabled only while competitive mode is ON.
                    if on && self.get(2, 1, 139, prof).ok() != Some(1) {
                        return Err(HubError::Invalid(
                            "Turn on competitive mode before enabling 20K FPS".into(),
                        ));
                    }
                    self.set(2, 1, 19, prof, on as u8)
                }
                "dpi_xy" => {
                    // Official $2(): also rewrite the stage X/Y on toggle (Y=X when OFF).
                    let on = value.as_bool();
                    self.set(2, 1, 13, prof, on as u8)?;
                    let stages = self.read_stages(prof)?;
                    if !stages.is_empty() {
                        let fixed: Vec<DpiStage> = stages
                            .iter()
                            .map(|s| DpiStage {
                                y: if on { Some(s.y.unwrap_or(s.x)) } else { None },
                                ..*s
                            })
                            .collect();
                        self.write_stages(prof, &fixed)?;
                    }
                    Ok(())
                }
                "angle_tune" => {
                    let v = value.as_num() as i32;
                    self.set(
                        2,
                        1,
                        20,
                        prof,
                        if v < 0 { (255 + v + 1) as u8 } else { v as u8 },
                    )
                }
                "sensor_offset" => {
                    // The value is in o[8] (o[7] is unused) — same as the official setSensorOffsetValue.
                    let v = value.as_num().clamp(-5., 5.) as i32;
                    let enc = if v < 0 { (255 + v + 1) as u8 } else { v as u8 };
                    let read_back = || {
                        self.run(2, 3, 1, 149, &[prof])
                            .ok()
                            .and_then(|r| u8_at(&r, 9).ok())
                    };
                    if read_back() == Some(enc) {
                        return Ok(());
                    }
                    match self.run(2, 3, 1, 21, &[prof, 0, enc]) {
                        Ok(_) => Ok(()),
                        Err(e) => {
                            if read_back() == Some(enc) {
                                return Ok(());
                            }
                            Err(e)
                        }
                    }
                }
                "dongle_led" => {
                    // Official: ON=SetLightness(1,prof,0,255),
                    // OFF=SetLightEffect idx255 mode0. ON when the mode value is non-zero.
                    if value.as_bool() {
                        self.run(1, 3, 2, 2, &[prof, 0, 255]).map(|_| ())
                    } else {
                        self.run(1, 5, 2, 0, &[prof, 255, 0, 0, 0]).map(|_| ())
                    }
                }
                "dongle_led1" | "dongle_led2" | "dongle_led3" => {
                    let idx = match id.as_str() {
                        "dongle_led1" => 5,
                        "dongle_led2" => 3,
                        _ => 4,
                    };
                    let mode = value.as_choice() as u8;
                    self.run(1, 5, 2, 0, &[prof, idx, mode, 0, 0]).map(|_| ())
                }
                // Glass mode: official setGlassModeDetect [2,1,1,14,v] (no profile argument).
                "glass_mode" => {
                    let v = value.as_bool() as u8;
                    let read = || {
                        self.run(2, 1, 1, 142, &[])
                            .ok()
                            .and_then(|r| u8_at(&r, 7).ok())
                    };
                    if read() == Some(v) {
                        return Ok(());
                    }
                    let res = self.run(2, 1, 1, 14, &[v]).map(|_| ());
                    match read() {
                        Some(b) if b == v => Ok(()),
                        Some(_) => Err(HubError::Protocol(
                            "Glass mode was not applied (this firmware may not support it)".into(),
                        )),
                        None => res,
                    }
                }
                "debounce" => self.set(2, 0, 8, prof, value.as_num().clamp(0., 255.) as u8),
                "sleep_min" => {
                    // The official options are only 1/5/10/15/20/25 minutes (OFF cannot be written).
                    let min = value.as_choice();
                    if !matches!(min, 1 | 5 | 10 | 15 | 20 | 25) {
                        return Err(HubError::Invalid("Choose a sleep time between 1 and 25 minutes".into()));
                    }
                    let secs = (min * 60) as u16;
                    let [hi, lo] = secs.to_be_bytes();
                    let read_back = || {
                        self.run(2, 3, 0, 135, &[prof]).ok().map(|r| {
                            u16::from_be_bytes([
                                u8_at(&r, 8).unwrap_or(0),
                                u8_at(&r, 9).unwrap_or(0),
                            ])
                        })
                    };
                    if read_back() == Some(secs) {
                        return Ok(());
                    }
                    match self.run(2, 3, 0, 7, &[prof, hi, lo]) {
                        Ok(_) => Ok(()),
                        Err(_) if read_back() == Some(secs) => Ok(()),
                        Err(e) => Err(e),
                    }
                }
                "rapid_left" | "rapid_right" => {
                    let left = id == "rapid_left";
                    let read_lr = || {
                        self.run(2, 3, 0, 154, &[prof])
                            .ok()
                            .map(|r| (u8_at(&r, 8).unwrap_or(0), u8_at(&r, 9).unwrap_or(0)))
                    };
                    let (cur_l, cur_r) =
                        read_lr().ok_or(HubError::Protocol("speedEnable".into()))?;
                    let (nl, nr) = if left {
                        (value.as_bool() as u8, cur_r)
                    } else {
                        (cur_l, value.as_bool() as u8)
                    };
                    if (nl, nr) == (cur_l, cur_r) {
                        return Ok(());
                    }
                    match self.run(2, 3, 0, 26, &[prof, nl, nr]) {
                        Ok(_) => Ok(()),
                        Err(_) if read_lr() == Some((nl, nr)) => Ok(()),
                        Err(e) => Err(e),
                    }
                }
                "bhop_dir" | "bhop_ms" => {
                    let read_bp = || {
                        self.run(2, 4, 0, 153, &[prof]).ok().map(|r| {
                            (
                                u8_at(&r, 8).unwrap_or(0),
                                u16::from_be_bytes([
                                    u8_at(&r, 9).unwrap_or(0),
                                    u8_at(&r, 10).unwrap_or(200),
                                ]),
                            )
                        })
                    };
                    let (cur_dir, cur_ms) = read_bp().unwrap_or((0, 200));
                    let dir = if id == "bhop_dir" {
                        value.as_choice() as u8
                    } else {
                        cur_dir
                    };
                    let ms = if id == "bhop_ms" {
                        (value.as_num().clamp(200., 1000.) as u16 / 200) * 200
                    } else {
                        cur_ms
                    };
                    if (dir, ms) == (cur_dir, cur_ms) {
                        return Ok(());
                    }
                    let [hi, lo] = ms.to_be_bytes();
                    match self.run(2, 4, 0, 25, &[prof, dir, hi, lo]) {
                        Ok(_) => Ok(()),
                        Err(_) if read_bp() == Some((dir, ms)) => Ok(()),
                        Err(e) => Err(e),
                    }
                }
                _ => Err(HubError::Unsupported("setting")),
            },
            Change::DpiStages { stages, active } => {
                // Three commands: value → color → active stage — finish input validation before writing.
                self.write_stages(prof, stages)?;
                self.set(2, 1, 2, prof, (*active as u8) + 1)?;
                // The per-axis X/Y flag is synced with the stage contents (the UI toggle only
                // rewrites the stage, so keep it consistent here, as the official $2() does).
                self.set(
                    2,
                    1,
                    13,
                    prof,
                    stages.iter().any(|s| s.y.is_some()) as u8,
                )
            }
            Change::ActiveStage(a) => self.set(2, 1, 2, prof, (*a as u8) + 1),
            Change::Adaptive(c) => self.write_adaptive(prof, c),
            Change::Button { slot, action } => {
                if *slot >= 5 {
                    return Err(HubError::Invalid("button slot".into()));
                }
                self.write_button(prof, (*slot as u8) + 1, action)
            }
            _ => Err(HubError::Unsupported("change")),
        }
    }

    fn run_action(&self, id: &str, _file: Option<&std::path::Path>) -> HubResult<ActionOutcome> {
        match id {
            "reset_profile" => {
                // Do not erase profile 1 while the active profile is unknown — report the failure.
                let prof = self.active_profile()?;
                self.run(2, 1, 0, 13, &[prof])?;
                sleep(Duration::from_millis(500));
                Ok(ActionOutcome {
                    message: Some("Profile reset".into()),
                    reload: true,
                })
            }
            "factory_reset" => {
                // Official Xv(): defaultDevice only sends (no response). On wireless, after 1 second
                // the receiver (target 0) is also reset. No resend (avoids a double reset).
                {
                    let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
                    send_only(&dev, 2, 2, 0, 0, &[192, 1])?;
                }
                if self.is_dongle() {
                    sleep(Duration::from_millis(1000));
                    let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
                    send_only(&dev, 0, 2, 0, 0, &[192, 1])?;
                }
                sleep(Duration::from_millis(1500));
                Ok(ActionOutcome {
                    message: Some("Restored factory defaults".into()),
                    reload: true,
                })
            }
            _ => Err(HubError::Unsupported("action")),
        }
    }

    fn battery(&self) -> HubResult<Battery> {
        let r = self.run(2, 2, 0, 131, &[])?;
        Ok(Battery {
            percent: Some(u8_at(&r, 8).unwrap_or(0).min(100)),
            charging: r.get(7).copied().unwrap_or(0) == 1,
        })
    }
}
