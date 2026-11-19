//! Data model of device settings (contract v2).
//!
//! The **common language** of the frontend (device-hub) and the backend (devhub-hid).
//! It never describes protocol-level representations (byte sequences, EEPROM addresses, command numbers).
//! Converting them is the job of each driver (on the devhub-hid side).
//!
//! # The three layers
//!
//! 1. **`Capabilities`** — a static description of "what the device has". It decides everything
//!    the UI shows or hides. The UI never branches on device name or VID/PID.
//! 2. **`DeviceState`** — a snapshot of the current setting values (`DeviceDriver::read`).
//! 3. **`Change`** — **one fine-grained change** sent by the UI (`DeviceDriver::apply`).
//!    Applied immediately, like the web version. The driver only needs to write the items it received
//!    (there is no need to rewrite the whole keymap every time).
//!
//! # Generic settings (`SettingSpec`)
//!
//! "Scalar settings" that can be expressed as a toggle, slider or choice (polling rate, LOD,
//! debounce, MotionSync, sleep time, USB mode...) are declared with `SettingSpec` **without adding types**.
//! The UI lines them up automatically in a card per section.
//! A device's own feature appears in the UI as soon as the driver adds a single spec.
//!
//! Only structured settings (DPI stages, button assignments, macros, keymaps, per-key
//! travel, advanced keys, lighting) have dedicated types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

// ====================================================================== identity

/// A stable device identifier. For HID, `hid:<vid>:<pid>:<serial or path hash>` is recommended.
/// The same device must get the same ID across restarts (the UI uses it to remember display names, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    /// A receiver on its own, etc.
    Other,
    /// Earbuds / headphones (settings over Bluetooth, not HID).
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    /// Connected directly by cable.
    Wired,
    /// Via a 2.4GHz dongle.
    Wireless24G,
    Bluetooth,
    Unknown,
}

/// Kind of protocol driver (1:1 with the implementation modules in devhub-hid).
/// When adding a new protocol family, add one line here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverKind {
    /// hub.xsyd.top family (Sparklink WebHID SDK).
    Xsyd,
    /// www.mouse.fit family (EEPROM-style protocol of G-Wolves, etc.).
    Mousefit,
    /// peq.szwalkplay.com family (WalkPlay CB-series USB DAC. report 0x4B S1 protocol).
    Walkplay,
    /// hub.atk.pro / ATK HUB family (ATK F1, etc. report-8/EEPROM, model determined by CID/MID).
    Atk,
    /// www.lamzu.net / Aurora family (64B feature report, if2 FFFF:00).
    Lamzu,
    /// RongYuan (VID 3151) keyboards, mice and their 2.4G receivers:
    /// 64-byte feature reports on FFFF:02, device resolved by the `0x8F` id.
    Ry,
    /// EarFun earbuds on Qualcomm chips: GAIA frames over Bluetooth RFCOMM (SPP).
    Earfun,
    /// Unassigned (can only be detected).
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Battery {
    /// 0..=100. None if unknown.
    pub percent: Option<u8>,
    pub charging: bool,
}

/// The name tag of one detected device.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub kind: DeviceKind,
    /// Display name (e.g. "HTS Plus Pro").
    pub name: String,
    pub vendor: String,
    /// Model identifier (the registry's model. The UI only uses it for things like icon selection).
    pub model: String,
    pub vid: u16,
    pub pid: u16,
    pub transport: Transport,
    /// Main-unit firmware.
    pub firmware: Option<String>,
    /// Receiver (dongle) firmware.
    pub dongle_firmware: Option<String>,
    pub serial: Option<String>,
    pub driver: DriverKind,
    /// The battery as known at detection time. The latest value is `DeviceDriver::battery`.
    pub battery: Option<Battery>,
    /// false if there is no protocol implementation or it is incomplete. The UI shows "detected, unsupported".
    pub supported: bool,
    /// true when a wireless device does not respond, e.g. because it is asleep (only the dongle is visible).
    /// The UI shows "Move the mouse to wake it up".
    pub asleep: bool,
    /// Board identifier (Sparklink family uses it to query the firmware distribution API).
    /// None for drivers that cannot obtain it.
    pub board_id: Option<u32>,
}

// ====================================================================== color

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    pub const fn hex(v: u32) -> Self {
        Self::new(
            ((v >> 16) & 255) as u8,
            ((v >> 8) & 255) as u8,
            (v & 255) as u8,
        )
    }
    pub fn to_hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
}

// ====================================================================== generic settings

/// The value of a generic setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SettingValue {
    Bool(bool),
    /// Slider/number. The unit is on the spec side.
    Num(f32),
    /// A `ChoiceOpt::value` of the options.
    Choice(i64),
}

