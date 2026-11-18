//! EarFun earbuds on Qualcomm chips: GAIA frames over Bluetooth RFCOMM (SPP).
//!
//! ```text
//! FF | 04 | flags | length | vendor (2) | command (2) | payload
//! ```
//!
//! EarFun's settings live on vendor `000A` as commands `03xx`. A reply repeats
//! the command with the top bit set (`83xx`) and starts with a status byte;
//! frames the earbuds send on their own (`03xx`) are skipped while waiting.
//!
//! Only the Air Pro 4+ (model code TW700) is registered. The command numbers
//! and value tables come from published protocol notes (see
//! `docs/PROTOCOL_NOTES.md`); no unit was available, so nothing here has been
//! exercised against hardware yet. The EQ is left out for that reason: it
//! cannot be read back, so a wrong frame would fail silently.

use crate::bt;
use devhub_core::backend::*;
use devhub_core::model::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const VENDOR: u16 = 0x000A;

mod cmd {
    pub const BATTERY_LEFT: u8 = 0x06;
    pub const BATTERY_RIGHT: u8 = 0x07;
    pub const RESTART: u8 = 0x08;
    pub const FIRMWARE: u8 = 0x09;
    pub const SET_KEY: u8 = 0x0A;
    pub const GET_KEYS: u8 = 0x0B;
    pub const SET_IN_EAR: u8 = 0x49;
    pub const GET_IN_EAR: u8 = 0x4A;
    pub const CONTROL: u8 = 0x51;
}

/// Settings that are one byte, read with one command and written with another.
/// `(id, read, write)`.
const BYTES: &[(&str, u8, u8)] = &[
    ("noise_mode", 0x15, 0x14),
    ("anc_mode", 0x3B, 0x3A),
    ("ambient_mode", 0x3D, 0x3C),
    ("game_mode", 0x13, 0x12),
    ("multipoint", 0x26, 0x27),
    ("codec", 0x2F, 0x2E),
    ("mic", 0x31, 0x30),
    ("prompt_volume", 0x39, 0x38),
    ("volume_limit", 0x54, 0x53),
    ("balance", 0x56, 0x55),
    ("auto_off", 0x48, 0x47),
    ("touch_lock", 0x35, 0x34),
    ("find", 0x33, 0x32),
];

/// Codec bits of `codec`: aptX, aptX Adaptive, LDAC, aptX Lossless.
const CODEC_LDAC: i64 = 0x08;

/// Touch gestures in the order the earbuds report them: left then right for
/// each of one, two, three taps and press-and-hold. `(id, label, gesture)`.
const GESTURES: &[(&str, &str, u8)] = &[
    ("tap1", "Single tap", 1),
    ("tap2", "Double tap", 2),
    ("tap3", "Triple tap", 3),
    ("hold", "Press and hold", 17),
];

// ==================================================================== frames

fn encode(vendor: u16, command: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0xFF, 0x04, 0x00, payload.len() as u8];
    f.extend_from_slice(&vendor.to_be_bytes());
    f.extend_from_slice(&command.to_be_bytes());
    f.extend_from_slice(payload);
    f
}

#[derive(Debug, PartialEq, Eq)]
struct Frame {
    vendor: u16,
    command: u16,
    payload: Vec<u8>,
}

/// Incremental decoder: replies arrive split or glued together.
#[derive(Default)]
struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            // Resynchronise on the start byte and a plausible header.
            match self.buf.iter().position(|b| *b == 0xFF) {
                Some(0) => {}
                Some(n) => {
                    self.buf.drain(..n);
                }
                None => {
                    self.buf.clear();
                    break;
                }
            }
            if self.buf.len() < 8 {
                break;
            }
            if !matches!(self.buf[1], 3 | 4) || self.buf[2] & !1 != 0 {
                self.buf.remove(0);
                continue;
            }
            let checksum = (self.buf[2] & 1) as usize;
            let total = 8 + self.buf[3] as usize + checksum;
            if self.buf.len() < total {
                break;
            }
            let frame: Vec<u8> = self.buf.drain(..total).collect();
            if checksum == 1 && frame.iter().fold(0u8, |a, b| a ^ b) != 0 {
                continue;
            }
            out.push(Frame {
                vendor: u16::from_be_bytes([frame[4], frame[5]]),
                command: u16::from_be_bytes([frame[6], frame[7]]),
                payload: frame[8..total - checksum].to_vec(),
            });
        }
        out
    }
}

