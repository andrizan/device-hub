//! Model table and Capabilities for the LAMZU Aurora (www.lamzu.net) LM20 generation.
//!
//! Used by both the driver (`devhub-hid::lamzu`) and the demo (`demo.rs`).
//! All values are taken from the official `Config/env-models.json` (v1.0.38) and the LM20 path
//! (`T2()`) of the settings screen `index-Bn9q780G.js`
//! (docs/LAMZU_MAYA_X_V2_AUDIT2.md). Display names follow the wording of the official Aurora.

use crate::model::*;

/// One model of the LM20 generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LamzuModel {
    /// Wired PID (via the dongle it is returned as the paired-device PID).
    pub pid: u16,
    /// Product name (as written on the official product page).
    pub name: &'static str,
    /// Internal model id.
    pub model: &'static str,
    /// Row name in the official model definition.
    pub row: &'static str,
    pub sensor_offset: bool,
    pub adaptive_dpi: bool,
    pub glass_mode: bool,
}

/// LM20 rows of the official model definition (the Mini's receiver is 0054).
pub const MODELS: &[LamzuModel] = &[
    LamzuModel {
        pid: 0x0044,
        name: "LAMZU MAYA X V2",
        model: "maya-x-v2",
        row: "Maya X(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
    LamzuModel {
        pid: 0x0050,
        name: "LAMZU MAYA V2",
        model: "maya-v2",
        row: "Maya(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
    LamzuModel {
        pid: 0x0052,
        name: "LAMZU MAYA M",
        model: "maya-m",
        row: "MAYA M(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
    LamzuModel {
        pid: 0x0056,
        name: "LAMZU ORCUS V2",
        model: "orcus-v2",
        row: "Orcus V2(LM20)",
        sensor_offset: false,
        adaptive_dpi: false,
        glass_mode: false,
    },
    LamzuModel {
        pid: 0x0046,
        name: "LAMZU ATLANTIS MINI V2",
        model: "atlantis-mini-v2",
        row: "Mini(LM20)",
        sensor_offset: true,
        adaptive_dpi: true,
        glass_mode: true,
    },
];

pub fn model_by_pid(pid: u16) -> Option<&'static LamzuModel> {
    MODELS.iter().find(|m| m.pid == pid)
}

/// Response of the sensor-model command (cmd 143) → (display name, DPI max, DPI step, LOD candidates (mm×10)).
/// Official `T2()`: 1=PAW3395 (26000, 1/2mm), 2=PAW3950 (30000, 0.7/1/2mm),
/// 4=PAW3955 (50000, 0.7/1/1.3/1.5/1.7mm, step 1). Unknown falls back to the model definition default (50000, 0.7/1/2).
pub fn sensor_profile(code: u8) -> (&'static str, u32, u32, &'static [i64]) {
    match code {
        1 => ("PAW3395", 26000, 50, &[10, 20]),
        2 => ("PAW3950", 30000, 50, &[7, 10, 20]),
        4 => ("PAW3955", 50000, 1, &[7, 10, 13, 15, 17]),
        _ => ("", 50000, 50, &[7, 10, 20]),
    }
}

/// The official speed-linked DPI presets (settings screen `AdaDPI*PointList`).
pub fn adaptive_presets() -> Vec<(String, Vec<AdaptivePoint>)> {
    let p = |v: &[(u16, f32)]| {
        v.iter()
            .map(|&(speed, ratio)| AdaptivePoint { speed, ratio })
            .collect::<Vec<_>>()
    };
    vec![
        ("Classic".into(), p(&[(0, 1.0), (20, 1.0), (40, 1.0)])),
        ("Natural".into(), p(&[(0, 1.5), (20, 0.75), (40, 0.1)])),
        ("Leap".into(), p(&[(0, 0.1), (20, 0.75), (40, 1.5)])),
    ]
}

fn opts(v: &[(i64, &str)]) -> Vec<ChoiceOpt> {
    v.iter().map(|(a, b)| ChoiceOpt::new(*a, *b)).collect()
}

/// Capabilities matched to the model, connection and sensor.
pub fn caps(model: &LamzuModel, transport: Transport, sensor: u8) -> Capabilities {
    use section::*;
    let wireless = transport == Transport::Wireless24G;
    let (_, dpi_max, dpi_step, lods) = sensor_profile(sensor);
    let polling: &[(i64, &str)] = if wireless {
        // Official: wireless is PollingRateWireless = 1000;2000;4000;8000.
        &[(1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")]
    } else {
        &[(500, "500"), (1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")]
    };
    let lod_opts: Vec<ChoiceOpt> = lods
        .iter()
        .map(|&v| {
            let label = if v % 10 == 0 {
                format!("{}mm", v / 10)
            } else {
                format!("{:.1}mm", v as f32 / 10.)
            };
            ChoiceOpt::new(v, &label)
        })
        .collect();

    let mut settings = vec![
        SettingSpec::choice("polling_wireless", PERFORMANCE, "Polling rate", opts(polling))
            .help("How often the mouse updates its data (Hz)"),
        SettingSpec::toggle("hyper_mode", PERFORMANCE, "Competitive mode").help(
            "When OFF, high-speed mode is used. A mode tuned to balance peak performance and battery life",
        ),
        SettingSpec::toggle("tracking_mode", PERFORMANCE, "20K FPS")
            .help("Extreme-performance mode with 20kHz scanning. Uses more power than competitive mode")
            .enabled_when(
                "hyper_mode",
                SettingValue::Bool(true),
                "Turn on competitive mode first",
            )
            .confirm(
                "20K FPS shortens battery life considerably compared with competitive mode. Switch anyway?",
            ),
        SettingSpec::choice("lod", SENSOR, "LOD", lod_opts)
            .help("The height at which the mouse leaves the tracking surface and the sensor stops responding"),
    ];
    if model.glass_mode {
        settings.push(
            SettingSpec::toggle("glass_mode", SENSOR, "Glass mode")
                .help("Automatically detects and adapts to glass mousepads, and disables LOD adjustment"),
        );
        // LOD cannot be changed while glass mode is on (same as official).
        if let Some(l) = settings.iter_mut().find(|s| s.id == "lod") {
            l.enabled_when = Some((
                "glass_mode".into(),
                SettingValue::Bool(false),
                "LOD cannot be changed while glass mode is on".into(),
            ));
        }
    }
    settings.extend([
        SettingSpec::toggle("motion_sync", SENSOR, "Motion sync")
            .help("Syncs mouse movement with PC processing for more accurate trajectories"),
        SettingSpec::toggle("angle_snap", SENSOR, "Angle snapping")
            .help("Makes mouse input straighter and more precise"),
        SettingSpec::toggle("ripple", SENSOR, "Ripple control")
            .help("Eliminates wavy pointer jitter during fast mouse movement"),
        SettingSpec::slider("angle_tune", SENSOR, "Angle tuning", -30., 30., 1., "°")
            .help("Corrects the sensor angle to match your grip, so the pointer moves horizontally even if you hold the mouse at a slant"),
    ]);
    if model.sensor_offset {
        settings.push(
            SettingSpec::slider("sensor_offset", SENSOR, "Sensor center offset", -5., 5., 1., "")
                .help("Correction of the sensor's center position (official SensorCenterOffset)"),
        );
    }
    settings.extend([
        SettingSpec::slider("debounce", CLICK, "Debounce time", 0., 15., 1., "ms").help(
            "Too low a value can cause double clicks. 2–4ms is recommended for gaming",
        ),
        SettingSpec::toggle("rapid_left", CLICK, "Rapid Trigger (left click)")
            .help("Maximizes key response with an algorithm — may cause accidental double clicks"),
        SettingSpec::toggle("rapid_right", CLICK, "Rapid Trigger (right click)")
            .help("Maximizes key response with an algorithm — may cause accidental double clicks"),
        SettingSpec::choice(
            "bhop_dir",
            CLICK,
            "Scroll bhop",
            opts(&[(0, "Off"), (2, "Scroll up"), (3, "Scroll down")]),
        )
        .help("Ignores the first scroll signal and does not scroll unless a second one arrives within the response time"),
        SettingSpec::choice(
            "bhop_ms",
            CLICK,
            "Wheel response time",
            opts(&[
                (200, "200ms"),
                (400, "400ms"),
                (600, "600ms"),
                (800, "800ms"),
                (1000, "1000ms"),
            ]),
        )
        .help("How long Scroll bhop waits for the second scroll"),
        SettingSpec::choice(
            "sleep_min",
            POWER,
            "Sleep time",
            opts(&[
                (1, "1 min"),
                (5, "5 min"),
                (10, "10 min"),
                (15, "15 min"),
                (20, "20 min"),
                (25, "25 min"),
            ]),
        )
        .help("Goes to sleep after a period of inactivity"),
    ]);
    if wireless {
        settings.push(
            SettingSpec::toggle("dongle_led", LIGHTING, "Dongle LED")
                .help("Shows the mouse status on the receiver's light"),
        );
        for (id, name) in [
            ("dongle_led1", "LED 1"),
            ("dongle_led2", "LED 2"),
            ("dongle_led3", "LED 3"),
        ] {
            settings.push(
                SettingSpec::choice(
                    id,
                    LIGHTING,
                    name,
                    opts(&[(6, "Battery"), (11, "DPI"), (10, "Polling rate"), (12, "Signal")]),
                )
                .when("dongle_led", SettingValue::Bool(true)),
            );
        }
    }
    Capabilities {
        settings,
        actions: vec![
            ActionSpec::new("reset_profile", section::SYSTEM, "Reset profile")
                .help("Restores only the current profile to its factory values")
                .danger("All settings of the current profile will be reset"),
            ActionSpec::new("factory_reset", section::SYSTEM, "Factory reset")
                .help(if wireless {
                    "Restores the mouse and receiver to their factory default settings"
                } else {
                    "Restores the mouse to its factory default settings"
                })
                .danger("After you press OK, the mouse returns to its factory default settings"),
        ],
        profiles: Some(ProfileCaps {
            count: 4,
            renamable: false,
        }),
        mouse: Some(MouseCaps {
            dpi_min: 50,
            dpi_max,
            dpi_step,
            min_stages: 1,
            max_stages: 5,
            xy_split: true,
            stage_colors: true,
            stage_toggle: false,
            buttons: ["Left click", "Right click", "Wheel click", "Back", "Forward"]
                .iter()
                .map(|n| ButtonSlot { name: (*n).into() })
                .collect(),
            button_kinds: vec![
                ButtonActionKind::Mouse,
                ButtonActionKind::Key,
                ButtonActionKind::Media,
                ButtonActionKind::Dpi,
                ButtonActionKind::Fire,
                ButtonActionKind::PollingCycle,
                ButtonActionKind::ProfileCycle,
                ButtonActionKind::Disabled,
            ],
            require_left_click: true,
            macros: None,
            live_buttons: vec![],
            adaptive_dpi: model.adaptive_dpi.then(|| AdaptiveCaps {
                min_points: 2,
                max_points: 5,
                speed_max: 105,
                ratio_max: 1.5,
                presets: adaptive_presets(),
            }),
        }),
        keyboard: None,
        lighting: vec![],
        battery: true,
        peq: None,
        firmware_feed: None,
    }
}
