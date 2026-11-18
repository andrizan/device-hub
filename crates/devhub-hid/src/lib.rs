//! devhub-hid: the layer that talks to real devices.
//!
//! - `connect()` … opens hidapi and returns a `RealHub`
//! - `RealHub::scan` … reports interfaces matching `devhub_core::registry::KNOWN` as
//!   `DeviceInfo` and records ID → HID path in `paths`
//!   (driver kinds that cannot be opened over HID get `supported: false`).
//! - `RealHub::driver` … opens the recorded path for an ID, creates the driver and caches it.
//!   The same ID gets the same `Arc` (the UI calls it from one worker per device).

use devhub_core::backend::*;
use devhub_core::model::*;
use devhub_core::registry;
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex};

// mousefit new protocol (64B feature report) driver — every model observed (5219 wired,
// 3854 dongle) lacks feature reports on Windows, so it is currently not assigned to any model.
// Kept as an implementation based on the JS bundle analysis; use it here if a model with feature reports appears.
/// ATK family driver. Public because the diagnostic example (atkread) also uses it.
#[doc(hidden)]
pub mod atk;
/// Bluetooth Classic (RFCOMM) channel. Public because the diagnostic example
/// (`btprobe`) uses it too.
#[doc(hidden)]
pub mod bt;
mod earfun;
#[doc(hidden)]
pub mod lamzu;
#[allow(dead_code)]
mod mousefit;
mod mousefit8;
mod ry;
/// Raw HID channel. Public so diagnostic examples (axraw, etc.) can send frames directly.
#[doc(hidden)]
pub mod transport;
mod walkplay;
mod xsyd;

/// Entry point of the real-device backend. Err if HID cannot be opened (the UI falls back to demo).
pub fn connect() -> HubResult<Arc<dyn HubBackend>> {
    let api = transport::open()?;
    Ok(Arc::new(RealHub {
        api: Mutex::new(api),
        paths: Mutex::new(HashMap::new()),
        drivers: Mutex::new(HashMap::new()),
        names: Mutex::new(HashMap::new()),
        watcher: Mutex::new(None),
        detected: Arc::new(Mutex::new(HashMap::new())),
    }))
}

/// Whether the implementation for each driver kind is done.
fn driver_ready(kind: DriverKind) -> bool {
    match kind {
        DriverKind::Xsyd => true,
        DriverKind::Mousefit => true,
        DriverKind::Walkplay => true,
        DriverKind::Atk => true,
        DriverKind::Lamzu => true,
        DriverKind::Ry => true,
        DriverKind::Earfun => true,
        DriverKind::Unsupported => false,
    }
}

pub struct RealHub {
    api: Mutex<hidapi::HidApi>,
    /// `DeviceId.0` → HID path (as of the last scan).
    paths: Mutex<HashMap<String, CString>>,
    /// `DeviceId.0` → driver (an opened one is reused).
    drivers: Mutex<HashMap<String, Arc<dyn DeviceDriver>>>,
    /// `DeviceId.0` → product name reported by HID (recorded at scan time).
    names: Mutex<HashMap<String, String>>,
    /// `DeviceId.0` → determined model behind the dongle (model, display name).
    /// Keeps the result the driver determined from the EID so that driver recreation
    /// (disconnect/invalidate) does not revert to the registry default.
    detected: Arc<Mutex<HashMap<String, (String, String)>>>,
    /// Notification sink registered by `watch` (handed to drivers).
    watcher: Mutex<Option<Watcher>>,
}

/// Event sink handed to drivers.
type Watcher = Arc<dyn Fn(HubEvent) + Send + Sync>;

