//! Demo backend — drives every screen of the UI without real hardware.
//!
//! **Also a sample for driver implementers**: `mouse_caps` / `keyboard_caps` declare
//! the features of the web versions of mouse.fit (G-Wolves HTS Plus Pro) and hub.xsyd.top (SU66 Max)
//! as they are. A real driver only needs to return caps of the same shape
//! (confirm value ranges and options on the real device / JS bundle and substitute them).

use crate::backend::*;
use crate::keys;
use crate::model::*;
use crate::registry;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct DemoHub {
    drivers: Vec<Arc<DemoDriver>>,
    unsupported: DeviceInfo,
}

impl DemoHub {
    pub fn new() -> Self {
        let mut drivers = vec![
            Arc::new(DemoDriver::mouse(
                "demo:hts-plus-pro",
                "HTS Plus Pro",
                "hts-plus-pro",
                0x5219,
                Transport::Wireless24G,
                Some(87),
                false,
            )),
            Arc::new(DemoDriver::keyboard()),
            Arc::new(DemoDriver::mouse(
                "demo:htx-ultra",
                "HTX Ultra",
                "htx-ultra",
                0x5608,
                Transport::Wired,
                None,
                false,
            )),
            Arc::new(DemoDriver::dac()),
            Arc::new(DemoDriver::lamzu()),
        ];
        drivers.shrink_to_fit();
        Self {
            drivers,
            unsupported: DeviceInfo {
                id: DeviceId("demo:atk-f1".into()),
                kind: DeviceKind::Mouse,
                name: "ATK F1 Ultimate".into(),
                vendor: "ATK".into(),
                model: "atk-f1-ultimate".into(),
                vid: 0x373B,
                pid: 0x1000,
                transport: Transport::Wireless24G,
                firmware: None,
                dongle_firmware: None,
                serial: None,
                driver: DriverKind::Unsupported,
                battery: None,
                supported: false,
                asleep: false,
                board_id: None,
            },
        }
    }
}

impl Default for DemoHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Destination of notifications received via watch (the demo has one hub per process, so a static is enough).
type Sink = Box<dyn Fn(HubEvent) + Send + Sync>;
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

fn emit(ev: HubEvent) {
    if let Some(f) = SINK.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        f(ev);
    }
}

impl HubBackend for DemoHub {
    fn watch(&self, cb: Box<dyn Fn(HubEvent) + Send + Sync>) {
        *SINK.lock().unwrap_or_else(|p| p.into_inner()) = Some(cb);
    }

    fn scan(&self) -> HubResult<Vec<DeviceInfo>> {
        let mut v: Vec<DeviceInfo> = self.drivers.iter().map(|d| d.info()).collect();
        v.push(self.unsupported.clone());
        Ok(v)
    }

    fn driver(&self, id: &DeviceId) -> HubResult<Arc<dyn DeviceDriver>> {
        if *id == self.unsupported.id {
            return Err(HubError::NotImplemented("ATK F1 Ultimate protocol"));
        }
        self.drivers
            .iter()
            .find(|d| d.info.id == *id)
            .map(|d| d.clone() as Arc<dyn DeviceDriver>)
            .ok_or_else(|| HubError::NotFound(id.clone()))
    }

    fn is_demo(&self) -> bool {
        true
    }
}

// ====================================================================== Driver

pub struct DemoDriver {
    info: DeviceInfo,
    caps: Capabilities,
    state: Mutex<DeviceState>,
    /// Saved state per profile (onboard profile) — as on a real device,
    /// switching the profile swaps out the whole contents of the settings.
    saved: Mutex<std::collections::BTreeMap<u8, DeviceState>>,
    /// Initial value when a profile that has never been touched is opened.
    factory: DeviceState,
    born: Instant,
}

impl DemoDriver {
    fn mouse(
        id: &str,
        name: &str,
        model: &str,
        pid: u16,
        transport: Transport,
        battery: Option<u8>,
        magnetic: bool,
    ) -> Self {
        let caps = mouse_caps(transport == Transport::Wireless24G, magnetic);
        let state = mouse_state(&caps);
        Self {
            info: DeviceInfo {
                id: DeviceId(id.into()),
                kind: DeviceKind::Mouse,
                name: name.into(),
                vendor: "G-Wolves".into(),
                model: model.into(),
                vid: 0x33E4,
                pid,
                transport,
                firmware: Some("2.05".into()),
                dongle_firmware: (transport == Transport::Wireless24G).then(|| "1.12".into()),
                serial: None,
                driver: DriverKind::Mousefit,
                battery: battery.map(|p| Battery {
                    percent: Some(p),
                    charging: false,
                }),
                supported: true,
                asleep: false,
                board_id: None,
            },
            caps,
            state: Mutex::new(state.clone()),
            saved: Mutex::new(Default::default()),
            factory: state,
            born: Instant::now(),
        }
    }

    fn keyboard() -> Self {
        let caps = keyboard_caps();
        let state = keyboard_state(&caps);
        Self {
            info: DeviceInfo {
                id: DeviceId("demo:su66max".into()),
                kind: DeviceKind::Keyboard,
                name: "SU66 Max".into(),
                vendor: "XSYD".into(),
                model: "su66max".into(),
                vid: 0x1CA6,
                pid: 0x3009,
                transport: Transport::Wired,
                firmware: Some("1.4.2".into()),
                dongle_firmware: None,
                serial: Some("49142".into()),
                driver: DriverKind::Xsyd,
                battery: None,
                supported: true,
                asleep: false,
                board_id: None,
            },
            caps,
            state: Mutex::new(state.clone()),
            saved: Mutex::new(Default::default()),
            factory: state,
            born: Instant::now(),
        }
    }

