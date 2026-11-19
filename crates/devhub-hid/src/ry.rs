//! RongYuan (USB VID 3151) driver: keyboards of the "yc500" command family,
//! mice of the "pan1080" family, and the 2.4G receivers that relay to them.
//!
//! Every command is a 64-byte feature report on the vendor collection
//! (usage page FFFF, usage 2) with a one-byte checksum. A receiver answers a
//! handful of opcodes itself and forwards the rest over the radio; the reply
//! to a forwarded GET has to be fetched with a second exchange.
//!
//! One USB product id is shared by many boards, so the model is never taken
//! from the PID: the device is asked for its id (`0x8F`) and only boards in
//! [`BOARDS`] are driven. Protocol notes and what was checked on hardware are
//! in `docs/PROTOCOL_NOTES.md` (RongYuan section).

use crate::transport::Channel;
use devhub_core::backend::*;
use devhub_core::keys;
use devhub_core::model::*;
use devhub_core::registry;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

type Events = Option<Arc<dyn Fn(HubEvent) + Send + Sync>>;

mod op {
    pub const IDENTIFY: u8 = 0x8F;
    pub const REVISION: u8 = 0x80;
    pub const GET_PROFILE: u8 = 0x85;
    // Keyboard (yc500).
    pub const GET_KEYMAP: u8 = 0x89;
    pub const GET_FN: u8 = 0x90;
    pub const SET_DEBOUNCE: u8 = 0x11;
    pub const GET_DEBOUNCE: u8 = 0x91;
    pub const SET_KEY: u8 = 0x13;
    pub const SET_FN_KEY: u8 = 0x15;
    pub const SET_AUTO_OS: u8 = 0x17;
    pub const GET_AUTO_OS: u8 = 0x97;
    // Mouse (pan1080).
    pub const MOUSE_SET_KEY: u8 = 0x50;
    pub const MOUSE_GET_KEYS: u8 = 0xD0;
    pub const MOUSE_SET_OPTIONS: u8 = 0x53;
    pub const MOUSE_GET_OPTIONS: u8 = 0xD3;
    pub const MOUSE_SET_DPI: u8 = 0x54;
    pub const MOUSE_GET_DPI: u8 = 0xD4;
    // Receiver, answered locally.
    pub const RX_SELECT: u8 = 0xF6;
    pub const RX_STATUS: u8 = 0xF7;
    pub const RX_RELEASE: u8 = 0xFC;
}

/// Relay targets for `RX_SELECT`.
const TARGET_KEYBOARD: u8 = 10;
const TARGET_MOUSE: u8 = 5;

const ASLEEP: &str = "the device is asleep or out of range of its receiver";

// ==================================================================== packets

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sum {
    /// Over bytes 0..=6, stored at byte 7. Every command this driver sends.
    Bit7,
    /// Over bytes 0..=7, stored at byte 8. The LED commands, which are not
    /// sent (see the keyboard section); kept so the framing stays documented.
    #[allow(dead_code)]
    Bit8,
}

fn packet(body: &[u8], sum: Sum) -> [u8; 64] {
    let mut p = [0u8; 64];
    p[..body.len()].copy_from_slice(body);
    let at = match sum {
        Sum::Bit7 => 7,
        Sum::Bit8 => 8,
    };
    let total: u32 = p[..at].iter().map(|b| *b as u32).sum();
    p[at] = 0xFF - (total & 0xFF) as u8;
    p
}

/// The receiver's own status reply (`0xF7`). Only the first bytes are
/// meaningful; the rest of the buffer keeps whatever was relayed last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RxStatus {
    /// A relayed reply is waiting for `RX_RELEASE`.
    reply_ready: bool,
    keyboard_battery: u8,
    mouse_battery: u8,
    keyboard_online: bool,
    mouse_online: bool,
    /// The receiver will take a packet to relay.
    can_send: bool,
}

fn parse_status(r: &[u8; 64]) -> RxStatus {
    RxStatus {
        reply_ready: r[0] == 1,
        keyboard_battery: r[1],
        mouse_battery: r[2],
        keyboard_online: r[3] == 0,
        mouse_online: r[4] == 0,
        can_send: r[5] == 1,
    }
}

// ==================================================================== link

/// The feature-report channel plus, for a receiver, the relay to the device
/// behind it.
struct Link {
    ch: Channel,
    /// `Some(target)` when this is a receiver.
    relay: Option<u8>,
}

impl Link {
    fn reply(&self) -> HubResult<[u8; 64]> {
        let r = self.ch.get_feature()?;
        let mut out = [0u8; 64];
        out.copy_from_slice(&r[1..65]);
        Ok(out)
    }

    fn status(&self) -> HubResult<RxStatus> {
        self.ch.send_feature(&packet(&[op::RX_STATUS], Sum::Bit7))?;
        sleep(Duration::from_millis(10));
        Ok(parse_status(&self.reply()?))
    }

    /// Wait until the receiver sees its device and will take a packet.
    fn relay_ready(&self, target: u8) -> HubResult<()> {
        for _ in 0..5 {
            let st = self.status()?;
            let online = if target == TARGET_MOUSE {
                st.mouse_online
            } else {
                st.keyboard_online
            };
            if !online {
                return Err(HubError::Protocol(ASLEEP.into()));
            }
            if st.can_send {
                return Ok(());
            }
            sleep(Duration::from_millis(100));
        }
        Err(HubError::Protocol("receiver busy".into()))
    }

    fn forward(&self, target: u8, p: &[u8; 64]) -> HubResult<()> {
        self.relay_ready(target)?;
        self.ch
            .send_feature(&packet(&[op::RX_SELECT, target], Sum::Bit7))?;
        sleep(Duration::from_millis(10));
        self.ch.send_feature(p)
    }

    /// Send a GET and return the 64-byte reply.
    fn get(&self, body: &[u8]) -> HubResult<[u8; 64]> {
        let p = packet(body, Sum::Bit7);
        let Some(target) = self.relay else {
            self.ch.send_feature(&p)?;
            sleep(Duration::from_millis(20));
            return self.reply();
        };
        self.forward(target, &p)?;
        // The reply comes back over the radio about 100-150 ms later.
        for _ in 0..25 {
            sleep(Duration::from_millis(40));
            if self.status()?.reply_ready {
                self.ch
                    .send_feature(&packet(&[op::RX_RELEASE], Sum::Bit7))?;
                sleep(Duration::from_millis(15));
                return self.reply();
            }
        }
        Err(HubError::Protocol(ASLEEP.into()))
    }