impl SettingValue {
    pub fn as_bool(&self) -> bool {
        match self {
            SettingValue::Bool(b) => *b,
            SettingValue::Num(n) => *n != 0.,
            SettingValue::Choice(c) => *c != 0,
        }
    }
    pub fn as_num(&self) -> f32 {
        match self {
            SettingValue::Bool(b) => *b as i32 as f32,
            SettingValue::Num(n) => *n,
            SettingValue::Choice(c) => *c as f32,
        }
    }
    pub fn as_choice(&self) -> i64 {
        match self {
            SettingValue::Bool(b) => *b as i64,
            SettingValue::Num(n) => n.round() as i64,
            SettingValue::Choice(c) => *c,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceOpt {
    pub value: i64,
    pub label: String,
}

impl ChoiceOpt {
    pub fn new(value: i64, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    Toggle,
    /// Continuous value. `unit` is for display ("ms", "mm", "°", "min", etc.).
    Slider {
        min: f32,
        max: f32,
        step: f32,
        unit: String,
    },
    /// Options. Shown as a segmented control if there are few, a dropdown if many.
    Choice {
        options: Vec<ChoiceOpt>,
    },
}

/// The card a setting is placed on (a UI section). Even an unknown string does not crash the UI; it just goes under "Other".
pub mod section {
    /// Polling rate, USB mode, high-speed mode, etc.
    pub const PERFORMANCE: &str = "performance";
    /// LOD, MotionSync, angle snapping, ripple, sensor angle, etc.
    pub const SENSOR: &str = "sensor";
    /// Click/wheel debounce, SPDT, misoperation prevention.
    pub const CLICK: &str = "click";
    /// Magnetic switch buttons (trigger position, Rapid Trigger, vibration).
    pub const MAGNETIC: &str = "magnetic";
    /// Sleep, power saving, battery.
    pub const POWER: &str = "power";
    /// Overall lighting switches, etc. (effects belong to LightArea).
    pub const LIGHTING: &str = "lighting";
    /// Whole keyboard: OS type, Win key lock, anti-chatter, etc.
    pub const KEYBOARD: &str = "keyboard";
    /// Connection, receiver.
    pub const CONNECTION: &str = "connection";
    /// Audio settings for USB DACs (EQ, filters, balance, etc.).
    pub const AUDIO: &str = "audio";
    /// Touch gestures and buttons of earbuds / headphones.
    pub const CONTROLS: &str = "controls";
    /// Everything else (reset, firmware, etc.).
    pub const SYSTEM: &str = "system";
}

/// The declaration of one generic setting.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    /// Unique within the driver ("polling_wired", "lod", "motion_sync" ...). Comes back in `Change::Setting`.
    pub id: String,
    /// One of `section::*`.
    pub section: String,
    pub label: String,
    /// A one-line description.
    pub help: Option<String>,
    pub kind: SettingKind,
    /// Shown only when another setting has this value.
    pub visible_when: Option<(String, SettingValue)>,
    /// Text of the confirmation dialog shown before a change (for dangerous settings).
    pub confirm: Option<String>,
    /// Operable only when another setting has this value (otherwise shown disabled).
    /// `(setting id, value, explanation when disabled)`. Example: 20K FPS only while competitive mode is ON.
    pub enabled_when: Option<(String, SettingValue, String)>,
}

impl SettingSpec {
    pub fn toggle(id: &str, section: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            section: section.into(),
            label: label.into(),
            help: None,
            kind: SettingKind::Toggle,
            visible_when: None,
            confirm: None,
            enabled_when: None,
        }
    }
    pub fn slider(
        id: &str,
        section: &str,
        label: &str,
        min: f32,
        max: f32,
        step: f32,
        unit: &str,
    ) -> Self {
        Self {
            kind: SettingKind::Slider {
                min,
                max,
                step,
                unit: unit.into(),
            },
            ..Self::toggle(id, section, label)
        }
    }
    pub fn choice(id: &str, section: &str, label: &str, options: Vec<ChoiceOpt>) -> Self {
        Self {
            kind: SettingKind::Choice { options },
            ..Self::toggle(id, section, label)
        }
    }
    pub fn help(mut self, h: &str) -> Self {
        self.help = Some(h.into());
        self
    }
    pub fn when(mut self, id: &str, v: SettingValue) -> Self {
        self.visible_when = Some((id.into(), v));
        self
    }
    pub fn confirm(mut self, msg: &str) -> Self {
        self.confirm = Some(msg.into());
        self
    }
    /// Operable only when the setting `id` is `v` (otherwise shown disabled with `note` attached).
    pub fn enabled_when(mut self, id: &str, v: SettingValue, note: &str) -> Self {
        self.enabled_when = Some((id.into(), v, note.into()));
        self
    }
}

/// An operation run by a single button (reset, pairing, calibration, firmware update...).
///
/// UI conventions (naming rules for id):
/// - `calibrate` … for a keyboard with `PerfCaps::live_travel`, the UI opens the
///   **calibration wizard** instead of a confirmation dialog (on start it runs `calibrate` → watches every key bottom out via `live_travel`
///   → on completion it runs `calibrate_finish` if there is one).
/// - ids ending in `*_finish` … not shown in the list (they are for the wizard's finishing step).
/// - For a long operation, sending `HubEvent::Progress` while it runs makes the UI show a progress bar.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionSpec {
    pub id: String,
    pub section: String,
    pub label: String,
    pub help: Option<String>,
    /// If Some, a confirmation dialog is shown before running.
    pub confirm: Option<String>,
    /// If Some, the UI opens a file-chooser dialog and passes the chosen path (extension filter).
    pub file: Option<Vec<String>>,
    /// If true, `file` is a save dialog (for exporting settings). false = open.
    pub save: bool,
    /// Shown in red as a destructive operation.
    pub danger: bool,
}

impl ActionSpec {
    pub fn new(id: &str, section: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            section: section.into(),
            label: label.into(),
            help: None,
            confirm: None,
            file: None,
            save: false,
            danger: false,
        }
    }
    pub fn help(mut self, h: &str) -> Self {
        self.help = Some(h.into());
        self
    }
    pub fn danger(mut self, confirm: &str) -> Self {
        self.danger = true;
        self.confirm = Some(confirm.into());
        self
    }
    pub fn confirm(mut self, confirm: &str) -> Self {
        self.confirm = Some(confirm.into());
        self
    }
    pub fn file(mut self, exts: &[&str]) -> Self {
        self.file = Some(exts.iter().map(|s| s.to_string()).collect());
        self
    }
    /// Same as `file`, but opens a "Save as" dialog (exporting settings).
    pub fn save_file(mut self, exts: &[&str]) -> Self {
        self.file = Some(exts.iter().map(|s| s.to_string()).collect());
        self.save = true;
        self
    }
}