    /// A USB DAC equivalent to peq.szwalkplay.com (WalkPlay TP55).
    /// LAMZU MAYA X V2 (via the 8K receiver) — the same caps as the official Aurora (`crate::lamzu`).
    fn lamzu() -> Self {
        let model = &crate::lamzu::MODELS[0];
        let caps = crate::lamzu::caps(model, Transport::Wireless24G, 4);
        let state = lamzu_state(&caps);
        Self {
            info: DeviceInfo {
                id: DeviceId("demo:maya-x-v2".into()),
                kind: DeviceKind::Mouse,
                name: model.name.into(),
                vendor: "LAMZU".into(),
                model: model.model.into(),
                vid: 0x37B0,
                pid: 0x0048,
                transport: Transport::Wireless24G,
                firmware: Some("0.0.3.0".into()),
                dongle_firmware: Some("0.0.3.0".into()),
                serial: None,
                driver: DriverKind::Lamzu,
                battery: Some(Battery {
                    percent: Some(100),
                    charging: false,
                }),
                supported: true,
                asleep: false,
                board_id: None,
            },
            state: Mutex::new(state.clone()),
            saved: Mutex::new(Default::default()),
            factory: state,
            caps,
            born: Instant::now(),
        }
    }

    fn dac() -> Self {
        let caps = dac_caps();
        Self {
            info: DeviceInfo {
                id: DeviceId("demo:tp55".into()),
                kind: DeviceKind::Other,
                name: "TP55".into(),
                vendor: "WalkPlay".into(),
                model: "tp55".into(),
                vid: 0x3302,
                pid: 0x4383,
                transport: Transport::Wired,
                firmware: Some("1.4".into()),
                dongle_firmware: None,
                serial: None,
                driver: DriverKind::Walkplay,
                battery: None,
                supported: true,
                asleep: false,
                board_id: None,
            },
            state: Mutex::new(dac_state(&caps)),
            saved: Mutex::new(Default::default()),
            factory: dac_state(&caps),
            caps,
            born: Instant::now(),
        }
    }
}