    /// A GET whose reply repeats the opcode in byte 0. An unsupported or
    /// dropped command leaves the previous reply in place, so a mismatch is
    /// retried and then reported instead of being parsed.
    fn get_echo(&self, body: &[u8]) -> HubResult<[u8; 64]> {
        let mut last = Err(HubError::Protocol("no reply".into()));
        // A board that is still storing a write answers zeros for a while.
        // Reports sent into that window can stall its control endpoint until
        // it is re-plugged, so wait a long beat before each of a few retries.
        for _ in 0..4 {
            match self.get(body) {
                Ok(r) if r[0] == body[0] => return Ok(r),
                Ok(r) => {
                    last = Err(HubError::Protocol(format!(
                        "command {:#04x} answered {:#04x}",
                        body[0], r[0]
                    )));
                    sleep(Duration::from_millis(300));
                }
                Err(e) => return Err(e),
            }
        }
        last
    }

    /// Send a SET. Nothing comes back; callers read the value again.
    fn set(&self, p: &[u8; 64]) -> HubResult<()> {
        match self.relay {
            None => {
                self.ch.send_feature(p)?;
                // The board stores the write in flash; the next report must
                // not arrive while it does (see `get_echo`).
                sleep(Duration::from_millis(500));
            }
            Some(target) => {
                self.forward(target, p)?;
                sleep(Duration::from_millis(160));
            }
        }
        Ok(())
    }
}

// ==================================================================== boards

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    /// yc500 keyboard commands.
    Keyboard,
    /// pan1080 mouse commands.
    Mouse,
}

#[derive(Debug)]
struct Board {
    /// The id the device reports to `0x8F`.
    id: u32,
    vendor: &'static str,
    model: &'static str,
    name: &'static str,
    family: Family,
}

/// Boards this driver knows. Anything else on the same USB ids is refused.
const BOARDS: &[Board] = &[
    Board {
        id: 694,
        vendor: "Fantech",
        model: "maxfit81",
        name: "MAXFIT81",
        family: Family::Keyboard,
    },
    // Reported by the Fantech Tanto PRO WG13P; the vendor's table calls this
    // platform "Tanto PRO S" (PAW3950).
    Board {
        id: 3109,
        vendor: "Fantech",
        model: "tanto-pro",
        name: "Tanto PRO WG13P",
        family: Family::Mouse,
    },
];

/// Which kind of device a registry entry is for. A board of the other kind
/// answering there would be sent the wrong commands, so it is refused.
fn family_for_model(model: &str) -> Option<Family> {
    match model {
        "maxfit81" | "ry-keyboard-receiver" => Some(Family::Keyboard),
        "tanto-pro" | "ry-mouse-receiver" => Some(Family::Mouse),
        _ => None,
    }
}

/// Ask the device which board it is. The USB ids are shared with other
/// vendors' products, so an id outside `BOARDS` is an error, never a guess.
fn identify(link: &Link) -> HubResult<&'static Board> {
    let r = link.get_echo(&[op::IDENTIFY])?;
    let id = u32::from_le_bytes([r[1], r[2], r[3], r[4]]);
    BOARDS
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| HubError::Protocol(format!("RongYuan board id {id} is not supported yet")))
}

// ==================================================================== keymap values

/// A keymap slot is four bytes; the contract carries it as a big-endian u32
/// so a plain key reads `0x0000_UU00`.
fn code_of(v: [u8; 4]) -> u32 {
    u32::from_be_bytes(v)
}

const KEY_FN: u32 = 0x0A01_0000;

/// Factory keymap of the MAXFIT81, slots 0..96, as read from a board whose
/// keymap showed no remaps. Used for "reset layer" and to name the
/// firmware-only functions that sit on its Fn layer.
const MAXFIT81_BASE: &str = "\
0000290000003500 00002b0000003900 0000e1000000e000 0000000000001e00 \
0000140000000400 0000640000000000 00003a0000001f00 00001a0000001600 \
00001d000000e300 00003b0000002000 0000080000000700 00001b000000e200 \
00003c0000002100 0000150000000900 0000060000000000 00003d0000002200 \
0000170000000a00 0000190000000000 00003e0000002300 00001c0000000b00 \
0000050000002c00 00003f0000002400 0000180000000d00 0000110000000000 \
0000400000002500 00000c0000000e00 0000100000000000 0000410000002600 \
0000120000000f00 000036000000e600 0000420000002700 0000130000003300 \
000037000a010000 0000430000002d00 00002f0000003400 000038000000e400 \
0000440000002e00 0000300000003200 0000e50000005000 0000450000002a00 \
0000310000002800 0000520000005100 000000000300e200 00004c0000004b00 \
00004e0000004f00 0300e9000300ea00 0000000000000000 0000000000000000";

const MAXFIT81_FN: &str = "\
0a02000000000000 0000000000000000 0000000013000000 000000000e000000 \
000000000a050000 0000000000000000 030070000e000100 000000000a050100 \
000000000a030000 03006f000e000200 0000000000000000 0000000000000000 \
0000e32b0e000500 0000000000000000 0000000000000000 0000e3080e000600 \
0000000000000000 0000000000000000 03008a0100000000 0000000000000000 \
0000000000000000 0300230200000000 0000000000000000 0000000000000000 \
0300b60000000000 0000000000000000 0d08000000000000 0300cd0000000000 \
0000460000004900 0d08030000000000 0300b50000000000 0000470000004a00 \
0d08010000000000 0300e20000000000 0000480000004d00 0d08020000000000 \
0300ea0000000000 0000000000000000 000000000d030100 0300e90000000000 \
0a0c000000000000 0d0201000d020200 0000000000000000 0d0100000d050100 \
0d0500000d030200 0000000000000000 0000000000000000 0000000000000000";

/// Slot values 0..96 from one of the tables above.
fn default_table(hex: &str) -> Vec<[u8; 4]> {
    let digits: Vec<u8> = hex
        .bytes()
        .filter(u8::is_ascii_hexdigit)
        .map(|c| (c as char).to_digit(16).unwrap_or(0) as u8)
        .collect();
    digits
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| {
            [
                c[0] << 4 | c[1],
                c[2] << 4 | c[3],
                c[4] << 4 | c[5],
                c[6] << 4 | c[7],
            ]
        })
        .collect()
}

