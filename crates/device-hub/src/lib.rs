//! Device HUB — GPUI frontend (flat design).
//!
//! - `app` … window shell (Shell): state, routing, sidebar, event wiring
//! - `pages/*` … the screens (`impl Shell`)
//! - `overlay` … popups, modals, toasts
//! - `ui` … components (card, row, toggle, slider, segmented control…)
//! - `surface` … flat surfaces (cards, bars, controls, popups)
//! - `motion` … spring animation and element position tracking
//! - `store` … bridge to the backend (devhub-core contract); one worker thread per device
//! - `theme` / `prefs` / `keycode`

pub mod app;
pub mod audio;
pub mod cursor;
pub mod fwstate;
pub mod keycode;
pub mod livefx;
pub mod motion;
pub mod overlay;
pub mod pages;
pub mod palette;
pub mod prefs;
pub mod presets;
pub mod procmon;
pub use devhub_obs::rawinput;
pub mod store;
pub mod surface;
pub mod theme;
pub mod ui;