impl DeviceDriver for DemoDriver {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }

    fn read(&self) -> HubResult<DeviceState> {
        std::thread::sleep(std::time::Duration::from_millis(120));
        Ok(self.state.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        // A delay that feels like a real-device HID round trip.
        std::thread::sleep(std::time::Duration::from_millis(12));
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        match change.clone() {
            Change::Setting { id, value } => {
                if self.caps.setting(&id).is_none() {
                    return Err(HubError::Invalid(format!("unknown setting {id}")));
                }
                s.settings.insert(id, value);
            }
            Change::Profile(p) => {
                if p != s.profile.active {
                    // As on a real device, the contents of the settings swap per profile — save the current
                    // profile, then read out the destination (a profile not yet visited is in the factory state).
                    let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
                    saved.insert(s.profile.active, s.clone());
                    let mut next = saved
                        .get(&p)
                        .cloned()
                        .unwrap_or_else(|| self.factory.clone());
                    next.profile.names = s.profile.names.clone();
                    *s = next;
                    s.profile.active = p;
                }
            }
            Change::ProfileName { index, name } => {
                if let Some(n) = s.profile.names.get_mut(index as usize) {
                    *n = name;
                }
            }
            Change::DpiStages { stages, active } => {
                let m = s.mouse.as_mut().ok_or(HubError::Unsupported("mouse"))?;
                m.stages = stages;
                m.active_stage = active.min(m.stages.len().saturating_sub(1));
            }
            Change::ActiveStage(a) => {
                let m = s.mouse.as_mut().ok_or(HubError::Unsupported("mouse"))?;
                m.active_stage = a;
            }
            Change::Button { slot, action } => {
                let m = s.mouse.as_mut().ok_or(HubError::Unsupported("mouse"))?;
                if let Some(b) = m.buttons.get_mut(slot) {
                    *b = action;
                }
            }
            Change::Macro { slot, def } => {
                let list = if let Some(m) = s.mouse.as_mut() {
                    &mut m.macros
                } else if let Some(k) = s.keyboard.as_mut() {
                    &mut k.macros
                } else {
                    return Err(HubError::Unsupported("macro"));
                };
                if slot >= list.len() {
                    return Err(HubError::Invalid("macro slot".into()));
                }
                list[slot] = def;
            }
            Change::Lighting { area, state } => {
                s.lighting.insert(area, state);
            }
            Change::KeyMap { layer, key, action } => {
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                if let Some(slot) = k
                    .keymap
                    .get_mut(layer as usize)
                    .and_then(|l| l.get_mut(key))
                {
                    *slot = action;
                }
            }
            Change::KeyMapReset { layer } => {
                let defaults = keyboard_state(&self.caps);
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                if let (Some(dst), Some(src)) = (
                    k.keymap.get_mut(layer as usize),
                    defaults
                        .keyboard
                        .as_ref()
                        .and_then(|d| d.keymap.get(layer as usize)),
                ) {
                    *dst = src.clone();
                }
            }
            Change::KeyPerf { keys, perf } => {
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                for i in keys {
                    if let Some(p) = k.perf.get_mut(i) {
                        *p = perf;
                    }
                }
            }
            Change::Advanced {
                key,
                layer,
                binding,
            } => {
                let k = s
                    .keyboard
                    .as_mut()
                    .ok_or(HubError::Unsupported("keyboard"))?;
                k.advanced.retain(|a| !(a.key == key && a.layer == layer));
                if let Some(b) = binding {
                    k.advanced.push(AdvancedKey {
                        key,
                        layer,
                        binding: b,
                    });
                }
            }
            Change::Peq { band, value } => {
                let n = self.caps.peq.as_ref().map(|p| p.bands).unwrap_or(0) as usize;
                if let Some(peq) = s.peq.as_mut().filter(|p| band < p.len() && band < n) {
                    peq[band] = value;
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
        Ok(())
    }

    fn run_action(&self, id: &str, file: Option<&Path>) -> HubResult<ActionOutcome> {
        std::thread::sleep(std::time::Duration::from_millis(600));
        let msg = match id {
            "reset_profile" | "factory_reset" | "reset_keymap" | "reset_perf"
            | "reset_lighting" => {
                let fresh = if self.caps.keyboard.is_some() {
                    keyboard_state(&self.caps)
                } else {
                    mouse_state(&self.caps)
                };
                *self.state.lock().unwrap_or_else(|p| p.into_inner()) = fresh;
                return Ok(ActionOutcome {
                    message: Some("Reset to defaults (demo)".into()),
                    reload: true,
                });
            }
            "pair" => "Entered pairing mode (demo)".to_string(),
            "calibrate" => {
                "Calibration started. Press every key all the way down (demo)".to_string()
            }
            "calibrate_finish" => "Calibration saved (demo)".to_string(),
            "eq_def" => {
                // 8 bands of the same DEF preset as walkplay.
                const DEF_FREQS: [u16; 8] = [50, 200, 500, 1000, 2500, 5000, 10000, 15000];
                let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(peq) = st.peq.as_mut() {
                    for (i, f) in DEF_FREQS.iter().enumerate() {
                        if let Some(b) = peq.get_mut(i) {
                            *b = PeqBand {
                                freq: *f,
                                q: 0.75,
                                gain: 0.,
                                filter: PeqFilter::Peak,
                            };
                        }
                    }
                }
                return Ok(ActionOutcome {
                    message: Some("Applied the DEF preset (demo)".into()),
                    reload: true,
                });
            }
            "firmware" => {
                for i in 0..=20 {
                    emit(HubEvent::Progress {
                        id: self.info.id.clone(),
                        label: "Writing firmware (demo)".into(),
                        fraction: Some(i as f32 / 20.),
                        done: false,
                    });
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
                emit(HubEvent::Progress {
                    id: self.info.id.clone(),
                    label: String::new(),
                    fraction: None,
                    done: true,
                });
                format!(
                    "Firmware update is not performed in the demo: {}",
                    file.map(|p| p.display().to_string()).unwrap_or_default()
                )
            }
            _ => return Err(HubError::Unsupported("action")),
        };
        Ok(ActionOutcome {
            message: Some(msg),
            reload: false,
        })
    }

    fn battery(&self) -> HubResult<Battery> {
        self.info.battery.ok_or(HubError::Unsupported("battery"))
    }

    /// Returns success as a no-op, since the caps it exposes support DirectDrive (there is just no destination to send to).
    fn direct_drive(&self, _area: u8, _cells: &BTreeMap<usize, Rgb>) -> HubResult<()> {
        Ok(())
    }

    fn direct_drive_stop(&self, _area: u8) -> HubResult<()> {
        Ok(())
    }

    fn live_travel(&self) -> HubResult<Vec<f32>> {
        let n = self
            .caps
            .keyboard
            .as_ref()
            .map(|k| k.layout.len())
            .unwrap_or(0);
        let t = self.born.elapsed().as_secs_f32();
        // Pseudo data in which WASD sink rhythmically.
        Ok((0..n)
            .map(|i| {
                let phase = match i {
                    16 => 0.0, // W
                    29 => 1.3, // A
                    30 => 2.1, // S
                    31 => 2.9, // D
                    _ => return 0.0,
                };
                (((t * 2.2 + phase).sin() * 0.5 + 0.5).powf(1.6) * 4.0).min(4.0)
            })
            .collect())
    }

    fn live_buttons(&self) -> HubResult<Vec<ButtonDepth>> {
        let n = self
            .caps
            .mouse
            .as_ref()
            .map(|m| m.live_buttons.len())
            .unwrap_or(0);
        if n == 0 {
            return Err(HubError::Unsupported("live_buttons"));
        }
        let t = self.born.elapsed().as_secs_f32();
        Ok((0..n)
            .map(|i| {
                let depth = ((t * 1.4 + i as f32 * 1.1).sin() * 0.5 + 0.5).powf(1.3);
                ButtonDepth {
                    depth,
                    trigger: Some(0.5),
                    pressed: Some(depth >= 0.5),
                }
            })
            .collect())
    }

    fn live_actuation(&self) -> HubResult<Vec<f32>> {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        Ok(st
            .keyboard
            .as_ref()
            .map(|k| {
                k.perf
                    .iter()
                    .map(|p| match p.mode {
                        TriggerMode::Rapid => p.rt_first,
                        TriggerMode::Normal => p.actuation,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn live_axis(&self) -> HubResult<Vec<AxisSample>> {
        Ok(self
            .live_travel()?
            .into_iter()
            .map(|r| AxisSample {
                route: r,
                adc: (2400.0 - r * 300.0) as u16,
                status: u8::from(r > 0.2),
                calibrated: 2,
            })
            .collect())
    }
}

// ====================================================================== Mouse (equivalent to mouse.fit)

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

fn macro_caps_mouse() -> MacroCaps {
    MacroCaps {
        slots: 3,
        max_events: 70,
        max_delay_ms: 65_535,
        modes: opts(&[
            (0, "Run specified count"),
            (1, "Repeat until released"),
            (2, "Repeat until pressed again"),
        ]),
        repeat_modes: vec![0],
        names: true,
        mouse_events: true,
    }
}

pub fn mouse_caps(wireless: bool, magnetic: bool) -> Capabilities {
    use section::*;
    use SettingValue::*;
    let mut settings = vec![SettingSpec::choice(
        "polling_wired",
        PERFORMANCE,
        "Polling rate (wired)",
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
    .help("How often the mouse reports its position to the PC (Hz)")];
    if wireless {
        settings.push(
            SettingSpec::choice(
                "polling_wireless",
                PERFORMANCE,
                "Polling rate (wireless)",
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
            .help("The selectable values change with the receiver in use (1K / 4K / 8K)"),
        );
    }
    settings.extend([
        SettingSpec::toggle("hyper_mode", PERFORMANCE, "High-speed mode")
            .help("Prioritizes performance. Uses more power (can be combined with 2K/4K/8K)"),
        SettingSpec::toggle("competitive", PERFORMANCE, "Competitive mode")
            .help("Runs the sensor at maximum performance all the time"),
        SettingSpec::toggle("long_distance", PERFORMANCE, "Long-distance mode")
            .help("Keeps the connection stable even when far from the receiver"),
        SettingSpec::choice(
            "lod",
            SENSOR,
            "Lift-off distance",
            opts(&[
                (7, "0.7mm"),
                (9, "0.9mm"),
                (12, "1.2mm"),
                (14, "1.4mm"),
                (16, "1.6mm"),
            ]),
        )
        .help("The height at which the mouse stops responding when lifted"),
        SettingSpec::toggle("motion_sync", SENSOR, "Motion sync").help(
            "Syncs the pointer with mouse movement for smoother trajectories (adds a slight delay)",
        ),
        SettingSpec::toggle("angle_snap", SENSOR, "Angle snapping").help("Corrects wobble in movement into straight lines"),
        SettingSpec::toggle("ripple", SENSOR, "Ripple control").help("Suppresses fine jitter at high DPI"),
        SettingSpec::slider("sensor_angle", SENSOR, "Sensor angle", -30., 30., 1., "°")
            .help("Rotates the coordinate axes to match your grip"),
        SettingSpec::toggle("sensor_fps_20k", SENSOR, "20K FPS")
            .help("Fixes the sensor frame rate at 20000"),
        SettingSpec::toggle("sensor_center", SENSOR, "Virtual sensor center")
            .help("Treats the sensor position as the center of the body"),
        SettingSpec::slider(
            "debounce_press",
            CLICK,
            "Click debounce (press)",
            0.,
            50.,
            1.,
            "ms",
        )
        .help("A shorter value responds faster; a longer one prevents accidental clicks")
        .confirm("Setting 0ms may cause double clicks. Continue?"),
        SettingSpec::slider(
            "debounce_release",
            CLICK,
            "Click debounce (release)",
            0.,
            50.,
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
        SettingSpec::toggle("spdt_left", CLICK, "Left SPDT")
            .help("Prevents chattering with the SPDT method of optical/mechanical switches"),
        SettingSpec::toggle("spdt_right", CLICK, "Right SPDT"),
        SettingSpec::toggle("liftoff_debounce", CLICK, "Accidental click prevention")
            .help("Ignores the click at the moment the mouse is lifted"),
        SettingSpec::toggle("combine_key", CLICK, "Key combination function")
            .help("Changes settings by pressing buttons on the mouse together. Turning it off also disables pairing switching"),
    ]);
    if magnetic {
        settings.extend([
            SettingSpec::slider(
                "mag_left_trigger",
                MAGNETIC,
                "Left trigger position",
                0.1,
                1.0,
                0.05,
                "mm",
            ),
            SettingSpec::slider(
                "mag_right_trigger",
                MAGNETIC,
                "Right trigger position",
                0.1,
                1.0,
                0.05,
                "mm",
            ),
            SettingSpec::toggle("mag_quick_trigger", MAGNETIC, "Quick trigger").help(
                "The factory calibration value. Changing it may cause repeated or missed clicks",
            ),
        ]);
    }
    if wireless {
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
            .help("Goes to sleep when left idle for this long"),
        );
    }
    settings.push(
        SettingSpec::toggle("dpi_indicator", POWER, "DPI indicator")
            .help("Signals DPI changes with the LED"),
    );
    settings.push(
        SettingSpec::choice(
            "dpi_led_effect",
            POWER,
            "Indicator display",
            opts(&[(0, "Always on"), (1, "Breathing"), (2, "On switch only")]),
        )
        .when("dpi_indicator", Bool(true)),
    );

    let mut actions = vec![
        ActionSpec::new("reset_profile", section::SYSTEM, "Reset settings")
            .help("Restores this profile's settings to their initial values")
            .danger("Restores the current settings to their initial values. Are you sure?"),
    ];
    if wireless {
        actions.push(
            ActionSpec::new("pair", section::CONNECTION, "Pair with receiver")
                .help("Insert the receiver, set the switch on the bottom of the mouse to 2.4G, then run")
                .confirm("Entering pairing mode."),
        );
    }
    if magnetic {
        actions.push(
            ActionSpec::new("calibrate", section::MAGNETIC, "Calibrate L/R buttons")
                .help("Connect by wire, press the L/R buttons all the way down and hold for 2 seconds → release")
                .confirm("Calibration will start. Wait without touching the L/R buttons."),
        );
    }
    actions.push(
        ActionSpec::new("firmware", section::SYSTEM, "Firmware update")
            .help("Choose a .bin file and write it")
            .file(&["bin"])
            .danger("Do not unplug the cable while writing. Continue?"),
    );

    Capabilities {
        settings,
        actions,
        profiles: None,
        mouse: Some(MouseCaps {
            dpi_min: 50,
            dpi_max: 40_000,
            dpi_step: 10,
            min_stages: 1,
            max_stages: 7,
            xy_split: true,
            stage_colors: true,
            stage_toggle: false,
            buttons: [
                "Left",
                "Right",
                "Wheel click",
                "Side (Back)",
                "Side (Forward)",
            ]
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
                ButtonActionKind::ScrollModeToggle,
                ButtonActionKind::Disabled,
            ],
            require_left_click: true,
            macros: Some(macro_caps_mouse()),
            live_buttons: if magnetic { vec![0, 1] } else { vec![] },
            adaptive_dpi: None,
        }),
        keyboard: None,
        lighting: vec![],
        battery: wireless,
        peq: None,
        firmware_feed: None,
    }
}

fn mouse_state(caps: &Capabilities) -> DeviceState {
    use SettingValue::*;
    let colors = [
        0xff3040, 0x2f6bff, 0x22d36b, 0xff3df5, 0xffd21a, 0xffffff, 0x2ee6ff,
    ];
    let dpis = [400, 800, 1600, 3200];
    let mut settings = BTreeMap::new();
    for s in &caps.settings {
        let v = match s.id.as_str() {
            "polling_wired" | "polling_wireless" => Choice(1000),
            "lod" => Choice(7),
            "debounce_press" | "debounce_release" => Num(2.),
            "wheel_debounce" => Num(10.),
            "sensor_angle" => Num(0.),
            "sleep_min" => Choice(1),
            "mag_left_trigger" | "mag_right_trigger" => Num(0.5),
            "dpi_led_effect" => Choice(0),
            "motion_sync" | "dpi_indicator" | "combine_key" => Bool(true),
            _ => match s.kind {
                SettingKind::Toggle => Bool(false),
                SettingKind::Slider { min, .. } => Num(min),
                SettingKind::Choice { ref options } => {
                    Choice(options.first().map(|o| o.value).unwrap_or(0))
                }
            },
        };
        settings.insert(s.id.clone(), v);
    }
    let m = caps.mouse.as_ref().expect("mouse caps");
    DeviceState {
        profile: ProfileState::default(),
        settings,
        mouse: Some(MouseState {
            stages: dpis
                .iter()
                .enumerate()
                .map(|(i, d)| DpiStage {
                    x: *d,
                    y: None,
                    color: Rgb::hex(colors[i]),
                    enabled: true,
                })
                .collect(),
            active_stage: 1,
            buttons: vec![
                ButtonAction::Mouse(MouseFn::Left),
                ButtonAction::Mouse(MouseFn::Right),
                ButtonAction::Mouse(MouseFn::Middle),
                ButtonAction::Mouse(MouseFn::Back),
                ButtonAction::Mouse(MouseFn::Forward),
            ],
            macros: {
                let mut v: Vec<Option<MacroDef>> =
                    vec![None; m.macros.as_ref().map(|c| c.slots as usize).unwrap_or(0)];
                if let Some(first) = v.first_mut() {
                    *first = Some(MacroDef {
                        name: "Jump throw".into(),
                        events: vec![
                            MacroEvent {
                                input: MacroInput::Key(0x2C),
                                edge: MacroEdge::Down,
                                delay_ms: 8,
                            },
                            MacroEvent {
                                input: MacroInput::Mouse(MouseFn::Left),
                                edge: MacroEdge::Up,
                                delay_ms: 15,
                            },
                            MacroEvent {
                                input: MacroInput::Key(0x2C),
                                edge: MacroEdge::Up,
                                delay_ms: 0,
                            },
                        ],
                        mode: 0,
                        repeat: 1,
                    });
                }
                v
            },
            adaptive: None,
        }),
        keyboard: None,
        lighting: BTreeMap::new(),
        peq: None,
    }
}

// ====================================================================== Keyboard (equivalent to hub.xsyd.top)

/// A candidate table matched to the xsyd key-code space (the numbers of keyTip_*).
/// Basic keys use the HID usage as is (xsyd also has the usage in the lower 16 bits).
pub fn xsyd_catalog() -> Vec<KeyGroup> {
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
        keys::basic_group(),
        g(
            "Mouse",
            &[
                (16640, "Mouse left", "ML"),
                (16896, "Mouse right", "MR"),
                (17152, "Mouse middle", "MM"),
                (17408, "Mouse forward", "MF"),
                (17664, "Mouse back", "MB"),
                (18945, "Wheel up", "W↑"),
                (19201, "Wheel down", "W↓"),
                (17920, "Cursor left", "M←"),
                (18176, "Cursor right", "M→"),
                (18432, "Cursor up", "M↑"),
                (18688, "Cursor down", "M↓"),
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
                (8303, "Screen brightness +", "☀+"),
                (8304, "Screen brightness −", "☀−"),
                (8586, "Mail", "Mail"),
                (8596, "My Computer", "PC"),
                (8739, "Browser", "Web"),
            ],
        ),
        g(
            "Lighting",
            &[
                (62208, "Switch lighting mode", "LM"),
                (62214, "Previous lighting mode", "LP"),
                (62209, "Switch lighting color", "LC"),
                (62210, "Brightness +", "L+"),
                (62211, "Brightness −", "L−"),
                (62212, "Speed +", "S+"),
                (62213, "Speed −", "S−"),
                (62215, "Light ON/OFF", "L⏻"),
                (62216, "Lighting direction", "L⇄"),
                (62224, "Switch deco light mode", "DM"),
                (62231, "Deco light ON/OFF", "D⏻"),
            ],
        ),
        g(
            "Layers",
            &[
                (61696, "To main layer", "L0"),
                (61697, "To Fn1 layer", "Fn1"),
                (61698, "To Fn2 layer", "Fn2"),
                (61699, "To Fn3 layer", "Fn3"),
            ],
        ),
        g(
            "System",
            &[
                (61957, "Switch to profile 1", "P1"),
                (61958, "Switch to profile 2", "P2"),
                (61959, "Switch to profile 3", "P3"),
                (61960, "Switch to profile 4", "P4"),
                (61961, "Win key lock", "WinL"),
                (61953, "Windows layout (hold)", "Win"),
                (61954, "Mac layout (hold)", "Mac"),
                (61955, "Magnetic switch calibration (hold)", "Cal"),
                (61952, "Reset (hold)", "Rst"),
            ],
        ),
    ]
}

pub fn keyboard_caps() -> Capabilities {
    use section::*;
    let layout = registry::su66max_layout();
    // Same as xsyd.rs (a mapping verified with a real-device camera — docs/PROTOCOL_NOTES.md).
    let effects = vec![
        LightEffect::new(0, "Solid color").color().still(),
        LightEffect::new(1, "Rainbow flow").color().dir(),
        LightEffect::new(2, "Slow color change").color(),
        LightEffect::new(3, "Fast color change").color(),
        LightEffect::new(4, "Rainbow checker").color(),
        LightEffect::new(5, "Rainbow cycle").color(),
        LightEffect::new(6, "Rainbow pulse").color(),
        LightEffect::new(7, "Soft window").color().dir(),
        LightEffect::new(8, "Gentle morph").color(),
        LightEffect::new(9, "Color band shift").color().dir(),
        LightEffect::new(10, "Rainbow wave").color().dir(),
        LightEffect::new(11, "Diagonal light band").color().dir(),
        LightEffect::new(12, "Confetti").color(),
        LightEffect::new(13, "Rainbow mosaic").color(),
        LightEffect::new(14, "Light rain").color().dir(),
        LightEffect::new(15, "Bouncing light").color(),
        LightEffect::new(16, "Custom").per_key(),
        LightEffect::new(17, "Press: around").color(),
        LightEffect::new(18, "Press: row").color(),
        LightEffect::new(19, "Press: wide").color(),
    ];
    let decor = vec![
        LightEffect::new(0, "Static").color().still(),
        LightEffect::new(1, "Breathing").color(),
        LightEffect::new(2, "Spectrum"),
        LightEffect::new(3, "Flow").dir(),
    ];
    let polling = opts(&[
        (8000, "8K"),
        (4000, "4K"),
        (2000, "2K"),
        (1000, "1K"),
        (500, "500"),
        (250, "250"),
        (125, "125"),
    ]);
    Capabilities {
        settings: vec![
            SettingSpec::choice("polling", PERFORMANCE, "Polling rate", polling)
                .help("How often key input is reported to the PC (Hz)"),
            SettingSpec::choice(
                "usb_mode",
                PERFORMANCE,
                "USB mode",
                opts(&[
                    (0, "USB 2.0"),
                    (1, "USB 3.0 · 8K"),
                    (2, "USB 3.0 · 16K"),
                    (3, "USB 3.0 · 32K"),
                ]),
            )
            .help("3.0 mode requires a dedicated driver")
            .confirm("Changing the USB mode reconnects the keyboard. Continue?"),
            SettingSpec::choice(
                "system_type",
                KEYBOARD,
                "OS layout",
                opts(&[(0, "Windows"), (1, "macOS")]),
            ),
            SettingSpec::toggle("win_lock", KEYBOARD, "Win key lock")
                .help("Prevents accidental Win key presses during games"),
            SettingSpec::toggle("shake_opt", KEYBOARD, "Anti-chatter optimization")
                .help("Disabling it may make keys react unintentionally with extreme settings")
                .confirm("Disabling it may cause mistyped input. Continue?"),
            SettingSpec::toggle("compat_mode", KEYBOARD, "Compatibility mode").help(
                "Use it when the keyboard is not recognized in the BIOS or some environments",
            ),
            SettingSpec::choice(
                "light_sleep",
                LIGHTING,
                "Lighting auto-off",
                opts(&[
                    (0, "Never"),
                    (1, "1 min"),
                    (3, "3 min"),
                    (5, "5 min"),
                    (10, "10 min"),
                    (30, "30 min"),
                ]),
            ),
            SettingSpec::toggle("light_link", LIGHTING, "Lighting link")
                .help("Gives the main unit and the decoration light the same effect"),
        ],
        actions: vec![
            ActionSpec::new(
                "calibrate",
                section::KEYBOARD,
                "Magnetic switch calibration",
            )
            .help("After starting, press every key all the way down once"),
            ActionSpec::new("calibrate_finish", section::KEYBOARD, "Save calibration"),
            ActionSpec::new("reset_perf", section::SYSTEM, "Reset trigger settings")
                .danger("Restores the trigger settings of all keys to their initial values."),
            ActionSpec::new("reset_keymap", section::SYSTEM, "Reset key mapping")
                .danger("Restores the key assignments of all layers to their initial values."),
            ActionSpec::new("reset_lighting", section::SYSTEM, "Reset lighting")
                .danger("Restores lighting to its initial values."),
            ActionSpec::new("factory_reset", section::SYSTEM, "Restore factory state")
                .danger("All profiles, macros and calibration will be erased. Are you sure?"),
            ActionSpec::new("firmware", section::SYSTEM, "Firmware update")
                .file(&["bin"])
                .danger("Do not disconnect during the update. Continue?"),
        ],
        profiles: Some(ProfileCaps {
            count: 4,
            renamable: true,
        }),
        mouse: None,
        keyboard: Some(KeyboardCaps {
            layout,
            layers: vec!["Main".into(), "Fn".into()],
            catalog: xsyd_catalog(),
            combos: true,
            perf: Some(PerfCaps {
                travel: 4.0,
                min: 0.01,
                step: 0.01,
                rapid_trigger: true,
                separate_release: true,
                rt_first: true,
                deadzones: true,
                continuous_rt: true,
                switch_types: vec![
                    "Default (magnetic A)".into(),
                    "Magnetic B (light)".into(),
                    "Magnetic C (long linear)".into(),
                ],
                switch_ids: vec![],
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
            macros: Some(MacroCaps {
                slots: 16,
                max_events: 128,
                max_delay_ms: 32_768,
                modes: opts(&[
                    (0, "Click to run a set number of times"),
                    (1, "Repeat while held (stops immediately on release)"),
                    (2, "Repeat while held (stops after completing)"),
                    (3, "Click to start / click again to stop"),
                ]),
                repeat_modes: vec![0],
                names: false,
                mouse_events: false,
            }),
        }),
        lighting: vec![
            LightArea {
                id: 0,
                name: "Main keyboard".into(),
                effects,
                brightness_max: 4,
                speed_max: 4,
                directions: vec!["Forward".into(), "Reverse".into()],
                switchable: true,
                zones: vec!["Upper light".into(), "Lower light".into()],
                per_key: true,
                direct_drive: true,
                live_readback: false,
            },
            LightArea {
                id: 1,
                name: "Decoration light".into(),
                effects: decor,
                brightness_max: 4,
                speed_max: 4,
                directions: vec!["Forward".into(), "Reverse".into()],
                switchable: true,
                zones: vec![],
                per_key: false,
                direct_drive: false,
                live_readback: false,
            },
        ],
        battery: false,
        peq: None,
        firmware_feed: None,
    }
}

fn keyboard_state(caps: &Capabilities) -> DeviceState {
    let k = caps.keyboard.as_ref().expect("keyboard caps");
    let n = k.layout.len();
    // Legend → HID usage (default keymap).
    let label_usage = |label: &str| -> KeyAction {
        let special = match label {
            "Esc" => Some(0x29),
            "Backspace" => Some(0x2A),
            "Tab" => Some(0x2B),
            "Caps" => Some(0x39),
            "Enter" => Some(0x28),
            "LShift" => Some(0xE1),
            "RShift" => Some(0xE5),
            "LCtrl" => Some(0xE0),
            "LWin" => Some(0xE3),
            "LAlt" => Some(0xE2),
            "RAlt" => Some(0xE6),
            "Space" => Some(0x2C),
            "Del" => Some(0x4C),
            "↑" => Some(0x52),
            "↓" => Some(0x51),
            "←" => Some(0x50),
            "→" => Some(0x4F),
            "Fn" => return KeyAction::Code(61697),
            _ => None,
        };
        let usage = special.or_else(|| keys::HID_KEYS.iter().find(|h| h.2 == label).map(|h| h.0));
        usage
            .map(|u| KeyAction::Code(u as u32))
            .unwrap_or(KeyAction::None)
    };
    let main: Vec<KeyAction> = k.layout.iter().map(|l| label_usage(&l.label)).collect();
    // Fn layer: number row → F1..F12, arrows → Home/End/PgUp/PgDn, the rest transparent.
    let fnl: Vec<KeyAction> = k
        .layout
        .iter()
        .map(|l| match l.label.as_str() {
            "1" => KeyAction::Code(0x3A),
            "2" => KeyAction::Code(0x3B),
            "3" => KeyAction::Code(0x3C),
            "4" => KeyAction::Code(0x3D),
            "5" => KeyAction::Code(0x3E),
            "6" => KeyAction::Code(0x3F),
            "7" => KeyAction::Code(0x40),
            "8" => KeyAction::Code(0x41),
            "9" => KeyAction::Code(0x42),
            "0" => KeyAction::Code(0x43),
            "-" => KeyAction::Code(0x44),
            "=" => KeyAction::Code(0x45),
            "↑" => KeyAction::Code(0x4B),
            "↓" => KeyAction::Code(0x4E),
            "←" => KeyAction::Code(0x4A),
            "→" => KeyAction::Code(0x4D),
            "Esc" => KeyAction::Code(0x35),
            "Fn" => KeyAction::Code(61697),
            "LWin" => KeyAction::Code(61961),
            "Q" => KeyAction::Code(61957),
            "W" => KeyAction::Code(61958),
            "E" => KeyAction::Code(61959),
            "R" => KeyAction::Code(61960),
            _ => KeyAction::Transparent,
        })
        .collect();
    let mut perf = vec![KeyPerf::default(); n];
    // WASD is a sample of RT.
    for (i, l) in k.layout.iter().enumerate() {
        if matches!(l.label.as_str(), "W" | "A" | "S" | "D") {
            perf[i] = KeyPerf {
                mode: TriggerMode::Rapid,
                actuation: 0.4,
                rt_first: 0.2,
                rt_press: 0.05,
                rt_release: 0.05,
                ..KeyPerf::default()
            };
        }
    }
    let idx = |label: &str| k.layout.iter().position(|l| l.label == label).unwrap_or(0);
    let mut lighting = BTreeMap::new();
    lighting.insert(
        0,
        LightingState {
            on: true,
            effect: 14,
            color: Rgb::hex(0x7ad7ff),
            brightness: 75,
            speed: 50,
            direction: 0,
            per_key: BTreeMap::new(),
            zone_on: vec![true, true],
            palette: vec![
                Rgb::hex(0x7ad7ff),
                Rgb::hex(0xff5c7a),
                Rgb::hex(0x5cff8a),
                Rgb::hex(0xffd25c),
                Rgb::hex(0x8a7aff),
                Rgb::hex(0xff8a3c),
                Rgb::hex(0x3cffe0),
                Rgb::hex(0xffffff),
            ],
            palette_sel: 0,
            color_correction: Some(Rgb::new(100, 100, 100)),
        },
    );
    lighting.insert(
        1,
        LightingState {
            on: true,
            effect: 1,
            color: Rgb::hex(0xa98bff),
            brightness: 50,
            speed: 50,
            direction: 0,
            per_key: BTreeMap::new(),
            zone_on: vec![],
            palette: vec![Rgb::hex(0xa98bff)],
            palette_sel: 0,
            color_correction: Some(Rgb::new(100, 100, 100)),
        },
    );
    let mut settings = BTreeMap::new();
    for s in &caps.settings {
        settings.insert(
            s.id.clone(),
            match s.id.as_str() {
                "polling" => SettingValue::Choice(8000),
                "shake_opt" => SettingValue::Bool(true),
                _ => match &s.kind {
                    SettingKind::Toggle => SettingValue::Bool(false),
                    SettingKind::Slider { min, .. } => SettingValue::Num(*min),
                    SettingKind::Choice { options } => {
                        SettingValue::Choice(options.first().map(|o| o.value).unwrap_or(0))
                    }
                },
            },
        );
    }
    let slots = k.macros.as_ref().map(|m| m.slots as usize).unwrap_or(0);
    DeviceState {
        profile: ProfileState {
            active: 0,
            names: vec![
                "VALORANT".into(),
                "FPS general".into(),
                "Typing".into(),
                String::new(),
            ],
        },
        settings,
        mouse: None,
        keyboard: Some(KeyboardState {
            keymap: vec![main, fnl],
            perf,
            advanced: vec![
                AdvancedKey {
                    key: idx("A"),
                    layer: 0,
                    binding: Advanced::Socd {
                        other: idx("D"),
                        code_a: 0x04,
                        code_b: 0x07,
                        mode: SocdMode::Last,
                        delay_ms: 0,
                    },
                },
                AdvancedKey {
                    key: idx("LShift"),
                    layer: 0,
                    binding: Advanced::Mt {
                        tap: 0x29,
                        hold: 0xE1,
                        hold_ms: 200,
                    },
                },
            ],
            macros: vec![None; slots],
        }),
        lighting,
        peq: None,
    }
}

// ====================================================================== USB DAC (equivalent to peq.szwalkplay.com)

fn dac_caps() -> Capabilities {
    Capabilities {
        settings: vec![
            SettingSpec::choice(
                "dac_filter",
                section::AUDIO,
                "DAC digital filter",
                opts(&[
                    (0, "FAST-LL"),
                    (1, "FAST-PC"),
                    (2, "SLOW-LL"),
                    (3, "SLOW-PC"),
                    (4, "NON-OS"),
                ]),
            ),
            SettingSpec::choice(
                "dac_work_mode",
                section::AUDIO,
                "DAC work mode",
                opts(&[(0, "Class-H"), (1, "Class-AB")]),
            ),
            SettingSpec::choice(
                "output_gain",
                section::AUDIO,
                "Output gain mode",
                opts(&[(0, "Low"), (1, "High")]),
            ),
            SettingSpec::slider(
                "channel_balance",
                section::AUDIO,
                "Channel balance",
                -127.,
                127.,
                1.,
                "",
            )
            .help("Positive = shift to the right (attenuate the left). 0 is center."),
            SettingSpec::slider("eq_volume", section::AUDIO, "EQ volume", -16., 6., 1., "dB"),
            SettingSpec::slider(
                "mic_offset",
                section::AUDIO,
                "Mic offset",
                -15.,
                15.,
                1.,
                "",
            ),
        ],
        actions: vec![ActionSpec::new(
            "eq_def",
            section::AUDIO,
            "Reset EQ to factory values (DEF)",
        )
        .help("Writes the official 8-band DEF preset")
        .confirm("Resets the EQ to the DEF preset. Are you sure?")],
        profiles: None,
        mouse: None,
        keyboard: None,
        lighting: vec![],
        battery: false,
        peq: Some(PeqCaps {
            bands: 10,
            freq_min: 20,
            freq_max: 20_000,
            q_min: 0.5,
            q_max: 10.,
            gain_min: -12.,
            gain_max: 12.,
            filters: vec![
                PeqFilter::LowShelf,
                PeqFilter::Peak,
                PeqFilter::HighShelf,
                PeqFilter::LowPass,
                PeqFilter::HighPass,
            ],
        }),
        firmware_feed: None,
    }
}

fn dac_state(caps: &Capabilities) -> DeviceState {
    let mut settings = BTreeMap::new();
    for s in &caps.settings {
        let v = match s.id.as_str() {
            "dac_filter" => SettingValue::Choice(2),
            "output_gain" => SettingValue::Choice(1),
            "eq_volume" => SettingValue::Num(-1.),
            _ => match s.kind {
                SettingKind::Toggle => SettingValue::Bool(false),
                SettingKind::Slider { .. } => SettingValue::Num(0.),
                SettingKind::Choice { ref options } => {
                    SettingValue::Choice(options.first().map(|o| o.value).unwrap_or(0))
                }
            },
        };
        settings.insert(s.id.clone(), v);
    }
    let freqs = [31u16, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    DeviceState {
        profile: ProfileState::default(),
        settings,
        mouse: None,
        keyboard: None,
        lighting: BTreeMap::new(),
        peq: Some(
            freqs
                .iter()
                .map(|&f| PeqBand {
                    freq: f,
                    q: 0.75,
                    gain: 0.,
                    filter: PeqFilter::Peak,
                })
                .collect(),
        ),
    }
}

// ====================================================================== LAMZU (equivalent to the official Aurora)

fn lamzu_state(caps: &Capabilities) -> DeviceState {
    let mut settings = BTreeMap::new();
    for (id, v) in [
        ("polling_wireless", SettingValue::Choice(8000)),
        ("hyper_mode", SettingValue::Bool(true)),
        ("tracking_mode", SettingValue::Bool(true)),
        ("lod", SettingValue::Choice(7)),
        ("glass_mode", SettingValue::Bool(false)),
        ("motion_sync", SettingValue::Bool(false)),
        ("angle_snap", SettingValue::Bool(false)),
        ("ripple", SettingValue::Bool(false)),
        ("dpi_xy", SettingValue::Bool(false)),
        ("angle_tune", SettingValue::Num(1.)),
        ("sensor_offset", SettingValue::Num(0.)),
        ("debounce", SettingValue::Num(0.)),
        ("rapid_left", SettingValue::Bool(true)),
        ("rapid_right", SettingValue::Bool(false)),
        ("bhop_dir", SettingValue::Choice(0)),
        ("bhop_ms", SettingValue::Choice(200)),
        ("sleep_min", SettingValue::Choice(1)),
        ("dongle_led", SettingValue::Bool(true)),
        ("dongle_led1", SettingValue::Choice(10)),
        ("dongle_led2", SettingValue::Choice(11)),
        ("dongle_led3", SettingValue::Choice(6)),
    ] {
        if caps.setting(id).is_some() {
            settings.insert(id.to_string(), v);
        }
    }
    let colors = [0xD00067u32, 0x00A5D1, 0x0F9B00, 0xFFFFFF, 0xCAC700];
    let stages = [400u32, 800, 1600, 3200, 6400]
        .iter()
        .zip(colors)
        .map(|(&x, c)| DpiStage {
            x,
            y: None,
            color: Rgb::hex(c),
            enabled: true,
        })
        .collect();
    DeviceState {
        profile: ProfileState::default(),
        settings,
        mouse: Some(MouseState {
            stages,
            active_stage: 2,
            buttons: vec![
                ButtonAction::Mouse(MouseFn::Left),
                ButtonAction::Mouse(MouseFn::Right),
                ButtonAction::Mouse(MouseFn::Middle),
                ButtonAction::Mouse(MouseFn::Back),
                ButtonAction::Mouse(MouseFn::Forward),
            ],
            macros: vec![],
            adaptive: Some(AdaptiveCurve {
                enabled: false,
                points: crate::lamzu::adaptive_presets()[0].1.clone(),
            }),
        }),
        keyboard: None,
        lighting: BTreeMap::new(),
        peq: None,
    }
}