fn keyboard_catalog(layout: &[LayoutKey]) -> Vec<KeyGroup> {
    let group = |name: &str, keys: Vec<(u32, String, Option<String>)>| KeyGroup {
        name: name.into(),
        keys: keys
            .into_iter()
            .map(|(code, label, short)| KeyChoice { code, label, short })
            .collect(),
    };
    let basic = keys::basic_group()
        .keys
        .into_iter()
        .map(|k| (k.code << 8, k.label, k.short))
        .collect();
    let consumer = |usage: u16, label: &str, short: &str| {
        (
            code_of([3, 0, usage as u8, (usage >> 8) as u8]),
            label.to_string(),
            Some(short.to_string()),
        )
    };
    let media = vec![
        consumer(0xE9, "Volume up", "Vol+"),
        consumer(0xEA, "Volume down", "Vol-"),
        consumer(0xE2, "Mute", "Mute"),
        consumer(0xCD, "Play / pause", "Play"),
        consumer(0xB5, "Next track", "Next"),
        consumer(0xB6, "Previous track", "Prev"),
        consumer(0xB7, "Stop", "Stop"),
        consumer(0x6F, "Brightness up", "Bri+"),
        consumer(0x70, "Brightness down", "Bri-"),
        consumer(0x192, "Calculator", "Calc"),
        consumer(0x194, "My computer", "PC"),
        consumer(0x18A, "Mail", "Mail"),
        consumer(0x223, "Browser home", "Web"),
    ];
    let mouse = [
        (0xF0, "Left click", "LMB"),
        (0xF1, "Right click", "RMB"),
        (0xF2, "Middle click", "MMB"),
        (0xF3, "Back", "Back"),
        (0xF4, "Forward", "Fwd"),
    ]
    .iter()
    .map(|(b, l, s)| (code_of([1, 0, *b, 0]), l.to_string(), Some(s.to_string())))
    .collect();
    let basic: Vec<(u32, String, Option<String>)> = basic;
    let mouse: Vec<(u32, String, Option<String>)> = mouse;
    // Lighting, connection and mode keys exist only as firmware functions,
    // and two Fn keys send a key combination. Their meaning is printed on the
    // keycaps, so they are offered under the factory position they come from.
    let mut firmware = vec![(KEY_FN, "Fn".to_string(), Some("Fn".to_string()))];
    for (slot, v) in default_table(MAXFIT81_FN).iter().enumerate() {
        let code = code_of(*v);
        let named = |list: &[(u32, String, Option<String>)]| list.iter().any(|k| k.0 == code);
        if code == 0 || named(&basic) || named(&media) || named(&mouse) || named(&firmware) {
            continue;
        }
        if let Some(key) = layout.iter().find(|k| k.id as usize == slot) {
            firmware.push((
                code,
                format!("Factory Fn + {}", key.label),
                Some(format!("Fn·{}", key.label)),
            ));
        }
    }
    vec![
        group("Basic", basic),
        group("Media", media),
        group("Mouse", mouse),
        group("Firmware functions", firmware),
    ]
}

// ==================================================================== mouse values

const MOUSE_BUTTONS: [&str; 6] = [
    "Left button",
    "Right button",
    "Wheel click",
    "Forward",
    "Back",
    "DPI button",
];

fn rate_from_wire(code: u8) -> Option<i64> {
    Some(match code {
        0x81 => 8000,
        0x82 => 4000,
        0x84 => 2000,
        1 => 1000,
        2 => 500,
        4 => 250,
        8 => 125,
        _ => return None,
    })
}

fn rate_to_wire(hz: i64) -> Option<u8> {
    Some(match hz {
        1000 => 1,
        500 => 2,
        250 => 4,
        125 => 8,
        _ => return None,
    })
}

fn mouse_action_from(v: [u8; 4]) -> ButtonAction {
    let raw = || ButtonAction::Unsupported {
        ty: v[0],
        params: v[1..].to_vec(),
    };
    match v {
        [0, 0, 0, 0] => ButtonAction::Disabled,
        [1, 0, 0xF0, 0] => ButtonAction::Mouse(MouseFn::Left),
        [1, 0, 0xF1, 0] => ButtonAction::Mouse(MouseFn::Right),
        [1, 0, 0xF2, 0] => ButtonAction::Mouse(MouseFn::Middle),
        [1, 0, 0xF3, 0] => ButtonAction::Mouse(MouseFn::Back),
        [1, 0, 0xF4, 0] => ButtonAction::Mouse(MouseFn::Forward),
        [0x14, 0, 0, 0] => ButtonAction::Dpi(DpiFn::Cycle),
        // Up to three HID usages; modifiers are usages too.
        [0, a, b, c] => {
            let mut mods = 0u8;
            let mut key = 0u16;
            for u in [a, b, c] {
                match keys::mod_bit(u as u16) {
                    Some(bit) => mods |= bit,
                    None if u == 0 => {}
                    None if key == 0 => key = u as u16,
                    None => return raw(),
                }
            }
            ButtonAction::Key(KeyStroke { mods, usage: key })
        }
        _ => raw(),
    }
}

fn mouse_action_to(a: &ButtonAction) -> HubResult<[u8; 4]> {
    Ok(match a {
        ButtonAction::Disabled => [0, 0, 0, 0],
        ButtonAction::Mouse(MouseFn::Left) => [1, 0, 0xF0, 0],
        ButtonAction::Mouse(MouseFn::Right) => [1, 0, 0xF1, 0],
        ButtonAction::Mouse(MouseFn::Middle) => [1, 0, 0xF2, 0],
        ButtonAction::Mouse(MouseFn::Back) => [1, 0, 0xF3, 0],
        ButtonAction::Mouse(MouseFn::Forward) => [1, 0, 0xF4, 0],
        ButtonAction::Dpi(DpiFn::Cycle) => [0x14, 0, 0, 0],
        ButtonAction::Key(k) => {
            let mut usages: Vec<u8> = (0..8u8)
                .filter(|bit| k.mods & (1 << bit) != 0)
                .map(|bit| 0xE0 + bit)
                .collect();
            if k.usage != 0 {
                usages.push(k.usage as u8);
            }
            if usages.is_empty() || usages.len() > 3 || k.usage > 0xFF {
                return Err(HubError::Invalid(
                    "a button takes one key with at most two modifiers".into(),
                ));
            }
            // One key sits at byte 2; a second follows it and a third goes in
            // front (`[0,0,key,0]`, `[0,0,mod,key]`, `[0,mod,mod,key]`).
            match usages[..] {
                [a] => [0, 0, a, 0],
                [a, b] => [0, 0, a, b],
                [a, b, c] => [0, a, b, c],
                _ => unreachable!("length checked above"),
            }
        }
        ButtonAction::Unsupported { ty, params } if params.len() == 3 => {
            [*ty, params[0], params[1], params[2]]
        }
        other => {
            return Err(HubError::Invalid(format!(
                "this mouse cannot take the action {other:?}"
            )))
        }
    })
}

// ==================================================================== driver

pub struct RyDriver {
    link: Link,
    info: Mutex<DeviceInfo>,
    /// The board the device said it is. `None` until it has answered (a
    /// receiver whose device sleeps); nothing is read or written before that.
    board: Mutex<Option<&'static Board>>,
    /// The device kind this interface was registered for.
    expected: Family,
    events: Events,
}