/// The result of `run_action`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionOutcome {
    /// Text shown in the toast (if None, the default "Done").
    pub message: Option<String>,
    /// If true, the UI rereads all settings with `read()` (after a reset, etc.).
    pub reload: bool,
}

// ====================================================================== Shared: keys and macros

/// Modifier key bits (same order as the HID modifier byte).
pub mod mods {
    pub const LCTRL: u8 = 0x01;
    pub const LSHIFT: u8 = 0x02;
    pub const LALT: u8 = 0x04;
    pub const LGUI: u8 = 0x08;
    pub const RCTRL: u8 = 0x10;
    pub const RSHIFT: u8 = 0x20;
    pub const RALT: u8 = 0x40;
    pub const RGUI: u8 = 0x80;
}

/// One keystroke of a keyboard key (+ modifiers). `usage` is a usage of the HID Keyboard/Keypad page (0x07).
/// 0 means "modifiers only". The name comes from `crate::keys::hid_name`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyStroke {
    pub mods: u8,
    pub usage: u16,
}

/// Basic mouse actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseFn {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    DoubleClick,
}

/// Media/system (representative ones of the HID Consumer page 0x0C).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaFn {
    PlayPause,
    Stop,
    Next,
    Prev,
    VolumeUp,
    VolumeDown,
    Mute,
    Calculator,
    MyComputer,
    Browser,
    Mail,
    BrightnessUp,
    BrightnessDown,
    Refresh,
}

/// The input of one macro event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MacroInput {
    /// HID usage (keyboard page). Modifier keys are also expressed as usages (0xE0..0xE7).
    Key(u16),
    Mouse(MouseFn),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MacroEdge {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroEvent {
    pub input: MacroInput,
    pub edge: MacroEdge,
    /// Wait time (ms) **after** this event.
    pub delay_ms: u32,
}

/// One macro.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacroDef {
    pub name: String,
    pub events: Vec<MacroEvent>,
    /// Run mode (a value of `MacroCaps::modes`).
    pub mode: u8,
    /// The count for "specified count" modes (used only for `MacroCaps::repeat_modes`).
    pub repeat: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MacroCaps {
    /// Number of slots.
    pub slots: u8,
    /// Maximum number of events per macro.
    pub max_events: u16,
    /// Maximum delay (ms).
    pub max_delay_ms: u32,
    /// List of run modes (value, display name).
    pub modes: Vec<ChoiceOpt>,
    /// Values of the modes that take a count (repeat is editable only in these modes).
    pub repeat_modes: Vec<u8>,
    /// Whether macro names can be saved.
    pub names: bool,
    /// Whether mouse buttons can be put into a macro.
    pub mouse_events: bool,
}

// ====================================================================== Profiles

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileCaps {
    /// Number of onboard profiles (if 1, the UI does not show profile switching).
    pub count: u8,
    /// Whether they can be named (whether the name is stored on the device).
    pub renamable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileState {
    pub active: u8,
    /// Length is `ProfileCaps::count`. Empty string if there is no name.
    pub names: Vec<String>,
}

// ====================================================================== Mouse

/// One DPI stage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DpiStage {
    pub x: u32,
    /// Y when XY is split. `None` = same as X.
    pub y: Option<u32>,
    /// Color of the DPI indicator LED.
    pub color: Rgb,
    /// For devices where a stage can be disabled (skipped).
    pub enabled: bool,
}

/// DPI-related features.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpiFn {
    /// Cycle through DPI in order (loop).
    Cycle,
    Up,
    Down,
    /// A specified DPI only while held (sniper / DPI lock).
    Shift(u32),
}

