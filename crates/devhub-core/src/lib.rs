//! devhub-core: the core of Device HUB (no GUI dependency).
//!
//! - `model` … data types for device settings (the common language of UI and drivers, contract v2)
//! - `backend` … the `HubBackend` / `DeviceDriver` traits (the backend contract)
//! - `keys` … HID usage ⇔ name table
//! - `registry` … table of known devices + physical layouts
//! - `demo` … demo backend that drives every screen without hardware (a sample of how to write caps)

pub mod backend;
pub mod demo;
pub mod keys;
pub mod lamzu;
pub mod model;
pub mod peq;
pub mod registry;

pub use backend::{DeviceDriver, HubBackend, HubError, HubEvent, HubResult};
pub use model::*;