pub fn open(dev: hidapi::HidDevice, info: DeviceInfo, events: Events) -> HubResult<RyDriver> {
    let relay = match info.model.as_str() {
        "ry-keyboard-receiver" => Some(TARGET_KEYBOARD),
        "ry-mouse-receiver" => Some(TARGET_MOUSE),
        _ => None,
    };
    let link = Link {
        ch: Channel::open(dev, 65, 65),
        relay,
    };
    build(link, info, events)
}

fn build(link: Link, info: DeviceInfo, events: Events) -> HubResult<RyDriver> {
    let expected = family_for_model(&info.model)
        .ok_or_else(|| HubError::Protocol(format!("unknown RongYuan model {}", info.model)))?;
    let driver = RyDriver {
        link,
        info: Mutex::new(info),
        board: Mutex::new(None),
        expected,
        // Attached below: the UI is still opening and takes `info()` itself.
        events: None,
    };
    match driver.board() {
        Ok(_) => {}
        // A receiver whose device sleeps: open anyway so the UI can say so.
        // Only this case is let through; an unknown id is an error above.
        Err(HubError::Protocol(m)) if m == ASLEEP && driver.link.relay.is_some() => {}
        Err(e) => return Err(e),
    }
    Ok(RyDriver { events, ..driver })
}

