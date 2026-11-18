# Live lighting (DirectDrive) and keyboard UI improvements — handoff (2026-11-15)

For the additional audit and the implementation order, see the [DirectDrive re-audit](DIRECTDRIVE_AUDIT.md). What follows is the initial implementation and the existing hardware verification records.

The frontend is implemented (owner: frontend). Another agent should work through "§3 Requests to the backend" in this file.
The contract (devhub-core) is **unchanged**. It works with only the existing `live_travel` / `direct_drive` / `direct_drive_stop`.

## 1. What can be done now (features the official HUB does not have)

The "Live" tab of the SU66 Max. The PC reads the press depth (mm), computes a color per key, and streams it straight to the LEDs with DirectDrive.

| Mode | What it does |
| --- | --- |
| Press depth | Brightness and color change continuously in proportion to the depth. Past the actuation point (the per-key `KeyPerf::actuation`) it switches to the "after actuation color". The analog value of the magnetic switch becomes light as is |
| Keystroke heatmap | Counts on the rising edge of the actuation point and shows blue → green → yellow → red. Shows the top keys and the total keystrokes, with a reset |
| Key highlight | Light only arbitrary keys, by preset (FPS / WASD / MOBA / number row) or by clicking. A pressed key lights up |
| Ripple | A ripple spreads from the pressed key (stronger and faster the deeper the press). Computed from the key coordinates (x/y of `LayoutKey`) |

Common: main color / pressed color / background color (palette + any color), background brightness, overall brightness, afterglow (ms),
**hybrid (use the keyboard-side effect as the background)**, **start automatically at launch**.
It keeps running when you switch tabs or devices, and even when minimized. On stop, `direct_drive_stop` → returns to the firmware lighting.
While live is running, the Lighting tab's "Preview on device" is not shown, so nothing is sent twice.