// ==================================================================== session

/// The byte stream under the frames. `bt::Rfcomm` in the app, a script in tests.
trait Stream: Send {
    fn send(&self, data: &[u8]) -> HubResult<()>;
    fn recv(&self, buf: &mut [u8], timeout: Duration) -> HubResult<usize>;
}

impl Stream for bt::Rfcomm {
    fn send(&self, data: &[u8]) -> HubResult<()> {
        bt::Rfcomm::send(self, data)
    }
    fn recv(&self, buf: &mut [u8], timeout: Duration) -> HubResult<usize> {
        bt::Rfcomm::recv(self, buf, timeout)
    }
}

struct Session {
    stream: Box<dyn Stream>,
    decoder: Decoder,
}

impl Session {
    /// Send one command and wait for its reply. Returns the bytes after the
    /// status byte, or `None` when nothing answered in time.
    fn exchange(&mut self, code: u8, payload: &[u8]) -> HubResult<Option<Vec<u8>>> {
        // The earbuds take one command at a time.
        std::thread::sleep(Duration::from_millis(50));
        self.stream
            .send(&encode(VENDOR, 0x0300 | code as u16, payload))?;
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut buf = [0u8; 512];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let n = self
                .stream
                .recv(&mut buf, left.min(Duration::from_millis(200)))?;
            for f in self.decoder.feed(&buf[..n]) {
                if f.vendor == VENDOR && f.command == 0x8300 | code as u16 {
                    // Status 0 is success; anything else carries no value.
                    return Ok(match f.payload.split_first() {
                        Some((0, value)) => Some(value.to_vec()),
                        _ => None,
                    });
                }
            }
        }
    }

    fn read_byte(&mut self, code: u8) -> HubResult<Option<u8>> {
        Ok(self.exchange(code, &[])?.and_then(|v| v.first().copied()))
    }
}

// ==================================================================== driver

pub struct EarfunDriver {
    session: Mutex<Session>,
    info: Mutex<DeviceInfo>,
}

/// Connect to a paired, powered-on device.
pub fn open(addr: u64, info: DeviceInfo) -> HubResult<EarfunDriver> {
    let link = bt::Rfcomm::connect(addr, bt::UUID_SPP, Duration::from_secs(6))?;
    Ok(build(Box::new(link), info))
}

fn build(stream: Box<dyn Stream>, info: DeviceInfo) -> EarfunDriver {
    EarfunDriver {
        session: Mutex::new(Session {
            stream,
            decoder: Decoder::default(),
        }),
        info: Mutex::new(info),
    }
}

fn opts(list: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    list.iter().map(|(v, l)| ChoiceOpt::new(*v, *l)).collect()
}

/// What a gesture can do. The last two are only offered on three taps and on
/// press-and-hold.
fn gesture_actions(extended: bool) -> Vec<ChoiceOpt> {
    let mut list = vec![
        (1, "Play / pause"),
        (2, "Volume up"),
        (3, "Volume down"),
        (4, "Previous track"),
        (5, "Next track"),
        (6, "Voice assistant"),
        (8, "Noise control"),
    ];
    if extended {
        list.extend([
            (7, "Game mode"),
            (9, "Noise cancelling off"),
            (13, "Call back"),
        ]);
    }
    list.push((16, "None"));
    opts(&list)
}

fn gesture_id(base: &str, right: bool) -> String {
    format!("{base}_{}", if right { "right" } else { "left" })
}

impl EarfunDriver {
    fn session(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self, code: u8, payload: &[u8]) -> HubResult<()> {
        // A missing acknowledgement is not an error: the UI reads the value
        // back, which is the check that counts.
        self.session().exchange(code, payload).map(|_| ())
    }

    fn read_setting(&self, id: &str) -> HubResult<Option<u8>> {
        let (_, get, _) = BYTES
            .iter()
            .find(|b| b.0 == id)
            .ok_or_else(|| HubError::Invalid(format!("unknown setting {id}")))?;
        self.session().read_byte(*get)
    }
}

impl DeviceDriver for EarfunDriver {
    fn info(&self) -> DeviceInfo {
        let mut i = self.info.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if i.firmware.is_none() {
            if let Ok(Some(v)) = self.session().exchange(cmd::FIRMWARE, &[]) {
                let text: String = v
                    .iter()
                    .filter(|b| b.is_ascii_graphic() || **b == b' ')
                    .map(|b| *b as char)
                    .collect();
                if !text.is_empty() {
                    i.firmware = Some(text);
                    self.info.lock().unwrap_or_else(|p| p.into_inner()).firmware =
                        i.firmware.clone();
                }
            }
        }
        if let Ok(b) = self.battery() {
            i.battery = Some(b);
        }
        i.supported = true;
        i
    }