impl RyDriver {
    /// The board behind this link, asking the device first when it has not
    /// answered yet. Every read and write goes through here, so a device that
    /// was asleep at open is identified before anything is sent to it.
    fn board(&self) -> HubResult<&'static Board> {
        let mut slot = self.board.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(b) = *slot {
            return Ok(b);
        }
        let b = self.seen(identify(&self.link))?;
        if b.family != self.expected {
            return Err(HubError::Protocol(format!(
                "{} answered on an interface registered for another device kind",
                b.name
            )));
        }
        *slot = Some(b);
        drop(slot);
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner());
        i.vendor = b.vendor.into();
        i.model = b.model.into();
        // A cable keeps its own product string; a receiver has none to show.
        if self.link.relay.is_some() {
            i.name = b.name.into();
        }
        i.asleep = false;
        let info = i.clone();
        drop(i);
        // The name and model changed, which tells the UI to fetch caps again.
        if let Some(ev) = &self.events {
            ev(HubEvent::Info(info.id.clone(), Box::new(info)));
        }
        Ok(b)
    }

    fn set_asleep(&self, asleep: bool) {
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner());
        if self.link.relay.is_none() || i.asleep == asleep {
            return;
        }
        i.asleep = asleep;
        let info = i.clone();
        drop(i);
        if let Some(ev) = &self.events {
            ev(HubEvent::Info(info.id.clone(), Box::new(info)));
        }
    }

    /// Record whether the device behind a receiver answered.
    fn seen<T>(&self, r: HubResult<T>) -> HubResult<T> {
        match &r {
            Ok(_) => self.set_asleep(false),
            Err(HubError::Protocol(m)) if m == ASLEEP => self.set_asleep(true),
            Err(_) => {}
        }
        r
    }

    fn get(&self, body: &[u8]) -> HubResult<[u8; 64]> {
        self.seen(self.link.get_echo(body))
    }

    /// Pages and tables come back raw, without the opcode.
    fn get_raw(&self, body: &[u8]) -> HubResult<[u8; 64]> {
        self.seen(self.link.get(body))
    }

    fn set(&self, body: &[u8], sum: Sum) -> HubResult<()> {
        self.seen(self.link.set(&packet(body, sum)))
    }

    fn profile(&self) -> HubResult<u8> {
        Ok(self.get(&[op::GET_PROFILE])?[1])
    }

    // ---------------------------------------------------------------- keyboard
    //
    // Only writes that were read back and restored on a MAXFIT81 are offered:
    // single keys on both layers, debounce and the auto-OS switch. The board
    // also answers reads for lighting (`0x87`), the options block (`0x86`),
    // sleep timers (`0x92`) and profiles, but its control endpoint locked up
    // (until re-plugged) around an options write and again around an LED
    // write, so none of those is written until the cause is understood.

    fn keyboard_caps(&self) -> Capabilities {
        let layout = registry::maxfit81_layout();
        let catalog = keyboard_catalog(&layout);
        Capabilities {
            settings: vec![
                SettingSpec::toggle(
                    "auto_os",
                    section::KEYBOARD,
                    "Detect the operating system automatically",
                ),
                SettingSpec::slider("debounce", section::KEYBOARD, "Debounce", 1., 10., 1., ""),
            ],
            actions: vec![],
            profiles: None,
            mouse: None,
            keyboard: Some(KeyboardCaps {
                layout,
                layers: vec!["Main".into(), "Fn".into()],
                catalog,
                combos: false,
                perf: None,
                advanced: vec![],
                macros: None,
            }),
            lighting: vec![],
            battery: self.link.relay.is_some(),
            peq: None,
            firmware_feed: None,
        }
    }

    /// Slots 0..96 of one layer (six raw pages of sixteen slots).
    fn read_layer(&self, opcode: u8, profile: u8) -> HubResult<Vec<[u8; 4]>> {
        let mut slots = Vec::with_capacity(96);
        for page in 0..6u8 {
            let r = self.get_raw(&[opcode, profile, page])?;
            slots.extend(r.as_chunks::<4>().0.iter().copied());
        }
        Ok(slots)
    }

    fn keyboard_read(&self) -> HubResult<DeviceState> {
        let profile = self.profile()?;
        let mut settings = std::collections::BTreeMap::new();
        if let Ok(r) = self.get(&[op::GET_AUTO_OS]) {
            settings.insert("auto_os".into(), SettingValue::Bool(r[1] == 1));
        }
        if let Ok(r) = self.get(&[op::GET_DEBOUNCE]) {
            settings.insert("debounce".into(), SettingValue::Num(r[2] as f32));
        }

        let layout = registry::maxfit81_layout();
        let base = self.read_layer(op::GET_KEYMAP, profile)?;
        let fn_layer = self.read_layer(op::GET_FN, profile)?;
        let layer = |slots: &[[u8; 4]], empty: KeyAction| -> Vec<KeyAction> {
            layout
                .iter()
                .map(|k| match slots.get(k.id as usize) {
                    Some([0, 0, 0, 0]) | None => empty,
                    Some(v) => KeyAction::Code(code_of(*v)),
                })
                .collect()
        };
        Ok(DeviceState {
            profile: ProfileState {
                active: profile,
                names: vec![],
            },
            settings,
            mouse: None,
            keyboard: Some(KeyboardState {
                // An empty slot is a dead key on the base layer and falls
                // through to the base layer on Fn.
                keymap: vec![
                    layer(&base, KeyAction::None),
                    layer(&fn_layer, KeyAction::Transparent),
                ],
                perf: vec![],
                advanced: vec![],
                macros: vec![],
            }),
            lighting: Default::default(),
            peq: None,
        })
    }

    fn write_slot(&self, profile: u8, layer: u8, slot: u8, v: [u8; 4]) -> HubResult<()> {
        let opcode = match layer {
            0 => op::SET_KEY,
            1 => op::SET_FN_KEY,
            _ => return Err(HubError::Invalid(format!("layer {layer}"))),
        };
        let mut body = [0u8; 12];
        body[0] = opcode;
        body[1] = profile;
        body[2] = slot;
        body[8..12].copy_from_slice(&v);
        self.set(&body, Sum::Bit7)
    }

    fn keyboard_apply(&self, change: &Change) -> HubResult<()> {
        match change {
            Change::Setting { id, value } => match id.as_str() {
                "auto_os" => self.set(&[op::SET_AUTO_OS, value.as_bool() as u8], Sum::Bit7),
                "debounce" => {
                    let v = value.as_num().round().clamp(1., 10.) as u8;
                    self.set(&[op::SET_DEBOUNCE, 0, v], Sum::Bit7)
                }
                other => Err(HubError::Invalid(format!("unknown setting {other}"))),
            },
            Change::KeyMap { layer, key, action } => {
                let layout = registry::maxfit81_layout();
                let slot = layout
                    .get(*key)
                    .ok_or_else(|| HubError::Invalid(format!("key {key}")))?
                    .id as u8;
                let v = match action {
                    KeyAction::None | KeyAction::Transparent => [0u8; 4],
                    KeyAction::Code(c) => c.to_be_bytes(),
                    other => {
                        return Err(HubError::Invalid(format!(
                            "this keyboard cannot take {other:?}"
                        )))
                    }
                };
                self.write_slot(self.profile()?, *layer, slot, v)
            }
            // The same single-key write, once per key that differs.
            Change::KeyMapReset { layer } => {
                let (opcode, table) = match layer {
                    0 => (op::GET_KEYMAP, default_table(MAXFIT81_BASE)),
                    1 => (op::GET_FN, default_table(MAXFIT81_FN)),
                    _ => return Err(HubError::Invalid(format!("layer {layer}"))),
                };
                let profile = self.profile()?;
                let cur = self.read_layer(opcode, profile)?;
                for (slot, want) in table.iter().enumerate() {
                    if cur.get(slot) != Some(want) {
                        self.write_slot(profile, *layer, slot as u8, *want)?;
                    }
                }
                Ok(())
            }
            other => Err(HubError::Invalid(format!(
                "not available on this keyboard: {other:?}"
            ))),
        }
    }

    // ---------------------------------------------------------------- mouse

    fn mouse_caps(&self) -> Capabilities {
        let hz = |v: i64| ChoiceOpt::new(v, format!("{v} Hz"));
        Capabilities {
            settings: vec![
                SettingSpec::choice(
                    "polling",
                    section::PERFORMANCE,
                    "Polling rate",
                    vec![hz(125), hz(250), hz(500), hz(1000)],
                ),
                SettingSpec::choice(
                    "lod",
                    section::SENSOR,
                    "Lift-off distance",
                    vec![ChoiceOpt::new(1, "1 mm"), ChoiceOpt::new(2, "2 mm")],
                ),
                SettingSpec::toggle("angle_snap", section::SENSOR, "Angle snapping"),
                SettingSpec::slider("debounce", section::CLICK, "Debounce", 0., 10., 1., "ms"),
            ],
            actions: vec![],
            profiles: None,
            mouse: Some(MouseCaps {
                dpi_min: 50,
                dpi_max: 42_000,
                dpi_step: 50,
                min_stages: 1,
                max_stages: 5,
                xy_split: true,
                stage_colors: true,
                stage_toggle: false,
                buttons: MOUSE_BUTTONS
                    .iter()
                    .map(|n| ButtonSlot { name: (*n).into() })
                    .collect(),
                button_kinds: vec![
                    ButtonActionKind::Disabled,
                    ButtonActionKind::Mouse,
                    ButtonActionKind::Dpi,
                    ButtonActionKind::Key,
                ],
                require_left_click: true,
                macros: None,
                live_buttons: vec![],
                adaptive_dpi: None,
            }),
            keyboard: None,
            lighting: vec![],
            battery: self.link.relay.is_some(),
            peq: None,
            firmware_feed: None,
        }
    }

    fn mouse_read(&self) -> HubResult<DeviceState> {
        let profile = self.profile()?;
        let mut settings = std::collections::BTreeMap::new();
        let opt = self.get(&[op::MOUSE_GET_OPTIONS])?;
        if let Some(hz) = rate_from_wire(opt[9]) {
            settings.insert("polling".into(), SettingValue::Choice(hz));
        }
        settings.insert("debounce".into(), SettingValue::Num(opt[10] as f32));
        settings.insert("lod".into(), SettingValue::Choice(opt[52] as i64));
        settings.insert("angle_snap".into(), SettingValue::Bool(opt[53] == 1));

        let dpi = self.get(&[op::MOUSE_GET_DPI, profile])?;
        let count = (dpi[3] as usize).clamp(1, 5);
        let stages = (0..count)
            .map(|i| {
                let x = u16::from_le_bytes([dpi[8 + i * 2], dpi[9 + i * 2]]) as u32;
                let y = u16::from_le_bytes([dpi[24 + i * 2], dpi[25 + i * 2]]) as u32;
                DpiStage {
                    x,
                    y: (y != x).then_some(y),
                    color: Rgb::new(dpi[40 + i * 3], dpi[41 + i * 3], dpi[42 + i * 3]),
                    enabled: true,
                }
            })
            .collect();

        let keys = self.get_raw(&[op::MOUSE_GET_KEYS, profile])?;
        let buttons = (0..MOUSE_BUTTONS.len())
            .map(|i| {
                mouse_action_from([
                    keys[i * 4],
                    keys[i * 4 + 1],
                    keys[i * 4 + 2],
                    keys[i * 4 + 3],
                ])
            })
            .collect();
        Ok(DeviceState {
            profile: ProfileState {
                active: profile,
                names: vec![],
            },
            settings,
            mouse: Some(MouseState {
                stages,
                active_stage: (dpi[2] as usize).min(count - 1),
                buttons,
                macros: vec![],
                adaptive: None,
            }),
            keyboard: None,
            lighting: Default::default(),
            peq: None,
        })
    }

    /// The options block is written whole, so start from what the mouse holds
    /// and change one field; fields this driver does not model are kept.
    fn write_mouse_options(&self, f: impl FnOnce(&mut [u8; 64])) -> HubResult<()> {
        let mut body = self.get(&[op::MOUSE_GET_OPTIONS])?;
        body[..8].fill(0);
        body[0] = op::MOUSE_SET_OPTIONS;
        f(&mut body);
        self.set(&body, Sum::Bit7)
    }

    fn write_dpi(&self, stages: &[DpiStage], active: usize) -> HubResult<()> {
        if stages.is_empty() || stages.len() > 5 {
            return Err(HubError::Invalid("1 to 5 DPI stages".into()));
        }
        let mut body = [0u8; 64];
        body[0] = op::MOUSE_SET_DPI;
        body[1] = self.profile()?;
        body[2] = active.min(stages.len() - 1) as u8;
        body[3] = stages.len() as u8;
        for (i, s) in stages.iter().enumerate() {
            let clamp = |v: u32| (v.clamp(50, 42_000) / 50 * 50) as u16;
            let (x, y) = (clamp(s.x), clamp(s.y.unwrap_or(s.x)));
            body[8 + i * 2..10 + i * 2].copy_from_slice(&x.to_le_bytes());
            body[24 + i * 2..26 + i * 2].copy_from_slice(&y.to_le_bytes());
            body[40 + i * 3..43 + i * 3].copy_from_slice(&[s.color.r, s.color.g, s.color.b]);
        }
        self.set(&body, Sum::Bit7)
    }

    fn mouse_apply(&self, change: &Change) -> HubResult<()> {
        match change {
            Change::Setting { id, value } => match id.as_str() {
                "polling" => {
                    let code = rate_to_wire(value.as_choice())
                        .ok_or_else(|| HubError::Invalid("polling rate".into()))?;
                    self.write_mouse_options(|b| b[9] = code)
                }
                "debounce" => {
                    let v = value.as_num().round().clamp(0., 10.) as u8;
                    self.write_mouse_options(|b| b[10] = v)
                }
                "lod" => {
                    let v = value.as_choice().clamp(1, 2) as u8;
                    self.write_mouse_options(|b| b[52] = v)
                }
                "angle_snap" => {
                    let v = value.as_bool() as u8;
                    self.write_mouse_options(|b| b[53] = v)
                }
                other => Err(HubError::Invalid(format!("unknown setting {other}"))),
            },
            Change::DpiStages { stages, active } => self.write_dpi(stages, *active),
            Change::ActiveStage(n) => {
                let cur = self
                    .mouse_read()?
                    .mouse
                    .ok_or_else(|| HubError::Protocol("no DPI table".into()))?;
                self.write_dpi(&cur.stages, *n)
            }
            Change::Button { slot, action } if *slot < MOUSE_BUTTONS.len() => {
                let mut body = [0u8; 12];
                body[0] = op::MOUSE_SET_KEY;
                body[1] = self.profile()?;
                body[2] = *slot as u8;
                body[8..12].copy_from_slice(&mouse_action_to(action)?);
                self.set(&body, Sum::Bit7)
            }
            other => Err(HubError::Invalid(format!(
                "not available on this mouse: {other:?}"
            ))),
        }
    }
}

