# Audio (USB DAC / PEQ) frontend — TP55 support

Owner: frontend. `crates/device-hub/src/pages/audio.rs`.
The backend (walkplay.rs / peq.rs / registry / demo) was implemented by another agent, and the contract is unchanged.

## Contract in use
- `Capabilities::peq: Option<PeqCaps>` → if present, show the EQ card (band count, ranges, and usable filter types all come from caps).
- `DeviceState::peq` / `Change::Peq { band, value }` / `Change::PeqAll { bands }`.
- The preamp is the generic setting `eq_volume` (Slider). It is shown inside the EQ card, so it is excluded from the generic card on models that have peq.
- The `eq_def` action is also merged into the EQ card's "DEF (official)" preset, so it is excluded from generic too.
- The graph uses `devhub_core::peq::response_db` (the same biquad / 96kHz as the driver), so it matches the real device's response.

## Screen
1. Response graph: composite curve + the curve of the selected band alone (filled). Numbered handles:
   drag = change frequency + gain at once (delta drag; a click alone does not move them), wheel = Q. Hovering shows Hz / composite dB at the cursor position.
2. Preamp row: shows "headroom / possible clipping" from the peak of the composite curve; "Auto" sets the largest value that does not clip.
3. Band list (equivalent to the official vertical column): type / frequency / gain / Q. Click to select.
4. Selected-band editing: ◀▶ to switch, type (LS/PK/HS/LP/HP), reset to 0dB, frequency (log slider) / Q / gain as numeric input + slider.
5. Toolbar: presets (8 built in: Flat, DEF, Bass boost, V-shape, Vocal, FPS footsteps, Harman-style, Reduce treble + saved ones), save, delete,
   **paste AutoEQ / Equalizer APO (ParametricEQ.txt)**, copy in APO format.
   Saved presets are `eq_presets` in `prefs.json` (shared across devices). The preamp of built-in presets is computed automatically from the peak.

## Generic parts added
- `Shell::begin_drag_xy` / `CbXY` (2D drag. `Drag::xy`).
- `Prefs::eq_presets` / `prefs::EqPreset`.
- Icon for the sidebar / home / header: `DriverKind::Walkplay` uses `headphones`.

## Requests to the backend (optional)
- If a band ON/OFF (bypass) can be held on the wire, please add `PeqBand::enabled` (the UI would only add a toggle to the band list).
- The official "ONLINE / SHARE" (cloud EQ) is not supported because it uses the login-only walkplay-api. If it is ever done, make it a separate crate, like devhub-update.
- When adding another WalkPlay-family DAC, just add its VID/PID to the registry and set `PeqCaps` to that model's values; the UI adapts automatically.