    fn caps(&self) -> Capabilities {
        let noise = |v: i64| SettingValue::Choice(v);
        let restart = "The earbuds restart to apply this and reconnect after about 20 seconds.";
        let mut settings = vec![
            SettingSpec::choice(
                "noise_mode",
                section::AUDIO,
                "Noise control",
                opts(&[(0, "Off"), (1, "Noise cancelling"), (2, "Ambient sound")]),
            ),
            SettingSpec::choice(
                "anc_mode",
                section::AUDIO,
                "Noise cancelling mode",
                opts(&[
                    (0, "Adapts to your ears"),
                    (4, "Adaptive"),
                    (6, "Level 1"),
                    (5, "Level 2"),
                    (3, "Level 3"),
                    (2, "Level 4"),
                    (1, "Wind noise"),
                ]),
            )
            .when("noise_mode", noise(1)),
            SettingSpec::choice(
                "ambient_mode",
                section::AUDIO,
                "Ambient sound mode",
                opts(&[(0, "Default"), (1, "Full")]),
            )
            .when("noise_mode", noise(2)),
            SettingSpec::toggle("game_mode", section::AUDIO, "Game mode (low latency)"),
            SettingSpec::choice(
                "mic",
                section::AUDIO,
                "Microphone",
                opts(&[(0, "Automatic"), (1, "Left earbud"), (2, "Right earbud")]),
            ),
            SettingSpec::choice(
                "prompt_volume",
                section::AUDIO,
                "Voice prompt volume",
                opts(&[
                    (0, "100 %"),
                    (1, "75 %"),
                    (2, "50 %"),
                    (3, "25 %"),
                    (4, "Off"),
                ]),
            ),
            SettingSpec::choice(
                "volume_limit",
                section::AUDIO,
                "Volume limit",
                opts(&[
                    (0, "Off"),
                    (1, "90 dB"),
                    (2, "80 dB"),
                    (3, "70 dB"),
                    (4, "60 dB"),
                    (5, "50 dB"),
                ]),
            ),
            SettingSpec::slider(
                "balance",
                section::AUDIO,
                "Left / right balance",
                0.,
                100.,
                1.,
                "",
            )
            .help("50 is the middle."),
            SettingSpec::toggle("multipoint", section::CONNECTION, "Connect two devices")
                .help("Cannot be combined with LDAC.")
                .confirm(restart),
            SettingSpec::choice(
                "codec",
                section::CONNECTION,
                "Sound quality",
                opts(&[
                    (0, "Stable connection"),
                    (1, "aptX"),
                    (3, "aptX Adaptive"),
                    (19, "aptX Lossless"),
                    (CODEC_LDAC, "LDAC"),
                ]),
            )
            .confirm(restart),
            SettingSpec::choice(
                "auto_off",
                section::POWER,
                "Power off when idle",
                opts(&[
                    (1, "5 min"),
                    (2, "10 min"),
                    (3, "30 min"),
                    (4, "60 min"),
                    (5, "120 min"),
                    (255, "Never"),
                ]),
            ),
            SettingSpec::toggle(
                "auto_pause",
                section::POWER,
                "Pause when an earbud is removed",
            ),
            SettingSpec::toggle(
                "auto_answer",
                section::POWER,
                "Answer calls when an earbud is put in",
            ),
            SettingSpec::choice(
                "touch_lock",
                section::CONTROLS,
                "Turn touch controls off",
                opts(&[
                    (0, "On both earbuds"),
                    (1, "Off on both"),
                    (2, "Off on the left"),
                    (3, "Off on the right"),
                ]),
            ),
        ];
        for (base, label, gesture) in GESTURES {
            for right in [false, true] {
                settings.push(SettingSpec::choice(
                    &gesture_id(base, right),
                    section::CONTROLS,
                    &format!("{label} ({})", if right { "right" } else { "left" }),
                    gesture_actions(*gesture == 3 || *gesture == 17),
                ));
            }
        }
        settings.push(
            SettingSpec::choice(
                "find",
                section::SYSTEM,
                "Find the earbuds",
                opts(&[
                    (0, "Off"),
                    (1, "Ring left"),
                    (2, "Ring right"),
                    (3, "Ring both"),
                ]),
            )
            .confirm("This plays a loud sound. Take the earbuds out of your ears first."),
        );
        Capabilities {
            settings,
            actions: vec![
                ActionSpec::new("restart", section::SYSTEM, "Restart the earbuds")
                    .confirm("The earbuds restart and reconnect after about 20 seconds."),
                ActionSpec::new("power_off", section::SYSTEM, "Power off")
                    .danger("Turn the earbuds off? Put them in the case to turn them on again."),
            ],
            profiles: None,
            mouse: None,
            keyboard: None,
            lighting: vec![],
            battery: true,
            peq: None,
            firmware_feed: None,
        }
    }