/// Behavior when a macro is played from a button (the 3 kinds of mouse.fit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MacroRun {
    /// A specified number of times.
    Times(u16),
    /// Repeat while held.
    WhileHeld,
    /// Repeat until pressed again.
    Toggle,
}

/// The assignment of one mouse button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ButtonAction {
    Disabled,
    Mouse(MouseFn),
    Dpi(DpiFn),
    /// Rapid fire: `button` `count` times, at `interval_ms` intervals.
    Fire {
        button: MouseFn,
        count: u8,
        interval_ms: u16,
    },
    /// Keyboard key / shortcut.
    Key(KeyStroke),
    Media(MediaFn),
    Macro {
        slot: u8,
        run: MacroRun,
    },
    /// Cycle the polling rate in order.
    PollingCycle,
    /// Cycle the profile in order.
    ProfileCycle,
    /// Toggle the vertical roll (wheel direction).
    ScrollModeToggle,
    /// An assignment recorded on the device that the app cannot interpret.
    /// It can be displayed and round-tripped, but is not offered as a candidate for new assignments.
    Unsupported {
        /// The device-side type code.
        ty: u8,
        /// Raw parameter bytes.
        params: Vec<u8>,
    },
}

/// The kinds that can be chosen in button assignment (shown per device).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonActionKind {
    Disabled,
    Mouse,
    Dpi,
    Fire,
    Key,
    Media,
    Macro,
    PollingCycle,
    ProfileCycle,
    ScrollModeToggle,
}

impl ButtonAction {
    pub fn kind(&self) -> ButtonActionKind {
        match self {
            ButtonAction::Disabled => ButtonActionKind::Disabled,
            ButtonAction::Mouse(_) => ButtonActionKind::Mouse,
            ButtonAction::Dpi(_) => ButtonActionKind::Dpi,
            ButtonAction::Fire { .. } => ButtonActionKind::Fire,
            ButtonAction::Key(_) => ButtonActionKind::Key,
            ButtonAction::Media(_) => ButtonActionKind::Media,
            ButtonAction::Macro { .. } => ButtonActionKind::Macro,
            ButtonAction::PollingCycle => ButtonActionKind::PollingCycle,
            ButtonAction::ProfileCycle => ButtonActionKind::ProfileCycle,
            ButtonAction::ScrollModeToggle => ButtonActionKind::ScrollModeToggle,
            // Not offered as a candidate: treated as Disabled for display.
            ButtonAction::Unsupported { .. } => ButtonActionKind::Disabled,
        }
    }
}

/// One physical button.
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonSlot {
    /// Display name ("Left button" / "Side front" ...).
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MouseCaps {
    pub dpi_min: u32,
    pub dpi_max: u32,
    /// DPI step (10/50/100).
    pub dpi_step: u32,
    pub min_stages: usize,
    pub max_stages: usize,
    /// Separate X/Y DPI.
    pub xy_split: bool,
    /// LED color per stage.
    pub stage_colors: bool,
    /// Enable/disable per stage.
    pub stage_toggle: bool,
    /// Physical buttons (order = the order of `MouseState::buttons`).
    pub buttons: Vec<ButtonSlot>,
    /// The assignable kinds.
    pub button_kinds: Vec<ButtonActionKind>,
    /// The UI also checks "at least one left click is required".
    pub require_left_click: bool,
    pub macros: Option<MacroCaps>,
    /// Slot numbers (index into `buttons`) of the buttons that can return a press depth. If empty, the UI does not show it.
    /// The return value of `DeviceDriver::live_buttons` follows this order.
    pub live_buttons: Vec<usize>,
    /// Curve of speed-linked DPI (AdaDPI / Adaptive DPI). `MouseState::adaptive` and `Change::Adaptive` are a pair.
    pub adaptive_dpi: Option<AdaptiveCaps>,
}

/// One button that can return a press depth (magnetic switch, etc.). `DeviceDriver::live_buttons`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ButtonDepth {
    /// 0.0 (released) to 1.0 (fully pressed).
    pub depth: f32,
    /// Actuation point 0..1 (if known). The UI draws a line at this position.
    pub trigger: Option<f32>,
    /// Whether the device judges it as "pressed" (if known).
    pub pressed: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MouseState {
    pub stages: Vec<DpiStage>,
    pub active_stage: usize,
    /// Same order as `MouseCaps::buttons`.
    pub buttons: Vec<ButtonAction>,
    /// Slot order. Empty slots are None.
    pub macros: Vec<Option<MacroDef>>,
    /// Speed-linked DPI curve (Some only when `MouseCaps::adaptive_dpi` is present).
    #[serde(default)]
    pub adaptive: Option<AdaptiveCurve>,
}

/// One point of the speed-linked DPI curve: mouse speed → DPI multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdaptivePoint {
    /// Speed (device units. For LAMZU a 0–105 scale).
    pub speed: u16,
    /// DPI multiplier (1.0 = unchanged).
    pub ratio: f32,
}