impl HubBackend for RealHub {
    fn scan(&self) -> HubResult<Vec<DeviceInfo>> {
        let mut api = self.api.lock().unwrap_or_else(|p| p.into_inner());
        api.refresh_devices()
            .map_err(|e| HubError::Io(e.to_string()))?;
        let mut out: Vec<DeviceInfo> = Vec::new();
        let mut paths = self.paths.lock().unwrap_or_else(|p| p.into_inner());
        let mut names = self.names.lock().unwrap_or_else(|p| p.into_inner());
        let prev_paths = std::mem::take(&mut *paths);
        names.clear();
        for d in api.device_list() {
            let Some(spec) = registry::match_device(
                d.vendor_id(),
                d.product_id(),
                d.usage_page(),
                d.usage(),
                d.product_string(),
            ) else {
                continue;
            };
            let serial = d
                .serial_number()
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let id = DeviceId(format!(
                "hid:{:04x}:{:04x}:{}",
                d.vendor_id(),
                d.product_id(),
                serial
                    .clone()
                    .unwrap_or_else(|| format!("{:08x}", fxhash(d.path().to_bytes())))
            ));
            if out.iter().any(|o| o.id == id) {
                continue;
            }
            paths.insert(id.0.clone(), d.path().to_owned());
            // Prefer the HID product string for the display name (otherwise the registry name).
            // A dongle, however, announces the receiver's own name, so prefer the registry (paired model name).
            let name = match spec.transport {
                Transport::Wireless24G | Transport::Bluetooth => spec.name.into(),
                _ => d
                    .product_string()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| spec.name.into()),
            };
            names.insert(id.0.clone(), name.clone());
            out.push(DeviceInfo {
                id,
                kind: spec.kind,
                name,
                vendor: spec.vendor.into(),
                model: spec.model.into(),
                vid: spec.vid,
                pid: spec.pid,
                transport: spec.transport,
                firmware: None,
                dongle_firmware: None,
                serial,
                driver: spec.driver,
                battery: None,
                supported: driver_ready(spec.driver),
                asleep: false,
                board_id: None,
            });
        }
        // Bluetooth audio devices: paired, connected right now, and known by
        // name. Listing them costs no radio traffic (the OS answers from its
        // own table); the settings channel is only opened by `driver()`.
        for d in bt::paired().unwrap_or_default() {
            if !d.connected {
                continue;
            }
            let Some(spec) = registry::match_bt(&d.name) else {
                continue;
            };
            let id = DeviceId(format!("bt:{:012x}", d.addr));
            // Recorded like a HID path so the cache check below keeps the
            // open channel for as long as the device stays connected.
            paths.insert(
                id.0.clone(),
                CString::new(id.0.as_bytes()).unwrap_or_default(),
            );
            names.insert(id.0.clone(), d.name.clone());
            out.push(DeviceInfo {
                id,
                kind: spec.kind,
                name: d.name,
                vendor: spec.vendor.into(),
                model: spec.model.into(),
                vid: 0,
                pid: 0,
                transport: Transport::Bluetooth,
                firmware: None,
                dongle_firmware: None,
                serial: None,
                driver: spec.driver,
                battery: None,
                supported: driver_ready(spec.driver),
                asleep: false,
                board_id: None,
            });
        }
        // The cache of an unplugged/replugged device is a dead handle — keep only those whose path
        // matches the previous one, drop the rest and reopen on the next driver().
        self.drivers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|id, _| paths.get(id).is_some() && prev_paths.get(id) == paths.get(id));
        Ok(out)
    }

    fn watch(&self, cb: Box<dyn Fn(HubEvent) + Send + Sync>) {
        *self.watcher.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::from(cb));
    }

    fn driver(&self, id: &DeviceId) -> HubResult<Arc<dyn DeviceDriver>> {
        if let Some(d) = self
            .drivers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id.0)
        {
            return Ok(d.clone());
        }
        if let Some(addr) = bt_addr(id) {
            return self.bt_driver(id, addr);
        }
        let path = self
            .paths
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id.0)
            .cloned()
            .ok_or_else(|| HubError::NotFound(id.clone()))?;
        let (vid, pid) = parse_id(id).ok_or_else(|| HubError::NotFound(id.clone()))?;
        let spec = known_spec(vid, pid).ok_or_else(|| HubError::NotFound(id.clone()))?;
        let api = self.api.lock().unwrap_or_else(|p| p.into_inner());
        let dev = api
            .open_path(&path)
            .map_err(|e| HubError::Io(format!("open {}: {e}", spec.name)))?;
        drop(api);
        let name = self
            .names
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id.0)
            .cloned()
            .unwrap_or_else(|| spec.name.into());
        let mut info = DeviceInfo {
            id: id.clone(),
            kind: spec.kind,
            name,
            vendor: spec.vendor.into(),
            model: spec.model.into(),
            vid: spec.vid,
            pid: spec.pid,
            transport: spec.transport,
            firmware: None,
            dongle_firmware: None,
            serial: None,
            driver: spec.driver,
            battery: None,
            supported: true,
            asleep: false,
            board_id: None,
        };
        // If the model behind the dongle is already determined, restore it (so a recreation also opens with the right caps).
        if let Some((m, n)) = self
            .detected
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id.0)
        {
            info.model = m.clone();
            info.name = n.clone();
        }
        let events = self
            .watcher
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let drv: Arc<dyn DeviceDriver> = match spec.driver {
            // mouse.fit models have no feature report behind a dongle, and also when wired (real 5219 device)
            // → all models use the report-8/EEPROM path.
            // The driver for the new protocol (64B feature report) is kept in mousefit.rs.
            DriverKind::Mousefit => {
                Arc::new(mousefit8::open(dev, info, events, self.detected.clone())?)
            }
            DriverKind::Xsyd => Arc::new(xsyd::open(dev, info)?),
            DriverKind::Walkplay => Arc::new(walkplay::open(dev, info)?),
            DriverKind::Atk => Arc::new(atk::open(dev, info, events)?),
            DriverKind::Lamzu => Arc::new(lamzu::open(dev, info, events)?),
            DriverKind::Ry => Arc::new(ry::open(dev, info, events)?),
            // Bluetooth drivers are opened by address, never through HID.
            DriverKind::Earfun => return Err(HubError::NotFound(id.clone())),
            DriverKind::Unsupported => {
                return Err(HubError::NotImplemented("a driver for this device"))
            }
        };
        self.drivers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.0.clone(), drv.clone());
        Ok(drv)
    }

    fn invalidate(&self, id: &DeviceId) {
        self.drivers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id.0);
    }
}