    fn read(&self) -> HubResult<DeviceState> {
        let mut settings = BTreeMap::new();
        let mut s = self.session();
        for (id, get, _) in BYTES {
            let Some(v) = s.read_byte(*get)? else {
                continue;
            };
            let value = match *id {
                "game_mode" | "multipoint" => SettingValue::Bool(v == 1),
                "balance" => SettingValue::Num(v as f32),
                _ => SettingValue::Choice(v as i64),
            };
            settings.insert((*id).to_string(), value);
        }
        // Bit 0 set turns pausing off, bit 1 answering calls.
        if let Some(bits) = s.read_byte(cmd::GET_IN_EAR)? {
            settings.insert("auto_pause".into(), SettingValue::Bool(bits & 1 == 0));
            settings.insert("auto_answer".into(), SettingValue::Bool(bits & 2 == 0));
        }
        // Sixteen-bit entries, the action in the low byte: left and right for
        // each gesture in `GESTURES` order.
        if let Some(keys) = s.exchange(cmd::GET_KEYS, &[])? {
            for (n, (base, ..)) in GESTURES.iter().enumerate() {
                for right in [false, true] {
                    if let Some(action) = keys.get((n * 2 + right as usize) * 2 + 1) {
                        settings.insert(
                            gesture_id(base, right),
                            SettingValue::Choice(*action as i64),
                        );
                    }
                }
            }
        }
        Ok(DeviceState {
            profile: ProfileState {
                active: 0,
                names: vec![],
            },
            settings,
            mouse: None,
            keyboard: None,
            lighting: Default::default(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        let Change::Setting { id, value } = change else {
            return Err(HubError::Invalid(format!(
                "not available on these earbuds: {change:?}"
            )));
        };
        match id.as_str() {
            "auto_pause" | "auto_answer" => {
                let bits = self
                    .session()
                    .read_byte(cmd::GET_IN_EAR)?
                    .ok_or_else(|| HubError::Protocol("in-ear setting did not answer".into()))?;
                let mask = if id == "auto_pause" { 1 } else { 2 };
                // The other bits are written back as read.
                let bits = if value.as_bool() {
                    bits & !mask
                } else {
                    bits | mask
                };
                self.write(cmd::SET_IN_EAR, &[bits])
            }
            // LDAC and two devices exclude each other; the earbuds' own app
            // refuses the second instead of switching the first off.
            "multipoint" if value.as_bool() => {
                if self
                    .read_setting("codec")?
                    .is_some_and(|c| c as i64 & CODEC_LDAC != 0)
                {
                    return Err(HubError::Invalid(
                        "turn LDAC off before connecting two devices".into(),
                    ));
                }
                self.write(0x27, &[1])?;
                // The earbuds' app restarts them three seconds after this.
                std::thread::sleep(Duration::from_secs(3));
                self.write(cmd::RESTART, &[])
            }
            "codec" => {
                let bits = value.as_choice();
                if bits & CODEC_LDAC != 0 && self.read_setting("multipoint")? == Some(1) {
                    return Err(HubError::Invalid(
                        "turn off the second device connection before choosing LDAC".into(),
                    ));
                }
                self.write(0x2E, &[bits as u8])?;
                self.write(cmd::RESTART, &[])
            }
            _ => {
                if let Some((_, _, set)) = BYTES.iter().find(|b| b.0 == id) {
                    let byte = match value {
                        SettingValue::Bool(b) => *b as u8,
                        SettingValue::Num(n) => n.round().clamp(0., 100.) as u8,
                        SettingValue::Choice(c) => (*c).clamp(0, 255) as u8,
                    };
                    return self.write(*set, &[byte]);
                }
                for (base, _, gesture) in GESTURES {
                    for right in [false, true] {
                        if *id == gesture_id(base, right) {
                            let action = value.as_choice().clamp(0, 255) as u8;
                            let payload = if right {
                                [0, 0, *gesture, action]
                            } else {
                                [*gesture, action, 0, 0]
                            };
                            return self.write(cmd::SET_KEY, &payload);
                        }
                    }
                }
                Err(HubError::Invalid(format!("unknown setting {id}")))
            }
        }
    }

    fn run_action(&self, id: &str, _file: Option<&std::path::Path>) -> HubResult<ActionOutcome> {
        match id {
            "restart" => self.write(cmd::RESTART, &[])?,
            "power_off" => self.write(cmd::CONTROL, &[1])?,
            _ => return Err(HubError::Unsupported("action")),
        }
        Ok(ActionOutcome {
            message: None,
            reload: false,
        })
    }

    /// The lower of the two earbuds. The case is not reported: the contract
    /// has one battery per device.
    fn battery(&self) -> HubResult<Battery> {
        let mut s = self.session();
        let left = s.read_byte(cmd::BATTERY_LEFT)?;
        let right = s.read_byte(cmd::BATTERY_RIGHT)?;
        let percent = [left, right]
            .into_iter()
            .flatten()
            .filter(|v| *v <= 100)
            .min();
        Ok(Battery {
            percent,
            charging: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Arc;

    /// Answers each request from a table of `command code -> value bytes`.
    #[derive(Default)]
    struct Script {
        values: Mutex<BTreeMap<u8, Vec<u8>>>,
        sent: Mutex<Vec<Vec<u8>>>,
        pending: Mutex<VecDeque<u8>>,
    }

    struct Fake(Arc<Script>);

    impl Stream for Fake {
        fn send(&self, data: &[u8]) -> HubResult<()> {
            self.0.sent.lock().unwrap().push(data.to_vec());
            let code = data[7];
            let mut reply = Vec::new();
            // An unprompted report first: the reader has to skip it.
            reply.extend(encode(VENDOR, 0x034D, &[0, 2]));
            // A value the script holds answers with status 0; anything else is
            // refused with a bare non-zero status, as an unsupported command is.
            let payload = match self.0.values.lock().unwrap().get(&code) {
                Some(v) => [&[0u8][..], v].concat(),
                None => vec![1u8],
            };
            // Earbuds answer with protocol version 3.
            let mut f = encode(VENDOR, 0x8300 | code as u16, &payload);
            f[1] = 3;
            reply.extend(f);
            self.0.pending.lock().unwrap().extend(reply);
            Ok(())
        }
        fn recv(&self, buf: &mut [u8], _timeout: Duration) -> HubResult<usize> {
            // Hand the reply over three bytes at a time to exercise reassembly.
            let mut p = self.0.pending.lock().unwrap();
            let n = p.len().min(3).min(buf.len());
            for b in buf.iter_mut().take(n) {
                *b = p.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId("bt:000000000001".into()),
            kind: DeviceKind::Audio,
            name: "EarFun Air Pro 4+".into(),
            vendor: "EarFun".into(),
            model: "earfun-tw700".into(),
            vid: 0,
            pid: 0,
            transport: Transport::Bluetooth,
            firmware: None,
            dongle_firmware: None,
            serial: None,
            driver: DriverKind::Earfun,
            battery: None,
            supported: true,
            asleep: false,
            board_id: None,
        }
    }

    fn driver(values: &[(u8, &[u8])]) -> (EarfunDriver, Arc<Script>) {
        let script = Arc::new(Script::default());
        for (code, v) in values {
            script.values.lock().unwrap().insert(*code, v.to_vec());
        }
        (build(Box::new(Fake(script.clone())), info()), script)
    }

    #[test]
    fn frames_match_a_captured_exchange() {
        // Firmware query and reply as recorded from an Air Pro 4.
        assert_eq!(
            encode(VENDOR, 0x0309, &[]),
            [0xFF, 0x04, 0x00, 0x00, 0x00, 0x0A, 0x03, 0x09]
        );
        let mut d = Decoder::default();
        let reply = [0xFF, 0x03, 0x00, 0x02, 0x00, 0x0A, 0x83, 0x06, 0x00, 0x5A];
        assert!(d.feed(&reply[..4]).is_empty());
        let frames = d.feed(&reply[4..]);
        assert_eq!(
            frames,
            [Frame {
                vendor: VENDOR,
                command: 0x8306,
                payload: vec![0x00, 0x5A]
            }]
        );
        // Noise between frames is dropped.
        let mut glued = vec![0x00, 0x12];
        glued.extend(reply);
        glued.extend(reply);
        assert_eq!(d.feed(&glued).len(), 2);
    }

    #[test]
    fn reads_settings_and_gestures() {
        let keys: &[u8] = &[0, 1, 0, 1, 0, 5, 0, 4, 0, 6, 0, 7, 0, 8, 0, 16];
        let (d, _) = driver(&[
            (0x15, &[1]),
            (0x3B, &[6]),
            (0x13, &[1]),
            (0x26, &[0]),
            (0x2F, &[19]),
            (0x56, &[50]),
            (0x48, &[255]),
            (cmd::GET_IN_EAR, &[0x02]),
            (cmd::GET_KEYS, keys),
            (cmd::BATTERY_LEFT, &[90]),
            (cmd::BATTERY_RIGHT, &[70]),
            (cmd::FIRMWARE, b"TW700_v1.2.3"),
        ]);
        let st = d.read().unwrap();
        assert_eq!(st.settings["noise_mode"], SettingValue::Choice(1));
        assert_eq!(st.settings["anc_mode"], SettingValue::Choice(6));
        assert_eq!(st.settings["game_mode"], SettingValue::Bool(true));
        assert_eq!(st.settings["multipoint"], SettingValue::Bool(false));
        assert_eq!(st.settings["codec"], SettingValue::Choice(19));
        assert_eq!(st.settings["balance"], SettingValue::Num(50.));
        assert_eq!(st.settings["auto_off"], SettingValue::Choice(255));
        assert_eq!(st.settings["auto_pause"], SettingValue::Bool(true));
        assert_eq!(st.settings["auto_answer"], SettingValue::Bool(false));
        assert_eq!(st.settings["tap2_left"], SettingValue::Choice(5));
        assert_eq!(st.settings["hold_right"], SettingValue::Choice(16));
        // A setting the earbuds did not answer is left out, not guessed.
        assert!(!st.settings.contains_key("mic"));

        assert_eq!(d.battery().unwrap().percent, Some(70));
        assert_eq!(d.info().firmware.as_deref(), Some("TW700_v1.2.3"));
        // Every declared choice value the earbuds reported is a listed option.
        let caps = d.caps();
        for (id, v) in &st.settings {
            if let (Some(spec), SettingValue::Choice(c)) = (caps.setting(id), v) {
                let SettingKind::Choice { options } = &spec.kind else {
                    panic!("{id} is not a choice");
                };
                assert!(options.iter().any(|o| o.value == *c), "{id}={c}");
            }
        }
    }

    #[test]
    fn writes_use_the_documented_payloads() {
        let (d, script) = driver(&[(cmd::GET_IN_EAR, &[0x0C]), (0x2F, &[0x08]), (0x26, &[0])]);
        let set = |id: &str, value: SettingValue| {
            d.apply(&Change::Setting {
                id: id.into(),
                value,
            })
        };
        let last = || script.sent.lock().unwrap().last().unwrap()[6..].to_vec();

        set("noise_mode", SettingValue::Choice(2)).unwrap();
        assert_eq!(last(), [0x03, 0x14, 2]);
        set("tap3_right", SettingValue::Choice(13)).unwrap();
        assert_eq!(last(), [0x03, cmd::SET_KEY, 0, 0, 3, 13]);
        set("hold_left", SettingValue::Choice(8)).unwrap();
        assert_eq!(last(), [0x03, cmd::SET_KEY, 17, 8, 0, 0]);
        // Bits 2 and 3 are kept; bit 0 turns pausing off.
        set("auto_pause", SettingValue::Bool(false)).unwrap();
        assert_eq!(last(), [0x03, cmd::SET_IN_EAR, 0x0D]);
        // LDAC is on in this script, so a second device is refused unsent.
        let before = script.sent.lock().unwrap().len();
        assert!(set("multipoint", SettingValue::Bool(true)).is_err());
        assert!(script.sent.lock().unwrap()[before..]
            .iter()
            .all(|f| f[7] == 0x2F));
        // A codec change is followed by the restart command.
        set("codec", SettingValue::Choice(3)).unwrap();
        let sent = script.sent.lock().unwrap();
        assert_eq!(sent[sent.len() - 2][6..], [0x03, 0x2E, 3]);
        assert_eq!(sent[sent.len() - 1][6..], [0x03, cmd::RESTART]);
    }
}