/// State of speed-linked DPI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveCurve {
    pub enabled: bool,
    /// In ascending order of speed.
    pub points: Vec<AdaptivePoint>,
}

/// Feature declaration of speed-linked DPI.
#[derive(Clone, Debug, PartialEq)]
pub struct AdaptiveCaps {
    pub min_points: usize,
    pub max_points: usize,
    pub speed_max: u16,
    pub ratio_max: f32,
    /// Official presets (name, list of points).
    pub presets: Vec<(String, Vec<AdaptivePoint>)>,
}

// ====================================================================== Keyboard

/// The physical placement of one key.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutKey {
    /// Key identification on the protocol (for xsyd, the matrix `(row<<4)|col`).
    /// All arrays of `KeyboardState` are **in layout order**; this value is for the driver's reference.
    pub id: u16,
    /// Top-left coordinates and size (in 1u units).
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The default legend.
    pub label: String,
}

/// One assignment candidate (keyboard).
#[derive(Clone, Debug, PartialEq)]
pub struct KeyChoice {
    /// Device-specific key code (goes into `KeyAction::Code` as is).
    pub code: u32,
    /// The name shown in the list.
    pub label: String,
    /// Short name shown on the keycap (label if absent).
    pub short: Option<String>,
}

/// A group of assignment candidates ("Basic" / "Mouse" / "Media" / "Lighting" / "Layer" / "System" / "Gamepad").
#[derive(Clone, Debug, PartialEq)]
pub struct KeyGroup {
    pub name: String,
    pub keys: Vec<KeyChoice>,
}

/// The action assigned to a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyAction {
    /// Disabled (outputs nothing).
    None,
    /// Transparent (uses the layer below).
    Transparent,
    /// A code of `KeyboardCaps::catalog`.
    Code(u32),
    /// Modifiers + code (a key combination).
    Combo { mods: u8, code: u32 },
    /// Macro slot.
    Macro(u8),
}

/// Trigger mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerMode {
    /// Fixed actuation.
    Normal,
    /// Rapid Trigger.
    Rapid,
}

/// Magnetic switch settings of one key (all lengths in mm).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyPerf {
    pub mode: TriggerMode,
    /// The depth at which it turns ON when pressed.
    pub actuation: f32,
    /// The depth at which it turns OFF on release (None = same as actuation).
    pub release: Option<f32>,
    /// When RT is enabled: the depth of the first ON (First touch).
    pub rt_first: f32,
    /// RT: the amount of further pressing that turns it ON again.
    pub rt_press: f32,
    /// RT: the amount of release that turns it OFF.
    pub rt_release: f32,
    /// Top dead zone.
    pub top_deadzone: f32,
    /// Bottom dead zone.
    pub bottom_deadzone: f32,
    /// Continuous RT (RT continues until fully released).
    pub continuous: bool,
    /// Switch type (index into `PerfCaps::switch_types`).
    pub switch_type: u8,
}

