# Frontend structure (crates/device-hub)

```
src/
  main.rs        opens the window (TitleBar::window_options, 1320x840, throttled to 250ms when in the background)
  app.rs         Shell: all state, root (Home / Device / Settings), tabs, sidebar, key input, drag
  store.rs       Hub / Session: a worker thread per device, optimistic updates (apply_local), results received over a futures channel
  overlay.rs     popup menus, modals (confirm / button assignment / key capture / color / text), toasts
  ui.rs          components: card / row / toggle / slider(_curve) / segmented / select / stepper / tab_bar / button / chip
  surface.rs     flat surfaces (Card / Bar / Control / Popup): one opaque quad + 1px border
  motion.rs      critically damped springs, recording of element rects (track / rect / hovered). Requests the next frame only while moving
  theme.rs       colors (dark/light, 6 accent colors), fonts
  prefs.rs       %APPDATA%\DeviceHub\prefs.json
  keycode.rs     GPUI key name → HID usage
  palette.rs     command palette (Ctrl+K)
  presets.rs     presets saved on the PC (%APPDATA%\DeviceHub\presets\<model>\*.json)
  pages/
    home.rs       device list
    device.rs     header (name, connection, profile, sync) + tabs + loading/failed/unsupported
    generic.rs    automatically lays out SettingSpec / ActionSpec into cards per section
    mouse.rs      DPI stages, button assignment
    keyboard.rs   keyboard diagram (shared), key mapping, triggers (magnetic switches / stroke test)
    advanced.rs   advanced keys (DKS/MPT/MT/TGL/END/SOCD/RS)
    lighting.rs   lighting (areas, effects, colors, per-key painting)
    macros.rs     macros (record, insert, edit delays)
    system.rs     device info, profiles, export/import, maintenance
    settings.rs   app settings
    test.rs       test (click/wheel/chattering, key input and simultaneous presses)
    calibrate.rs  magnetic-switch calibration wizard
```

## Which tabs are shown (`app::tabs_for`)

| caps | Tabs |
|---|---|
| mouse | DPI, Buttons (if there are buttons), Performance (if there are performance/sensor/click/magnetic/power settings) |
| keyboard | Key mapping, Triggers (perf), Advanced keys (advanced) |
| lighting / lighting section | Lighting |
| macros | Macros |
| always | General (system / connection / keyboard / unknown sections, info, profiles, backup) |

## Data flow

UI action → `Shell::apply(Change)` → `Hub::apply` rewrites the UI state immediately (apply_local) → sends to the worker →
the worker drops older changes to the same target and calls `DeviceDriver::apply` → result (`Msg::Applied`) → on failure, a toast + `read` again.
Dot in the header: yellow = writes pending / green = in sync.

## When a new screen is needed

Only when a new feature with structure has been added to the contract (`docs/FRONTEND_TODO.md`). Add a file to `pages/` and
one line to `Tab` and `tabs_for`. Scalar settings and operations are drawn automatically by generic.rs, so no screen needs to be added for them.

## Checking that it works

```bash
DEVHUB_DEMO=1 cargo run -p device-hub
```
The demo has 4 devices: HTS Plus Pro (wireless) / HTX Ultra (wired, magnetic buttons) / SU66 Max / ATK F1 (unsupported).

## Features that are more convenient than the web version (done in the frontend alone)

| Feature | Where | What it does |
|---|---|---|
| Undo / redo | ↶↷ in the header, Ctrl+Z / Ctrl+Y | Pushes the `DeviceState` before the change onto the history and writes back the difference (`store::diff_changes`). Consecutive operations within 0.8 seconds count as one |
| Presets | General tab, dropdown in the header | Saves the current settings on the PC under a name and applies them in one click (writing only the difference). Not limited by the number of onboard slots |
| Command palette | Ctrl+K / search in the sidebar | Find screens, settings, operations, and devices by name to jump to or run them |
| Test | Test tab | Click count, interval, suspected chattering, wheel, move event count, key input, simultaneous press count, double input |
| Calibration wizard | "Magnetic switch calibration" on the General tab | Shows keys pressed all the way to the bottom in green while watching the press depth |
| Trigger presets | Triggers tab | Applies Fastest RT / Competitive RT / Standard / Typing to the selected keys at once |
| Key search | Key mapping | Search by name across all groups |
| Layer copy | Key mapping | Duplicates the displayed layer to another layer |
| Progress display | Header | A progress bar driven by `HubEvent::Progress` |
| Low battery notification | Toast | Once at 15% or below |
| Settings profiles | Automation (sidebar) | Records changes made on the device screen as a `Change` sequence and replays them at any time (`profiles` in prefs.json). "Current state" records a snapshot (settings/DPI/performance/lighting/EQ — structural items such as key mapping are excluded), "Name" renames (rule references follow) |
| Process linking | Automation | While the exe name in `proc_rules` is in the foreground, the `on_fg` profile is applied; when it leaves, `on_leave`. `procmon.rs` checks the foreground window about every 1 second (works while minimized; verified on hardware). A process can be chosen from the "Running apps" dropdown (a list of exes with visible windows + titles), and rules have an ON/OFF toggle, editing, and a running chip |
| Always-on rate measurement | Performance/Test tabs | Updates the measured Hz with Raw Input `RIDEV_INPUTSINK` (about every 400ms, while the device screen is open). **Windows 11 (Moment 3 and later) restricts raw input delivery to background processes**, so the value drops and cannot be trusted while the window is in the background — the display switches on `last_window_active`, and while in the background it shows a faint "OS is restricting input" (the same OS restriction as MouseTester). The measurement thread uses MMCSS "Pro Audio" + ABOVE_NORMAL, and also shows the Hz back-calculated from the median interval and a p95 jitter warning |

### Verifying the true polling rate (outside the app)

WM_INPUT counts "what the OS delivered", so when you want the number of reports the device actually put on the wire
(true 8K verification), use **USBPcap + Wireshark**:

1. Install USBPcap (a kernel filter driver — administrator rights are required).
2. Select the target host controller in Wireshark and start capturing.
3. Move the mouse → for `usb.bInterfaceClass == 3` / interrupt-IN URBs,
read the frame intervals in the `usb.dst` (host) direction. For 8K, intervals of ~0.125ms line up.
4. The same applies via the dongle — you can see what the receiver puts on the bus.

Reading the vendor I/F (ff02 etc.) directly is not a substitute — its spontaneous reports are
event notifications only (~10Hz measured), and motion data is exclusive to the system TLC (Windows does not let you read it).