Hybrid is a differentiating feature that goes beyond the official HUB: DirectDrive's flag=0 means "do not touch this cell", so
if disabled cells are not included in DD, the firmware animation keeps running as the background. Only the pressed / highlighted keys
can be overlaid under PC control. Verified on hardware (only the 7 cells at the top left red, the rest continuing the firmware's light blue animation).

### Where it is implemented
- `crates/device-hub/src/livefx.rs` — the engine (pure functions, 2 tests). `FxPrefs` is `livefx` in `prefs.json`.
- `crates/device-hub/src/pages/live.rs` — `fx_start` / `fx_stop` / `fx_tick` and the screen.
- `app.rs` — `Tab::Live` (shown only for models with a DirectDrive-capable area **and** `PerfCaps::live_travel`), `fx_tick` from the 30fps ticker,
  and `stop_device_preview` and the automatic stop when leaving the Lighting tab **exclude the device that is live**.
- Data flow (current): worker `live_travel` → `Msg::Travel` → `Engine::frame` on the UI thread → `Hub::dd_frame` → worker `direct_drive`.

### Keyboard UI improvements (done at the same time)
- Add L/C/R to keys with the same legend (the 3 split Spaces looked like three "Space"s). Common to the `keyboard_view` of all screens.
- Trigger map: a color legend (a gradient from shallow min mm → deep travel mm), an explanation of T/R/RT, and an "Invert" button for the selection.

## 2. Verified / not verified
- Verified on Linux + demo: all 4 modes work (the demo's `live_travel` simulates pressing WASD). `cargo test -p device-hub --lib livefx` passes.
- **Verified on hardware (automated visual check via OBS Virtual Camera)**: with a 30fps DD stream only the 7 cells at the top left are red and the rest
  keep the firmware animation (hybrid). Sending an empty set releases the tracked pins with `[5,4]` and the whole keyboard returns to the firmware.
  The pins remain after the stream stops = there is no DD timeout on the firmware side. Confirmed that `direct_drive_stop` releases everything.
- **Verified in code**: `Shell::Drop` → `Hub::dd_stop_all()` stops at exit, and in addition `XsydDriver::Drop` makes a best-effort attempt
  to read back and restore the tracked cells. The restore waits for HID responses, so
  completion is not guaranteed even on a normal exit.
- **Partially verified**: added congestion control that does not resend unchanged cells and chunks with only flag=0. Measuring actual typing latency / dropped
  8K polling with a histogram of the user's keystrokes remains.
- **Remaining, not verified on hardware**: the overall E2E of UI start → each mode → stop, continuing while minimized, auto start, reconnection
  after unplugging/replugging USB, input latency with real keystrokes.

## 3. Requests to the backend (in priority order)

### P1-a Stopping DirectDrive at exit — **implemented and verified on hardware**
- Hardware result: flag=255 pins remain after the DD stream stops, and no firmware-side timeout is visible.
- `Shell::Drop` → `Hub::dd_stop_all()` sends `DdStop` to all sessions on a normal exit.
- Added a path where `XsydDriver::Drop` makes a best-effort restore of the tracked cells. It does not run on a crash or forced termination,
  and even when the handle is dropped, restoring may not be possible because of communication errors or the exit deadline.
- With `direct_drive_stop`, `[5,4]` release-all + resending `[5,2]` confirmed that the whole keyboard returns on the real device.

### P1-b Measure input latency during live — **congestion control done, measuring with real keystrokes remains**
- `direct_drive` does not send chunks with only flag=0, and the driver tracks pins/colors and sends **only chunks that
  have new or changed cells**. On a static hybrid screen that is effectively 0 reports, and for highlighting a few keys 1 to 2 chunks.
- The effect on actual key input (8K polling) needs a histogram of the user's keystrokes. The `rawinput` measurement path already exists
  on the `Job::PollRate` side.

### P2-a Move the engine to the worker side (lower latency)
Currently the read → UI → write path delays by 1 to 2 frames (about 33 to 66 ms). `livefx::Engine` is a pure function, so
add `Job::LiveFx(FxPrefs)` to the `store` worker and run `live_travel` → `frame` → `direct_drive` on the same thread at 60 to 90Hz,
so the key lights the moment it is pressed (continuous sending at 87fps was OK in probe9). The UI only receives `Msg::FxFrame` thinned out for the preview.
`store.rs` is prone to conflicts with other work, so coordinate in advance with the owner in AUDIT_HANDOFF.

### P2-b Partial overwrite (hybrid) — **verified on hardware and implemented**
- Hardware verification: cells not included in DD are driven by the firmware's effect as is. The DD cells can show red at the same time.
- Implementation: `FxPrefs.hybrid` + "Use the keyboard's effect as the background" on the Live screen. In `frame`, disabled cells
  (same as the background color) are not put into DD, and the driver releases the cells that dropped out with `[5,4]`.
- This is the "firmware background + PC live foreground" composition that the official HUB does not have. The result has been appended to `PROTOCOL_NOTES.md`.

### P3 New input sources for lighting (contract: additions only)
The frontend is expected to receive them as layers. Since a hook for passing external signals to `Engine::frame` will be added, please build the data side.

| Idea | Data source | Example of the lighting |
| --- | --- | --- |
| Audio visualizer | WASAPI loopback (new crate `devhub-audio`, `cpal` or the `windows` crate), 14-band FFT | a per-column spectrum. Post-EQ sound of the TP55 is also possible |
| VALORANT integration | The Riot client's local API (lockfile → own presence in `/chat/v4/presences`: `sessionLoopState`, score, etc.). **Reading memory is forbidden**, official local API only | round score on the number row, color change during agent select, flash on win/loss |
| Device state | Existing battery (mouse), Windows Caps Lock / microphone mute | number row as a remaining-charge bar when the mouse battery is low, Caps display |
| Persisting keystroke stats | Per-day live keystroke counts into `%APPDATA%\DeviceHub\stats.json` | "today's heatmap" and long-term comparison |

Proposed contract (addition): `pub enum FxSignal { Spectrum(Vec<f32>), Score{ally:u8, enemy:u8}, Phase(String), Battery(u8), Flash(Rgb) }` is sent from
`Hub` to the UI (`Msg::FxSignal`). The frontend only adds modes.

## 4. Next candidates on the frontend side (for reference)
- Layer composition (e.g. key highlight + ripple + status display) and a layer editing UI.
- Linking with game presets (also switch the live mode when a preset is applied).
- On the Key mapping screen, highlight the pressed key with the live depth (so you do not have to look for which key it is).