impl DeviceDriver for RyDriver {
    fn info(&self) -> DeviceInfo {
        // The battery poll also identifies a device that has just woken up.
        let battery = self.battery();
        let known = self
            .board
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if i.firmware.is_none() && known {
            if let Ok(r) = self.get(&[op::REVISION]) {
                // Each nibble is a decimal digit: `08 05` is 5.08.
                i.firmware = Some(format!("{:x}.{:02x}", r[2], r[1]));
                self.info.lock().unwrap_or_else(|p| p.into_inner()).firmware = i.firmware.clone();
            }
        }
        if let Ok(b) = battery {
            i.battery = Some(b);
        }
        i.asleep = self.info.lock().unwrap_or_else(|p| p.into_inner()).asleep;
        i.supported = true;
        i
    }

    /// Empty until the device has said which board it is: the settings of a
    /// board that was only assumed are never shown.
    fn caps(&self) -> Capabilities {
        let board = *self.board.lock().unwrap_or_else(|p| p.into_inner());
        match board.map(|b| b.family) {
            Some(Family::Keyboard) => self.keyboard_caps(),
            Some(Family::Mouse) => self.mouse_caps(),
            None => Capabilities {
                battery: self.link.relay.is_some(),
                ..Capabilities::empty()
            },
        }
    }

    fn read(&self) -> HubResult<DeviceState> {
        match self.board()?.family {
            Family::Keyboard => self.keyboard_read(),
            Family::Mouse => self.mouse_read(),
        }
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        match self.board()?.family {
            Family::Keyboard => self.keyboard_apply(change),
            Family::Mouse => self.mouse_apply(change),
        }
    }