impl Default for KeyPerf {
    fn default() -> Self {
        Self {
            mode: TriggerMode::Normal,
            actuation: 1.2,
            release: None,
            rt_first: 0.3,
            rt_press: 0.15,
            rt_release: 0.15,
            top_deadzone: 0.1,
            bottom_deadzone: 0.1,
            continuous: false,
            switch_type: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerfCaps {
    /// Total stroke (mm).
    pub travel: f32,
    /// Minimum settable value (mm).
    pub min: f32,
    /// Step (RT precision, 0.01 / 0.001 mm).
    pub step: f32,
    pub rapid_trigger: bool,
    /// Whether a release point separate from actuation is available.
    pub separate_release: bool,
    /// Whether RT has First touch.
    pub rt_first: bool,
    pub deadzones: bool,
    pub continuous_rt: bool,
    /// List of switch type (axis) names. If empty, the UI does not show it.
    pub switch_types: Vec<String>,
    /// Vendor axis IDs in the same order as `switch_types` (the key for looking up axis names online). Optional.
    pub switch_ids: Vec<u16>,
    /// Whether it can be set per key (if false, all keys at once).
    pub per_key: bool,
    /// Whether real-time press depth display (`DeviceDriver::live_travel`) is possible.
    pub live_travel: bool,
}

/// Raw stroke measurement data for one key (`DeviceDriver::live_axis`).
/// Fill in only the items the driver can provide (unsupported fields stay 0/false).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxisSample {
    /// Press depth in mm (same value as `live_travel`).
    pub route: f32,
    /// Raw ADC value (Hall sensor, etc.). 0 if unsupported.
    pub adc: u16,
    /// Key state flags (0 = released / non-0 = pressed, etc.; model-dependent).
    pub status: u8,
    /// Calibration state (0 = not done / non-0 = done, etc.; model-dependent).
    pub calibrated: u8,
}

/// The action in one phase of a DKS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DksPhase {
    #[default]
    Off,
    /// Press and release in that phase.
    Tap,
    /// Held down from that phase until the next boundary.
    Hold,
}

/// One row of a DKS (1 assigned key × 4 phases).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DksRow {
    pub code: u32,
    /// [start of press, bottom-out, start of release, full release]
    pub phases: [DksPhase; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SocdMode {
    /// Last input wins (overwrite).
    Last,
    /// First input wins.
    First,
    /// A wins.
    PriorityA,
    /// B wins.
    PriorityB,
    /// Cancel both.
    Neutral,
}

/// Advanced keys (Sparklink's DKS/MPT/MT/TGL/END/SOCD/RS).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Advanced {
    /// Dynamic Keystroke: up to 4 keys × 4 phases. `bottom` is the depth (mm) for bottom-out detection.
    Dks { rows: Vec<DksRow>, bottom: f32 },
    /// Multi-stage trigger: separate keys at 3 depths (depths increase monotonically).
    Mpt { depths: [f32; 3], codes: [u32; 3] },
    /// Tap/hold.
    Mt { tap: u32, hold: u32, hold_ms: u16 },
    /// Toggle. `delay_ms` is the official wire's `time` (the press duration of the toggle output).
    Tgl { code: u32, delay_ms: u16 },
    /// End trigger: outputs `code` on press and `code_release` on release, for `delay_ms`.
    /// Corresponds to the official wire `kcs:[press,release]` (`point` does not exist on the wire).
    End {
        code: u32,
        code_release: u32,
        delay_ms: u16,
    },
    /// SOCD: a pair of itself (A) and `other` (B, layout index).
    /// `delay_ms` is the official wire's `delay` (the delay of the reversal decision).
    Socd {
        other: usize,
        code_a: u32,
        code_b: u32,
        mode: SocdMode,
        delay_ms: u16,
    },
    /// Rapid Snap: of two keys, the one pressed deeper.
    Rs {
        other: usize,
        code_a: u32,
        code_b: u32,
        delay_ms: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdvancedKind {
    Dks,
    Mpt,
    Mt,
    Tgl,
    End,
    Socd,
    Rs,
}

impl Advanced {
    pub fn kind(&self) -> AdvancedKind {
        match self {
            Advanced::Dks { .. } => AdvancedKind::Dks,
            Advanced::Mpt { .. } => AdvancedKind::Mpt,
            Advanced::Mt { .. } => AdvancedKind::Mt,
            Advanced::Tgl { .. } => AdvancedKind::Tgl,
            Advanced::End { .. } => AdvancedKind::End,
            Advanced::Socd { .. } => AdvancedKind::Socd,
            Advanced::Rs { .. } => AdvancedKind::Rs,
        }
    }
}

/// An advanced key attached to a layout index (the order of `KeyboardCaps::layout`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdvancedKey {
    pub key: usize,
    pub layer: u8,
    pub binding: Advanced,
}

/// Declaration of one lighting effect.
#[derive(Clone, Debug, PartialEq)]
pub struct LightEffect {
    pub id: u8,
    pub name: String,
    /// Whether this effect lets you choose colors.
    pub color: bool,
    pub speed: bool,
    pub direction: bool,
    /// Whether it is a per-key color (custom) mode.
    pub per_key: bool,
}

impl LightEffect {
    pub fn new(id: u8, name: &str) -> Self {
        Self {
            id,
            name: name.into(),
            color: false,
            speed: true,
            direction: false,
            per_key: false,
        }
    }
    pub fn color(mut self) -> Self {
        self.color = true;
        self
    }
    pub fn still(mut self) -> Self {
        self.speed = false;
        self
    }
    pub fn dir(mut self) -> Self {
        self.direction = true;
        self
    }
    pub fn per_key(mut self) -> Self {
        self.per_key = true;
        self.speed = false;
        self
    }
}

/// A lighting area (main keys / decoration / mouse LED / receiver).
#[derive(Clone, Debug, PartialEq)]
pub struct LightArea {
    pub id: u8,
    pub name: String,
    pub effects: Vec<LightEffect>,
    pub brightness_max: u8,
    pub speed_max: u8,
    /// Direction options (none if empty).
    pub directions: Vec<String>,
    /// Whether it has an overall ON/OFF.
    pub switchable: bool,
    /// Zone names for upper/lower split lights (e.g. ["Upper light", "Lower light"]).
    /// A single zone if empty. If non-empty, `LightingState.zone_on` holds the open/closed state of each zone.
    pub zones: Vec<String>,
    /// Whether it can have per-key custom colors (keyboard areas only).
    pub per_key: bool,
    /// Whether it supports DirectDrive (per-LED live write `[5,5]`).
    /// When true, the UI shows a "Preview on device" toggle and can stream the lit image
    /// straight to the real LEDs (not persisted; stopping returns to firmware drive).
    pub direct_drive: bool,
    /// Whether it supports reading back the live lit colors (`DeviceDriver::live_lighting`).
    /// When true, the UI polls the device's current colors while shown and uses them for the preview.
    /// If false, the driver just returns `Unsupported`, which is harmless, but it is stated explicitly
    /// to keep to the principle of looking at caps only.
    pub live_readback: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightingState {
    /// Whether it is lit (true if any zone is open).
    pub on: bool,
    pub effect: u8,
    /// The color of the selected palette slot (`palette[palette_sel]`).
    pub color: Rgb,
    pub brightness: u8,
    pub speed: u8,
    pub direction: u8,
    /// Per-key colors of the custom color overlay (layout index → color).
    /// Independent of the base effect — isCustom cells overlay on top in every mode.
    pub per_key: BTreeMap<usize, Rgb>,
    /// Open/closed state of each upper/lower zone (same order as `LightArea.zones`. If empty, only `on` is looked at).
    #[serde(default)]
    pub zone_on: Vec<bool>,
    /// Colors of all palette slots (if empty, the palette is unsupported).
    #[serde(default)]
    pub palette: Vec<Rgb>,
    /// Index of the selected palette slot.
    #[serde(default)]
    pub palette_sel: u8,
    /// Saturation (ColorCorrection, the official saturation). Some only on models
    /// readable with `[5,1,area,2]` — R,G,B each 0-100. If None, the UI does not show it.
    #[serde(default)]
    pub color_correction: Option<Rgb>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardCaps {
    pub layout: Vec<LayoutKey>,
    /// Layer names ("Main", "Fn1" ...).
    pub layers: Vec<String>,
    /// Assignment candidates (lined up in the UI in group order).
    pub catalog: Vec<KeyGroup>,
    /// Whether key combinations (modifier + key) can be assigned.
    pub combos: bool,
    pub perf: Option<PerfCaps>,
    /// Usable advanced keys.
    pub advanced: Vec<AdvancedKind>,
    pub macros: Option<MacroCaps>,
}

impl KeyboardCaps {
    /// Display name of a code (looked up from the catalog).
    pub fn code_label(&self, code: u32) -> Option<(&str, Option<&str>)> {
        self.catalog
            .iter()
            .flat_map(|g| g.keys.iter())
            .find(|k| k.code == code)
            .map(|k| (k.label.as_str(), k.short.as_deref()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyboardState {
    /// `[layer][layout index]`.
    pub keymap: Vec<Vec<KeyAction>>,
    /// In layout index order.
    pub perf: Vec<KeyPerf>,
    pub advanced: Vec<AdvancedKey>,
    pub macros: Vec<Option<MacroDef>>,
}

// ====================================================================== Audio (USB DAC)

/// Filter type of an EQ band (the meaning on the UI; conversion to the wire index is the driver's job).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeqFilter {
    LowShelf,
    Peak,
    HighShelf,
    LowPass,
    HighPass,
}

impl PeqFilter {
    pub fn label(self) -> &'static str {
        match self {
            PeqFilter::LowShelf => "LS",
            PeqFilter::Peak => "PK",
            PeqFilter::HighShelf => "HS",
            PeqFilter::LowPass => "LP",
            PeqFilter::HighPass => "HP",
        }
    }
}

/// One band of the parametric EQ.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeqBand {
    /// Center frequency in Hz.
    pub freq: u16,
    /// Q value (kept even for filter types that do not use Q).
    pub q: f32,
    /// Gain in dB.
    pub gain: f32,
    /// Filter type (a value contained in `PeqCaps::filters`).
    pub filter: PeqFilter,
}

/// Feature declaration of the parametric EQ. The UI shows the EQ editor when `peq` is Some.
#[derive(Clone, Debug, PartialEq)]
pub struct PeqCaps {
    /// Number of bands.
    pub bands: u8,
    /// Frequency range (Hz).
    pub freq_min: u16,
    pub freq_max: u16,
    /// Q range.
    pub q_min: f32,
    pub q_max: f32,
    /// Gain range (dB).
    pub gain_min: f32,
    pub gain_max: f32,
    /// Usable filter types.
    pub filters: Vec<PeqFilter>,
}

// ====================================================================== Overall

/// The whole picture of the features a device has.
#[derive(Clone, Debug, PartialEq)]
pub struct Capabilities {
    pub settings: Vec<SettingSpec>,
    pub actions: Vec<ActionSpec>,
    pub profiles: Option<ProfileCaps>,
    pub mouse: Option<MouseCaps>,
    pub keyboard: Option<KeyboardCaps>,
    /// Lighting areas (shared by mice and keyboards).
    pub lighting: Vec<LightArea>,
    /// Has a battery (used to decide whether to show the remaining charge in the sidebar).
    pub battery: bool,
    /// Parametric EQ (USB DAC, etc.). `DeviceState::peq` and `Change::Peq*` are a pair.
    pub peq: Option<PeqCaps>,
    /// Source of the official firmware (for update notifications). None for models without one.
    pub firmware_feed: Option<FirmwareFeed>,
}

/// Source of the official firmware (only for update "notifications"; no writing is done).
#[derive(Clone, Debug, PartialEq)]
pub enum FirmwareFeed {
    /// mouse.fit: `Config/<ui_folder>/FWLINK.txt` holds one line with the zip URL.
    MouseFit { ui_folder: String },
    /// Sparklink: `api.sparklinkplayjoy.com/api/v1/getKeyboardFirmware`.
    /// board_id is taken from the device's DEVICE_INFO response (`DeviceInfo::board_id`).
    Sparklink {
        vid: u16,
        pid: u16,
        board_id: Option<u32>,
    },
}

impl Capabilities {
    pub fn empty() -> Self {
        Self {
            settings: vec![],
            actions: vec![],
            profiles: None,
            mouse: None,
            keyboard: None,
            lighting: vec![],
            battery: false,
            peq: None,
            firmware_feed: None,
        }
    }
    pub fn setting(&self, id: &str) -> Option<&SettingSpec> {
        self.settings.iter().find(|s| s.id == id)
    }
}

/// A snapshot of all settings. It can be written to a file via serde as is (the UI's "Export/Import").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub profile: ProfileState,
    /// `SettingSpec::id` → value. Settings that could not be read are left out (the UI shows that row disabled).
    pub settings: BTreeMap<String, SettingValue>,
    pub mouse: Option<MouseState>,
    pub keyboard: Option<KeyboardState>,
    /// `LightArea::id` → state.
    pub lighting: BTreeMap<u8, LightingState>,
    /// Bands in the same order as `PeqCaps::bands`. Some only when `Capabilities::peq` is present.
    #[serde(default)]
    pub peq: Option<Vec<PeqBand>>,
}

/// A fine-grained change from the UI to the driver. The driver only needs to write **this one**.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Change {
    /// Generic setting.
    Setting {
        id: String,
        value: SettingValue,
    },
    /// Onboard profile switch. After success the UI rereads with `read()`.
    Profile(u8),
    ProfileName {
        index: u8,
        name: String,
    },

    // ---- Mouse
    /// The whole set of DPI stages (count, values, colors) and the current stage.
    DpiStages {
        stages: Vec<DpiStage>,
        active: usize,
    },
    /// Switch only the current stage.
    ActiveStage(usize),
    Button {
        slot: usize,
        action: ButtonAction,
    },

    // ---- Shared
    /// One macro (None deletes it). `MouseState::macros` for a mouse, `KeyboardState::macros` for a keyboard.
    Macro {
        slot: usize,
        def: Option<MacroDef>,
    },
    Lighting {
        area: u8,
        state: LightingState,
    },

    // ---- Keyboard
    KeyMap {
        layer: u8,
        key: usize,
        action: KeyAction,
    },
    /// Return that layer to the default keymap.
    KeyMapReset {
        layer: u8,
    },
    /// The same travel settings for multiple keys.
    KeyPerf {
        keys: Vec<usize>,
        perf: KeyPerf,
    },
    /// Set/clear an advanced key (binding None clears it).
    Advanced {
        key: usize,
        layer: u8,
        binding: Option<Advanced>,
    },

    // ---- Audio (USB DAC)
    /// One EQ band.
    Peq {
        band: usize,
        value: PeqBand,
    },
    /// All EQ bands at once (for preset application, import and undo).
    PeqAll {
        bands: Vec<PeqBand>,
    },

    // ---- Mouse (speed-linked DPI)
    /// Enable/disable speed-linked DPI, and its curve.
    Adaptive(AdaptiveCurve),
}

impl Change {
    /// Whether running only the later change still applies every setting of the earlier one.
    pub fn is_superseded_by(&self, next: &Change) -> bool {
        self.same_target(next) && !matches!((self, next), (Self::PeqAll { .. }, Self::Peq { .. }))
    }

    /// Whether both changes target the same thing. A bulk EQ change and a single-band change overlap in target.
    pub fn same_target(&self, other: &Change) -> bool {
        use Change::*;
        match (self, other) {
            (Setting { id: a, .. }, Setting { id: b, .. }) => a == b,
            (DpiStages { .. }, DpiStages { .. }) => true,
            (ActiveStage(_), ActiveStage(_)) => true,
            (Button { slot: a, .. }, Button { slot: b, .. }) => a == b,
            (Macro { slot: a, .. }, Macro { slot: b, .. }) => a == b,
            (Lighting { area: a, .. }, Lighting { area: b, .. }) => a == b,
            (
                KeyMap {
                    layer: la, key: ka, ..
                },
                KeyMap {
                    layer: lb, key: kb, ..
                },
            ) => la == lb && ka == kb,
            (KeyPerf { keys: a, .. }, KeyPerf { keys: b, .. }) => a == b,
            (
                Advanced {
                    key: a, layer: la, ..
                },
                Advanced {
                    key: b, layer: lb, ..
                },
            ) => a == b && la == lb,
            (ProfileName { index: a, .. }, ProfileName { index: b, .. }) => a == b,
            (Peq { band: a, .. }, Peq { band: b, .. }) => a == b,
            (PeqAll { .. }, PeqAll { .. }) => true,
            (Peq { .. }, PeqAll { .. }) | (PeqAll { .. }, Peq { .. }) => true,
            (Adaptive(_), Adaptive(_)) => true,
            _ => false,
        }
    }
}
