# GPUI Device HUB

A personal GPUI app for Windows that configures mice and keyboards
without leaving a browser (www.mouse.fit / hub.xsyd.top) open. The design is Liquid Glass.

## Layout

```
crates/devhub-core   contract v2 (model / backend traits) + keys + registry + demo backend   ← boundary between UI and HID
crates/devhub-hid    real-device drivers (HID)                                                ← backend side
crates/device-hub    GPUI frontend (Liquid Glass)                                             ← frontend side
vendor/              gpui-pre / gpui-pre-windows with a GPU backdrop added (GPU backdrop patch)
docs/NEXT_TASKS.md      what to do next (v4)
docs/LAMZU_MAYA_X_V2_AUDIT2.md  LAMZU MAYA X V2 re-audit (fixes needed P1/P2, name mapping table, work order)
docs/POLLING_RATE.md    verification of the measured polling rate and improvement proposals (the 2700Hz issue)
docs/OBS_OVERLAY.md     OBS overlay (design, in-app editor, settings-file integration)
docs/LIVE_LIGHTING.md   live lighting (DirectDrive), keyboard UI improvements + backend requests
docs/AUDIO_FRONTEND.md  EQ page for USB DACs (TP55 / WalkPlay) and how the contract is used
docs/BACKEND_BRIEF.md   feature table for the complete Web HUB port (v3)
docs/ADD_DEVICE.md      steps for adding a new device (for another session / another agent)
docs/PROTOCOL_NOTES.md  results of analyzing the web configurators (command tables, addresses, feature lists)
docs/FRONTEND.md        frontend structure
```

## Current status (2026-11-15)

- Frontend: all pages (Home / DPI / Buttons / Performance / Key mapping / Triggers / Advanced keys / Lighting /
  Macros / Test / General / App settings) + undo, presets, command palette, calibration, wizard +
  sidebar battery/sleep display, magnetic-button press depth, font selection (bundled fonts + persistence),
  firmware update notifications (crates/devhub-update), operation-complete toasts. docs/FRONTEND.md.
- NEXT_TASKS v4 §1–§6 are done. HTX Ultra 3950 wired (33e4:5618) is also verified on hardware
  (all settings read, SPDT/polling UI writes, LOD/DPI/buttons/macros — docs/PROTOCOL_NOTES §1.4).
- Verified on hardware (HTX Ultra 3950 wired, 2026-11): SPDT @8 bit0/1 (click detected on both contacts → lower latency),
  LOD non-index {0.7→3, 1→1, 2→2}, polling 125–8K, competitive mode, debounce press/release/wheel,
  motion sync, straight-line correction, ripple, sensor angle, sleep, long-distance mode.
- Contract: v2 (`Capabilities` + `DeviceState` + fine-grained `Change` + `HubEvent::Info` to report state changes made on the device itself).
- devhub-hid: implemented — `transport.rs` (feature/output/input report) + `mousefit.rs`
  (kept for feature reports, not used by current models) + `mousefit8.rs` (report-8/EEPROM, shared by wired and dongle)
  + `xsyd.rs` (SU66 Max) + `atk.rs` (ATK HUB family, report-8/EEPROM, model resolved from CID/MID).
  Code from the v1 era is in `legacy/`; the original vendor distributions fetched for analysis are kept out of Git (`.private-analysis/`).
  Verification examples: `enum` (enumeration) / `kbdiff` (SU66 diff read) / `lite2e` (lighting round trip) / `tp55e2e` (TP55 EQ round trip)
  / `atkprobe` (ATK raw EEPROM) / `atkread` (ATK read/write round trip through the driver path).
- **mouse.fit path**: neither wired 5219 nor dongle 3854 has a feature report, only
  report-8/EEPROM → every model uses `mousefit8.rs`.
- Verified on hardware (2026-11-15): HTS Plus Pro wired+wireless (all settings, DPI, buttons, macros, polling 125–8K,
  magnetic-button calibration, press depth, **sleep detection**: asleep on command timeout + receive silence, automatic re-read on wake,
  **ShortcutKey is a compact `[count][press][release reversed][crc]` sequence, RT is an en<<7|val RMW,
  triggers follow the official one: displayed 1-20 = EEPROM-1** — all confirmed by round trip on hardware with `htse2e`),
  SU66 Max (keymap, lighting, **per-key performance, polling, OS/debounce/sleep,
  config 4, advanced keys, macros, live_travel, calibration** — all confirmed by read and write on hardware),
  ATK F1 V3 ULTIMATE+ **both wired and dongle paths** (CID/MID detection, FW 1.33, battery, DPI + stage colors,
  polling 8K, LOD **0.7–1.7mm in 0.1 steps confirmed on hardware**, motion sync / straight-line correction / ripple,
  wheel/click debounce, sleep, LED off while moving, bhop, ShortcutKey button Key read/write,
  profile 4, factory reset — all confirmed by round trip. **Dongle 373b:1278** relays the same 16B protocol
  + cmd29 dongle FW + sleep detection (asleep/HubEvent, CID/MID re-resolved on wake).
  Unsupported: macro @768, dynamicSensitivity, sensorAngle, BT, FW update — §4).