impl RealHub {
    /// Open the settings channel of a Bluetooth device found by `scan`.
    fn bt_driver(&self, id: &DeviceId, addr: u64) -> HubResult<Arc<dyn DeviceDriver>> {
        let name = self
            .names
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id.0)
            .cloned()
            .ok_or_else(|| HubError::NotFound(id.clone()))?;
        let spec = registry::match_bt(&name).ok_or_else(|| HubError::NotFound(id.clone()))?;
        let info = DeviceInfo {
            id: id.clone(),
            kind: spec.kind,
            name,
            vendor: spec.vendor.into(),
            model: spec.model.into(),
            vid: 0,
            pid: 0,
            transport: Transport::Bluetooth,
            firmware: None,
            dongle_firmware: None,
            serial: None,
            driver: spec.driver,
            battery: None,
            supported: true,
            asleep: false,
            board_id: None,
        };
        let drv: Arc<dyn DeviceDriver> = match spec.driver {
            DriverKind::Earfun => Arc::new(earfun::open(addr, info)?),
            _ => {
                return Err(HubError::NotImplemented(
                    "a driver for this Bluetooth device",
                ))
            }
        };
        self.drivers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.0.clone(), drv.clone());
        Ok(drv)
    }
}

/// `bt:<12 hex digits>` → the Bluetooth address.
fn bt_addr(id: &DeviceId) -> Option<u64> {
    u64::from_str_radix(id.0.strip_prefix("bt:")?, 16).ok()
}

/// `hid:<vid>:<pid>:<serial>` → (vid, pid).
fn parse_id(id: &DeviceId) -> Option<(u16, u16)> {
    let mut it = id.0.split(':');
    if it.next()? != "hid" {
        return None;
    }
    let vid = u16::from_str_radix(it.next()?, 16).ok()?;
    let pid = u16::from_str_radix(it.next()?, 16).ok()?;
    Some((vid, pid))
}

/// VID/PID → registry entry (the usage condition is assumed to have been met at scan time).
fn known_spec(vid: u16, pid: u16) -> Option<&'static registry::KnownDevice> {
    registry::KNOWN
        .iter()
        .find(|k| k.vid == vid && k.pid == pid && k.vid != 0xFFFF)
}

/// Stable hash of the path string (for the ID of devices without a serial).
fn fxhash(b: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for x in b {
        h ^= *x as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}