    /// Only a receiver reports a battery; the keyboard has no command for it
    /// on its own cable.
    fn battery(&self) -> HubResult<Battery> {
        let Some(target) = self.link.relay else {
            return Err(HubError::Unsupported("battery"));
        };
        let st = self.link.status()?;
        let (online, level) = if target == TARGET_MOUSE {
            (st.mouse_online, st.mouse_battery)
        } else {
            (st.keyboard_online, st.keyboard_battery)
        };
        self.set_asleep(!online);
        if online {
            // First contact after sleeping: find out which board this is.
            let _ = self.board();
        }
        Ok(Battery {
            percent: (level <= 100).then_some(level),
            charging: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(prefix: &[u8]) -> [u8; 64] {
        let mut f = [0u8; 64];
        f[..prefix.len()].copy_from_slice(prefix);
        f
    }

    fn info(model: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId("hid:3151:0000:test".into()),
            kind: DeviceKind::Mouse,
            name: "test".into(),
            vendor: "Fantech".into(),
            model: model.into(),
            vid: 0x3151,
            pid: 0,
            transport: Transport::Wired,
            firmware: None,
            dongle_firmware: None,
            serial: None,
            driver: DriverKind::Ry,
            battery: None,
            supported: true,
            asleep: false,
            board_id: None,
        }
    }

    // Receiver status as read from the Tanto PRO's receiver: mouse online at
    // 27 %, ready to relay; then the same with a relayed reply waiting.
    const RX_IDLE: [u8; 7] = [0x00, 0x00, 0x1b, 0x01, 0x00, 0x01, 0x02];
    const RX_READY: [u8; 7] = [0x01, 0x00, 0x1b, 0x01, 0x00, 0x01, 0x02];

    #[test]
    fn checksums_match_the_board() {
        // The keyboard echoed `8f .. 70` and `80 .. 7f`.
        assert_eq!(packet(&[0x8F], Sum::Bit7)[7], 0x70);
        assert_eq!(packet(&[0x80], Sum::Bit7)[7], 0x7F);
        assert_eq!(packet(&[op::RX_STATUS], Sum::Bit7)[7], 0x08);
        assert_eq!(packet(&[op::RX_RELEASE], Sum::Bit7)[7], 0x03);
        let led = packet(&[0x07, 1, 4, 0, 0x07, 0xBC, 0xBC, 0xBC], Sum::Bit8);
        assert_eq!(
            led[8],
            0xFF - ((0x07 + 1 + 4 + 0x07 + 0xBC * 3) & 0xFF) as u8
        );
        assert_eq!(led[7], 0xBC);
    }

    #[test]
    fn receiver_status_fields() {
        let st = parse_status(&frame(&RX_IDLE));
        assert!(st.mouse_online && !st.keyboard_online && st.can_send && !st.reply_ready);
        assert_eq!(st.mouse_battery, 27);
        assert!(parse_status(&frame(&RX_READY)).reply_ready);
    }

    #[test]
    fn default_tables_are_whole() {
        let base = default_table(MAXFIT81_BASE);
        let fn_layer = default_table(MAXFIT81_FN);
        assert_eq!((base.len(), fn_layer.len()), (96, 96));
        // 81 keys, the knob's two turns and the two ISO positions.
        assert_eq!(base.iter().filter(|v| **v != [0; 4]).count(), 85);
        for k in registry::maxfit81_layout() {
            assert_ne!(
                base[k.id as usize], [0; 4],
                "{} has no factory key",
                k.label
            );
        }
        let usage = |label: &str| {
            let k = registry::maxfit81_layout()
                .into_iter()
                .find(|k| k.label == label)
                .unwrap();
            base[k.id as usize]
        };
        assert_eq!(usage("Esc"), [0, 0, 0x29, 0]);
        assert_eq!(usage("Q"), [0, 0, 0x14, 0]);
        assert_eq!(usage("Enter"), [0, 0, 0x28, 0]);
        assert_eq!(usage("Fn"), KEY_FN.to_be_bytes());
        assert_eq!(usage("Knob"), [3, 0, 0xE2, 0]);
        assert_eq!(usage("→"), [0, 0, 0x4F, 0]);
        // Fn + F12 is volume up from the factory.
        let f12 = registry::maxfit81_layout()
            .into_iter()
            .find(|k| k.label == "F12")
            .unwrap();
        assert_eq!(fn_layer[f12.id as usize], [3, 0, 0xE9, 0]);
    }

    #[test]
    fn catalog_names_every_factory_function() {
        let layout = registry::maxfit81_layout();
        let catalog = keyboard_catalog(&layout);
        let known = |code: u32| catalog.iter().flat_map(|g| &g.keys).any(|k| k.code == code);
        for table in [MAXFIT81_BASE, MAXFIT81_FN] {
            for (slot, v) in default_table(table).iter().enumerate() {
                if *v != [0; 4] && layout.iter().any(|k| k.id as usize == slot) {
                    assert!(known(code_of(*v)), "slot {slot} value {v:02x?}");
                }
            }
        }
    }

    #[test]
    fn mouse_actions_round_trip() {
        for v in [
            [1, 0, 0xF0, 0],
            [1, 0, 0xF4, 0],
            [0x14, 0, 0, 0],
            [0, 0, 0, 0],
            [0, 0, 0x04, 0],
            [0, 0, 0xE3, 0x07],
            [0, 0xE0, 0xE1, 0x29],
            [1, 0, 0xF9, 0x01],
        ] {
            assert_eq!(
                mouse_action_to(&mouse_action_from(v)).unwrap(),
                v,
                "{v:02x?}"
            );
        }
        assert_eq!(
            mouse_action_from([0, 0, 0xE3, 0x07]),
            ButtonAction::Key(KeyStroke {
                mods: mods::LGUI,
                usage: 0x07
            })
        );
        assert!(mouse_action_to(&ButtonAction::Dpi(DpiFn::Up)).is_err());
    }

    #[test]
    fn mouse_reads_through_the_receiver() {
        let ch = Channel::mock();
        let relayed = |reply: &[u8]| {
            ch.mock_reply(&RX_IDLE);
            ch.mock_reply(&RX_READY);
            ch.mock_reply(reply);
        };
        // Captured from the mouse: id 3109, profile 0, options, DPI, keys.
        relayed(&[0x8f, 0x25, 0x0c, 0x00, 0x00, 0x05, 0x00, 0x70]);
        relayed(&[0x85, 0, 0, 0, 0, 0, 0, 0x7a]);
        let mut opt = frame(&[0xd3, 0, 0, 0, 0, 0, 0, 0x2c, 0x00, 0x01, 0x02]);
        opt[50] = 100;
        opt[51] = 100;
        opt[52] = 1;
        relayed(&opt);
        let mut dpi = frame(&[0xd4, 0x00, 0x03, 0x05, 0, 0, 0, 0x2b]);
        dpi[8..18].copy_from_slice(&[0x90, 0x01, 0x20, 0x03, 0xb0, 0x04, 0x60, 0x09, 0x80, 0x0c]);
        dpi[24..34].copy_from_slice(&[0x90, 0x01, 0x20, 0x03, 0xb0, 0x04, 0x60, 0x09, 0x80, 0x0c]);
        dpi[40..55].copy_from_slice(&[
            0xff, 0, 0, 0, 0xff, 0, 0, 0, 0xff, 0xff, 0xe6, 0, 0x64, 0x95, 0xed,
        ]);
        relayed(&dpi);
        relayed(&[
            1, 0, 0xf0, 0, 1, 0, 0xf1, 0, 1, 0, 0xf2, 0, 1, 0, 0xf4, 0, 1, 0, 0xf3, 0, 0x14, 0, 0,
            0,
        ]);

        let link = Link {
            ch,
            relay: Some(TARGET_MOUSE),
        };
        let d = build(link, info("ry-mouse-receiver"), None).unwrap();
        assert_eq!(d.board().unwrap().id, 3109);
        let st = d.read().unwrap();
        assert_eq!(st.settings["polling"], SettingValue::Choice(1000));
        assert_eq!(st.settings["debounce"], SettingValue::Num(2.));
        assert_eq!(st.settings["lod"], SettingValue::Choice(1));
        let m = st.mouse.unwrap();
        assert_eq!(
            m.stages.iter().map(|s| s.x).collect::<Vec<_>>(),
            [400, 800, 1200, 2400, 3200]
        );
        assert_eq!(m.active_stage, 3);
        assert_eq!(m.stages[4].color, Rgb::hex(0x6495ed));
        assert_eq!(m.buttons[3], ButtonAction::Mouse(MouseFn::Forward));
        assert_eq!(m.buttons[5], ButtonAction::Dpi(DpiFn::Cycle));

        // The relay selects the mouse before each forwarded packet.
        let sent = d.link.ch.mock_sent();
        assert_eq!(&sent[0][..2], &[op::RX_STATUS, 0]);
        assert_eq!(&sent[1][..2], &[op::RX_SELECT, TARGET_MOUSE]);
        assert_eq!(sent[2][0], op::IDENTIFY);
        assert_eq!(sent[4][0], op::RX_RELEASE);
    }

    #[test]
    fn a_sleeping_mouse_still_opens() {
        let ch = Channel::mock();
        // Mouse offline: byte 4 set.
        ch.mock_reply(&[0x00, 0x00, 0x1b, 0x01, 0x01, 0x01, 0x02]);
        let link = Link {
            ch,
            relay: Some(TARGET_MOUSE),
        };
        let d = build(link, info("ry-mouse-receiver"), None).unwrap();
        assert!(d.info.lock().unwrap().asleep);
        // Nothing is assumed about the device behind the receiver: no board,
        // no settings, and only the status query went out.
        assert!(d.board.lock().unwrap().is_none());
        let caps = d.caps();
        assert!(caps.mouse.is_none() && caps.settings.is_empty() && caps.battery);
        assert!(d.link.ch.mock_sent().iter().all(|p| p[0] == op::RX_STATUS));

        // It wakes up: the next read identifies it first, then reads.
        d.link.ch.mock_reply(&RX_IDLE);
        d.link.ch.mock_reply(&RX_READY);
        d.link
            .ch
            .mock_reply(&[0x8f, 0x25, 0x0c, 0x00, 0x00, 0x05, 0x00, 0x70]);
        // The read itself then fails in this script (no more replies), which
        // is beside the point: the board is known and named now.
        let _ = d.read();
        assert_eq!(d.board().unwrap().id, 3109);
        let i = d.info.lock().unwrap().clone();
        assert_eq!((i.model.as_str(), i.asleep), ("tanto-pro", false));
        assert!(d.caps().mouse.is_some());
    }

    #[test]
    fn another_product_behind_a_receiver_is_never_assumed() {
        // Awake at open with an id this driver does not know: refused, not
        // opened as the board the receiver usually ships with.
        let ch = Channel::mock();
        ch.mock_reply(&[0x00, 0x60, 0x00, 0x00, 0x01, 0x01, 0x01]);
        ch.mock_reply(&[0x01, 0x60, 0x00, 0x00, 0x01, 0x01, 0x01]);
        ch.mock_reply(&[0x8f, 0xaf, 0x07, 0x00, 0x00]);
        let link = Link {
            ch,
            relay: Some(TARGET_KEYBOARD),
        };
        let err = build(link, info("ry-keyboard-receiver"), None)
            .err()
            .unwrap();
        assert!(err.to_string().contains("1967"), "{err}");

        // Asleep at open, then it wakes up as an unknown board: every read
        // and write is refused, and only identify was ever forwarded.
        let ch = Channel::mock();
        ch.mock_reply(&[0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00]);
        let link = Link {
            ch,
            relay: Some(TARGET_KEYBOARD),
        };
        let d = build(link, info("ry-keyboard-receiver"), None).unwrap();
        assert!(d.caps().keyboard.is_none());
        for _ in 0..2 {
            d.link
                .ch
                .mock_reply(&[0x00, 0x60, 0x00, 0x00, 0x01, 0x01, 0x01]);
            d.link
                .ch
                .mock_reply(&[0x01, 0x60, 0x00, 0x00, 0x01, 0x01, 0x01]);
            d.link.ch.mock_reply(&[0x8f, 0xaf, 0x07, 0x00, 0x00]);
        }
        assert!(d.read().is_err());
        let write = d.apply(&Change::Setting {
            id: "debounce".into(),
            value: SettingValue::Num(3.),
        });
        assert!(write.is_err());
        let forwarded: Vec<u8> = d
            .link
            .ch
            .mock_sent()
            .iter()
            .map(|p| p[0])
            .filter(|op| !matches!(*op, op::RX_STATUS | op::RX_SELECT | op::RX_RELEASE))
            .collect();
        assert_eq!(forwarded, [op::IDENTIFY, op::IDENTIFY]);
        assert!(d.caps().keyboard.is_none());
    }

    #[test]
    fn the_mouse_on_its_cable_needs_no_relay() {
        let ch = Channel::mock();
        // Identify as answered on the cable (byte 5 is 6 there, 5 by radio).
        ch.mock_reply(&[0x8f, 0x25, 0x0c, 0x00, 0x00, 0x06, 0x00, 0x70]);
        let d = build(Link { ch, relay: None }, info("tanto-pro"), None).unwrap();
        assert_eq!(d.board().unwrap().id, 3109);
        // One report out, straight to the mouse: no status, select or release.
        let sent = d.link.ch.mock_sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0][0], op::IDENTIFY);
        // No receiver means no battery reading.
        assert!(!d.caps().battery);
        assert!(d.battery().is_err());
    }

