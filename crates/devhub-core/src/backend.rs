//! The contract with the backend (v2).
//!
//! The frontend (device-hub) knows **only `HubBackend` / `DeviceDriver`**.
//! All HID and protocol details stay inside the implementation side (devhub-hid).
//!
//! # Threading and how calls arrive (for implementers)
//!
//! - Everything is a **synchronous API**. The UI calls it from a dedicated per-device worker thread,
//!   so it may block on HID round trips (a few ms to several hundred ms). It is never called from the UI thread.
//! - Methods of the same driver are never called **concurrently** (one worker).
//!   However, `HubBackend::scan` is called in parallel from another (polling) thread.
//! - `apply` arrives continuously while a slider is being dragged. The UI drops stale changes
//!   to the same target before sending (`Change::is_superseded_by`), so drivers need no throttling.
//! - Errors are `HubError`. The UI shows the Display string as-is in a toast.

use crate::model::*;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

// ------------------------------------------------------------------ errors

#[derive(Debug)]
pub enum HubError {
    /// The device ID was not found (unplugged, etc.).
    NotFound(DeviceId),
    /// The device/driver does not have this feature.
    Unsupported(&'static str),
    /// Protocol not implemented.
    NotImplemented(&'static str),
    /// Communication error.
    Io(String),
    /// The response is corrupt, no ACK arrived, etc.
    Protocol(String),
    /// An argument is outside the caps range, etc.
    Invalid(String),
    /// The device was disconnected / the wireless device is asleep.
    Disconnected,
}

impl fmt::Display for HubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HubError::NotFound(id) => write!(f, "Device not found: {id}"),
            HubError::Unsupported(what) => write!(f, "This feature is not supported: {what}"),
            HubError::NotImplemented(what) => write!(f, "Not implemented: {what}"),
            HubError::Io(e) => write!(f, "Communication error: {e}"),
            HubError::Protocol(e) => write!(f, "Response error: {e}"),
            HubError::Invalid(e) => write!(f, "Invalid value: {e}"),
            HubError::Disconnected => write!(f, "The device was disconnected"),
        }
    }
}

impl std::error::Error for HubError {}

pub type HubResult<T> = Result<T, HubError>;

// ------------------------------------------------------------------ events

/// Asynchronous notification from the backend to the UI (optional).
/// Call it from the `HubBackend::watch` callback only when the implementation can detect this actively.
/// Even if nothing is called, the UI keeps working with a `scan` every 2 seconds and a `battery` every 30 seconds.
#[derive(Clone, Debug)]
pub enum HubEvent {
    /// Plug/unplug. The UI runs `scan` again.
    DevicesChanged,
    /// A setting changed on the device side (DPI or profile changed with the device's own buttons, etc.).
    /// The UI runs `read` again.
    StateChanged(DeviceId),
    Battery(DeviceId, Battery),
    /// A `DeviceInfo` that changed inside the driver (sleep state, a battery value obtained in the background, etc.).
    /// The UI reflects it in the session and the device list.
    Info(DeviceId, Box<DeviceInfo>),
    /// Progress of a long operation (firmware update, calibration, bulk write).
    /// `fraction` is 0..1 (None if unknown = indeterminate spinner). `done` removes the display.
    /// Sent from the driver via the `watch` callback while `run_action` is running.
    Progress {
        id: DeviceId,
        label: String,
        fraction: Option<f32>,
        done: bool,
    },
}

// ------------------------------------------------------------------ traits

/// Entry point of the whole hub. Created by `devhub_hid::connect()`.
pub trait HubBackend: Send + Sync {
    /// List of devices currently visible. Returning unsupported devices with `supported=false` shows them in the UI.
    /// The UI calls this every 2 seconds, so keep it light (HID enumeration only; do not query the device).
    fn scan(&self) -> HubResult<Vec<DeviceInfo>>;

    /// The driver for one device. May return the same instance for the same ID.
    fn driver(&self, id: &DeviceId) -> HubResult<Arc<dyn DeviceDriver>>;

    /// Discard a cached driver (call after Disconnected).
    /// Re-plugging into the same port keeps the device path unchanged, but the inner handle
    /// is dead, so it must be reopened on the next `driver()`.
    fn invalidate(&self, _id: &DeviceId) {}

    /// Register for event notifications (optional).
    fn watch(&self, _cb: Box<dyn Fn(HubEvent) + Send + Sync>) {}

    /// Tells the UI this is not real hardware (for demos, etc.).
    fn is_demo(&self) -> bool {
        false
    }
}