- Added 2026-11-18 (details and evidence in docs/PROTOCOL_NOTES §6–§7):
  - `ry.rs` — RongYuan (VID 3151). Fantech MAXFIT81 on the cable (id 694): key remapping on both layers,
    debounce and auto-OS, verified on hardware. Fantech Tanto PRO WG13P (id 3109) through its 2.4G receiver
    (3151:402d) and on its cable (3151:4026): DPI, buttons, report rate, lift-off, angle snap, debounce,
    verified on hardware on both; battery through the receiver only.
    **Do not add lighting / options writes for the MAXFIT81 without reading §6.3**: they locked up its
    control endpoint twice (re-plug needed).
  - `bt.rs` + `earfun.rs` — Bluetooth RFCOMM transport and the EarFun GAIA driver (Air Pro 4+, untested).
    EarFun Wave Pro and HUAWEI FreeBuds SE 3 are listed as detected only.
  - Examples: `ryprobe` (raw read-only), `ryread` (through the driver, `--roundtrip` writes and restores),
    `btprobe` (Bluetooth firmware query).

## Build / run

```bash
cargo run -p device-hub                  # real device (falls back to the demo automatically if HID cannot be opened)
cargo run --release -p device-hub        # use release for everyday use (lighter startup, rendering, memory)
DEVHUB_DEMO=1 cargo run -p device-hub    # always demo
DEVHUB_FAST=1 cargo run -p device-hub    # GPU glass off
cargo run -p devhub-hid --example enum   # HID enumeration
cargo run -p devhub-obs --example preview # check the overlay rendering as a PNG without OBS
cargo clippy --workspace
```

## OBS plugin (devhub-obs)

`crates/devhub-obs` is a cdylib OBS source plugin ("Device HUB Live").
It draws the press depth of magnetic keys (SU66 `live_axis`) and magnetic buttons (HTS `live_buttons`)
live. obs.dll is resolved at run time with GetProcAddress, so it
builds without obs.lib / a C++ toolchain.

```bash
cargo build --release -p devhub-obs
# Install (OBS 30.2+ does not read %APPDATA%\plugins → use ProgramData):
mkdir -p "/c/ProgramData/obs-studio/plugins/devhub_obs/bin/64bit"
cp target/release/devhub_obs.dll "/c/ProgramData/obs-studio/plugins/devhub_obs/bin/64bit/"
# Start OBS → add a source → "Device HUB Live"
# Diagnostic log: %TEMP%\devhub_obs.log (only create / tex / first draw)
# Look and displayed keys are set on the "OBS overlay" page of Device HUB → %APPDATA%\DeviceHub\obs_overlay.json (docs/OBS_OVERLAY.md)
# Check the design: cargo run -p devhub-obs --example gallery
```
```

The Rust toolchain is pinned to 1.95.0 in `rust-toolchain.toml` (required by gpui-kit 0.9.1).
`cargo check` also passes on Linux (gpui-pre-wgpu does not know the vendor backdrop, so only when
building on Linux does the wgpu side need a 2-line patch in `.cargo/config.toml` — not needed on Windows).

## Rules

- **The UI decides what to show from caps only.** Do not branch the UI on device name or VID/PID.
  A new device is added with just a registry entry + a driver.
- Declare scalar settings with `SettingSpec` and single-button operations with `ActionSpec` (they appear in the UI without changing the contract).
- When changing the contract (`devhub-core/src/model.rs`), **only add**. Update demo.rs too.
- Call `Window::paint_backdrop` only through `device-hub/src/glass.rs`.
- Keep the patches in `vendor/` to a minimal diff against upstream (crates.io `gpui-pre`/`gpui-pre-windows` 0.3.11).
  Mark each changed place with a `DEVHUB` comment (see `vendor/README.md`).
- Do not operate the real device while a Web version tab is open (they compete for HID).