    #[test]
    fn an_unknown_board_is_refused() {
        let ch = Channel::mock();
        ch.mock_reply(&[0x8f, 0x39, 0x05, 0x00, 0x00]);
        let link = Link { ch, relay: None };
        let err = build(link, info("maxfit81"), None).err().unwrap();
        assert!(err.to_string().contains("1337"), "{err}");
    }

    #[test]
    fn keyboard_writes() {
        let ch = Channel::mock();
        ch.mock_reply(&[0x8f, 0xb6, 0x02, 0x00, 0x00]);
        let d = build(Link { ch, relay: None }, info("maxfit81"), None).unwrap();
        let caps = d.caps();
        // Nothing that touches lighting, options, sleep or profiles is offered.
        assert!(caps.lighting.is_empty() && caps.profiles.is_none());
        assert_eq!(
            caps.settings
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["auto_os", "debounce"]
        );

        d.apply(&Change::Setting {
            id: "debounce".into(),
            value: SettingValue::Num(3.),
        })
        .unwrap();
        assert_eq!(
            &d.link.ch.mock_sent().pop().unwrap()[..3],
            &[op::SET_DEBOUNCE, 0, 3]
        );

        // A key write carries the slot and the four value bytes at 8.
        d.link.ch.mock_reply(&[0x85, 0x01]);
        let q = registry::maxfit81_layout()
            .iter()
            .position(|k| k.label == "Q")
            .unwrap();
        d.apply(&Change::KeyMap {
            layer: 1,
            key: q,
            action: KeyAction::Code(0x0000_2900),
        })
        .unwrap();
        let key = d.link.ch.mock_sent().pop().unwrap();
        assert_eq!(&key[..3], &[op::SET_FN_KEY, 1, 8]);
        assert_eq!(&key[8..12], &[0, 0, 0x29, 0]);

        // A profile switch is refused rather than sent.
        let before = d.link.ch.mock_sent().len();
        assert!(d.apply(&Change::Profile(1)).is_err());
        assert_eq!(d.link.ch.mock_sent().len(), before);
    }
}