/// A conversation with one device.
pub trait DeviceDriver: Send + Sync {
    /// The name tag (filled in with the information available after opening, e.g. firmware).
    fn info(&self) -> DeviceInfo;

    /// Declares the features. Called **only once, when opened**, and the UI caches it for the session.
    /// (If it changes, e.g. on profile switch, emit `HubEvent::StateChanged` to have it reread.)
    fn caps(&self) -> Capabilities;

    /// Read all settings. Generic settings that could not be read must be left out of the map, so the UI shows that row disabled.
    fn read(&self) -> HubResult<DeviceState>;

    /// Apply one change immediately. Success = the device accepted it (send success if there is no ACK).
    /// On failure the UI shows a toast and runs `read` to bring the display back in line with the device.
    fn apply(&self, change: &Change) -> HubResult<()>;

    /// Run an `ActionSpec`. `file` is the chosen path when `ActionSpec::file` is Some.
    fn run_action(&self, id: &str, file: Option<&Path>) -> HubResult<ActionOutcome> {
        let _ = (id, file);
        Err(HubError::Unsupported("action"))
    }

    /// Latest battery.
    fn battery(&self) -> HubResult<Battery> {
        Err(HubError::Unsupported("battery"))
    }

    /// Real-time press depth (mm, in layout index order). The UI calls this at about 30Hz during the "stroke test",
    /// and only when `PerfCaps::live_travel` is set.
    fn live_travel(&self) -> HubResult<Vec<f32>> {
        Err(HubError::Unsupported("live_travel"))
    }

    /// Raw stroke measurement data (route mm + ADC + state, in layout index order).
    /// For drivers that support it, the UI polls this instead of `live_travel` to show details.
    fn live_axis(&self) -> HubResult<Vec<AxisSample>> {
        Err(HubError::Unsupported("live_axis"))
    }

    /// Latest press depth, in the same order as `MouseCaps::live_buttons`. The UI calls this at 60Hz
    /// on the Test page. For devices that deliver data by notification (push), return only the latest value
    /// stored by the receive thread, and do not hit HID here.
    fn live_buttons(&self) -> HubResult<Vec<ButtonDepth>> {
        Err(HubError::Unsupported("live_buttons"))
    }

    /// Measured input report rate (Hz). The count of unsolicited reports the device streams,
    /// counted by the receive thread: a measured value, not the configured one. None for devices
    /// without a stream or if not implemented. The UI calls this every 0.5 to 1 second. Do not hit HID.
    fn live_report_hz(&self) -> Option<f32> {
        None
    }

    /// Actual per-key settings (layout order, mm). Drivers that cannot fetch them return Unsupported.
    fn live_key_perf(&self) -> HubResult<Vec<KeyPerf>> {
        Err(HubError::Unsupported("live_key_perf"))
    }

    /// Fast-actuation setting of the magnetic buttons (in live_buttons order, 0 = disabled).
    fn live_button_fast(&self) -> HubResult<Vec<Option<u8>>> {
        Err(HubError::Unsupported("live_button_fast"))
    }

    /// Per-key actuation point (mm, layout order). RT is First touch.
    fn live_actuation(&self) -> HubResult<Vec<f32>> {
        Err(HubError::Unsupported("live_actuation"))
    }

    /// DirectDrive: live per-LED write (no response, not persisted; temporarily overrides the firmware
    /// animation). `cells` maps key index to color, and expanding to LED cells is up to the driver.
    /// The UI streams frames (about 5fps) only when `LightArea::direct_drive` is set.
    /// Drivers without support return Unsupported.
    fn direct_drive(&self, _area: u8, _cells: &BTreeMap<usize, Rgb>) -> HubResult<()> {
        Err(HubError::Unsupported("direct_drive"))
    }

    /// Leave DirectDrive and return to the firmware animation.
    /// Supporting drivers resend the saved lighting settings to restore the device's lighting.
    fn direct_drive_stop(&self, _area: u8) -> HubResult<()> {
        Err(HubError::Unsupported("direct_drive"))
    }

    /// Read back the live lighting colors: key index → **the color actually lit right now**
    /// (the current frame of the effect rendered by the firmware, including user custom colors).
    /// The same mechanism as the official HUB preview: while the Lighting page is shown, the UI calls this at
    /// ~6Hz and draws exactly what the real device looks like. It is not a simulation.
    /// Unsupported drivers return Unsupported; the UI then switches to a local approximate preview.
    fn live_lighting(&self, _area: u8) -> HubResult<BTreeMap<usize, Rgb>> {
        Err(HubError::Unsupported("live_lighting"))
    }
}
