# Backend brief v3 — full port of the Web HUB + superset

> Audience: the agent in charge of everything **except the frontend** (the devhub-core contract and registry, devhub-hid).
> The frontend (`crates/device-hub`) is written by its owner (Claude). If you want to add to the contract for UI reasons, see §6.
>
> Goal: **make everything mouse.fit and hub.xsyd.top can do possible in this app, and make it easier to use.**
> The user is assumed to stop opening a browser, so leaving even one feature that exists only in the Web version is a failure.

---

## 0. Definition of done

1. **All of P0 to P2 in the §2 feature table are "Done"**. For P3, implement it or write the reason it can't be done in the table.
2. Check each item as "change in the app → same value in the Web version → kept after unplug/replug or power cycle", and write the verification date in the table.
3. `cargo test -p devhub-hid` has encode/decode tests using saved frames (fixtures).
4. `read()` takes **300ms or less** per device (800ms or less for a keyboard including per-key performance).
5. `cargo clippy --workspace --all-targets` has 0 warnings. Update the docs (§8).

## 1. Current status (as of 2026-11-15)

| Part | Status |
|---|---|
| `mousefit8.rs` (report-8/EEPROM path of the 8K dongle 33E4:3854) | Read/write of DPI, buttons, macros and main settings, pairing, battery, polling rate 125–8K (written back with the corrected code table), magnetic button calibration and press depth notifications (op10), asleep detection (timeout + silence → `DeviceInfo.asleep`, auto re-read on wake). **Verified on real hardware** |
| `mousefit.rs` (wired 64B feature report path) | Implemented, **not verified on real hardware** (the HTS Plus Pro wired 5219 is connected and verified via report-8) |
| `xsyd.rs` (SU66 Max) | Keymap, performance, polling rate, stroke, configurations, advanced keys, macros, lighting (main + decoration + per-key colors), settings, reset, calibration — **nearly all features verified on real hardware**. Remaining: linked lighting, firmware update (only the distribution source check is implemented) |
| Frontend | All screens + Test screen, undo/redo, presets, command palette, calibration, wizard, progress display + sidebar battery/sleep display, magnetic button depth, font selection, firmware update notification, operation-complete toast (confirmed on real hardware) |

The biggest gap is the **keyboard**: the heart of a Hall-effect keyboard (per-key trigger/RT) isn't there yet. Top priority.

## 2. Feature table (every Web version feature → how this app represents it)

Legend: **Done** / **Partial** / **Not done**. Priority P0 = top priority (used daily in games), P1 = everyday features in the Web version,
P2 = occasionally used features in the Web version, P3 = model-specific or high-risk.
"How represented" is the container in the contract. `spec:xxx` is the id of a `SettingSpec`, and `act:xxx` is the id of an `ActionSpec`.

### 2.1 Keyboard — hub.xsyd.top (SU66 Max, `xsyd.rs`)

Basis: the behavior of the [official Web HUB](https://hub.xsyd.top/keyboard) and verification on real hardware. See [PROTOCOL_NOTES.md](PROTOCOL_NOTES.md) §2.

| Feature | SDK method | How represented | Status | Priority |
|---|---|---|---|---|
| Per-key trigger (actuation point, release point, RT press/release sensitivity, First touch, top/bottom dead zones, continuous RT) | `getPerformance(row,col)` / `setPerformance{...}` (values are mm×1000) | `KeyboardState::perf` / `Change::KeyPerf` / `PerfCaps` | **Done** (read/write of 66 keys verified on real hardware 11-15. Continuous RT is not on the wire, so caps=false) | **P0** |
| RT precision / axis (switch type) list | `getRtPrecision` `getAxisList` `getCurrentAxis` `setDefaultAxis` | `PerfCaps::step` / `switch_types`, `KeyPerf::switch_type` | Partial (step / axis list fetched at open. `setDefaultAxis` is not exposed) | **P0** |
| Stroke test (real-time display of press depth) | `getADC` `getRoute` `getKeyStatus` | `DeviceDriver::live_travel` + `PerfCaps::live_travel=true` | **Done** (route → mm, verified on real hardware) | **P0** |
| Magnetic axis calibration | `CalibrationSetting(6)` `getCalibrate` | `act:calibrate` (start) + `act:calibrate_finish` (save). The UI shows a wizard (§4) | **Done** (UI wizard run on real hardware, and `[4,3,2,row]` status=2 confirmed all rows calibrated 11-15) | **P0** |
| Polling rate (8K to 125) | `ReportRateSetting(5)` | `spec:polling` (Choice, Hz) | **Done** (list/current/set, verified on real hardware) | **P0** |
| Saving settings (persistence) | `SaveParam(2)` | Send after each `apply` if needed | **Done** (sent after all applies, per scope) | **P0** |
| Switching configurations 1-4 / names | `ConfigSwitch(3)` `getConfigList` `getConfigName/setConfigName` | `ProfileCaps{count:4, renamable:true}` / `Change::Profile` `ProfileName` | **Done** (4 configurations, name read/write, active retrieval, verified on real hardware) | P1 |
| Advanced keys DKS / MPT / MT / TGL / END / SOCD / RS | `setHigherKey*` `getHigherKeyData` (SOCD/RS take 2 packets, release is NONE) | `KeyboardState::advanced` / `Change::Advanced` / `KeyboardCaps::advanced` | **Done** (TGL write/read/clear verified on real hardware 11-15. Same path for all types) | P1 |
| Macros (recording, mode, max 32768ms) | `setMacroData` `getMacroData` `setMacroMode` `getMacroSpaceInfo` | `MacroCaps` / `KeyboardState::macros` / `Change::Macro` | **Done** (write/read/delete verified on real hardware) | P1 |
| Assigning macros / key combinations | Key code space (PROTOCOL_NOTES §2.4) | `KeyAction::Macro` `KeyAction::Combo`, `KeyboardCaps::combos=true` | Partial (Combo=`0x1000\|mods<<8\|usage` implemented, caps=true. The key code space for Macro assignment is not settled) | P1 |
| Decoration light (ON/OFF, top/bottom, effects) | `getLightingBase(area)` `getDecorate1Custom` | `LightArea{id:1..}` | **Done** (area 1 read/write verified on real hardware) | P1 |
| Per-key colors (custom) | `getLightingCustom` `saveLightingCustom` `setLightingDirectDrive` | `LightingState::per_key` (`per_key` on the effect) | **Done** (write/read verified on real hardware 11-15. Cell mapping: 6×21 grid, row 5 is a measured table — `xsyd.rs::led_cells`, PROTOCOL_NOTES §2.5) | P1 |
| OS (Win/Mac), Win key lock | `SystemSetting(4)` `setSystemType` | `spec:system_type` `spec:win_lock` | Partial (system_type verified on real hardware. win_lock is on the key code side) | P1 |
| Scoped reset (all / calibration / performance / lighting / layout / advanced keys / macros / axes) | `ResetFactory(1)` + scope | `act:reset_perf` `act:reset_keymap` `act:reset_lighting` `act:reset_advanced` `act:reset_macro` `act:factory_reset` (`reload:true`) | **Done** (all scopes exposed) | P1 |
| Lighting auto-off, linking | `getLightingSleepTime/set` `getDoubleLighting` | `spec:light_sleep` `spec:light_link` | Partial (light_sleep verified on real hardware. Linking not done) | P2 |
| Anti-chatter optimization | `ShakeOptimizationSwitch(16)` | `spec:shake_opt` (with confirm) | **Done** (verified on real hardware) | P2 |
| USB mode (2.0 / 3.0 8K/16K/32K) | `USBModeSetting(18)` `getUSBModeStatus` | `spec:usb_mode` (with confirm) | N/A (on the real device `[2,18,*]`→0xFF unsupported. The SU66 is wired-only) | P2 |
| Compatibility mode | `openWebDriver` family / blackout | `spec:compat_mode` | Not done | P2 |
| Sleep time (shallow/deep, wireless models) | `setSleepTime{shallow,deep}` | `spec:sleep_shallow` `spec:sleep_deep` | Not done (wired models don't declare it) | P2 |
| Firmware update | `getAppToBoot` `getFlash` `upgrade` / Sparklink getFirmwarePack | `act:firmware` (`.file(&["bin"])`, sends Progress) | Partial (distribution source check + notification implemented. Writing not done) | P3 |
| Screen GIF / audio / haptics / gamepad (handle) mode | displayer / voice / haptic / handle | Only on models that have them. Needs a contract addition → §6 | Not done | P3 |

### 2.2 Mouse — www.mouse.fit (`mousefit8.rs` dongle, `mousefit.rs` wired)

Basis: the behavior of [mouse.fit](https://www.mouse.fit/) and verification on real hardware. EEPROM addresses are in [PROTOCOL_NOTES.md](PROTOCOL_NOTES.md) §1.3.

| Feature | Basis (JS / EEPROM) | How represented | Status | Priority |
|---|---|---|---|---|
| Wired path (5219 etc.) | 64B feature report | `mousefit.rs` | Driver path **unverified** (the 5219 wired connection itself is verified via report-8) | **P0** |
| HTX Ultra (5608/5617/5618/5619) | env-models.json | registry + model table | Unverified | **P0** |
| DPI stages (count, value, XY, color, current) | `DPIValue` `Sensor3955DPI` `DPIColor` `maxDpiStage` `CurrentDPI` | `MouseState::stages` / `Change::DpiStages` `ActiveStage` | Done (dongle) | P0 |
| Polling rate (wired / wireless. Wireless candidates vary by dongle type) | `ReportRate` / `getDonglePR` | `spec:polling_wired` `spec:polling_wireless` | Done (wireless 125–8K written back and verified on real hardware 11-15, code table fixed. Narrowing candidates for other dongles is unverified) | **P0** |
| LOD | `LOD` | `spec:lod` | Done | P0 |
| MotionSync / angle snapping / ripple / sensor angle | `MotionSync` `Angle` `Ripple` `AngleTune` | `spec:motion_sync` `angle_snap` `ripple` `sensor_angle` | Done | P0 |
| Click debounce (press/release) / wheel debounce | `DebounceTime` `DebounceReleaseTime` `WheelDebounceTime` | `spec:debounce_press` `debounce_release` `wheel_debounce` | Done | P0 |
| Battery (%, charging) / device and dongle FW | op 4 / 18 / 29 | `battery()` / `DeviceInfo::firmware` `dongle_firmware` | Done (fw 2.05 / 85% confirmed displayed on real hardware) | P0 |
| Following device-side changes / asleep | op10 notifications (depth/DPI/polling rate/battery) + `04 xx` + command timeout detection | `HubEvent::StateChanged` `Battery` `Info` / `DeviceInfo::asleep` | Done (asleep → sidebar dimmed, wake → auto re-read verified on real hardware) | P0 |
| Competitive mode / hyper mode (+ performance time) | `PerformanceState` `Performance` / `setHyperMode` `setHyperModePerformanceTime` | `spec:competitive` `spec:hyper_mode` (+ `spec:hyper_time`) | Partial (hyper mode not done) | P1 |
| DIY debounce 4 values + low/mid/high presets | `ButtonDebounceAllDiy` `ButtonDebounceLevel` | `spec:debounce_preset` (Choice) + 4 Sliders (shown only for custom via `visible_when`) | Not done | P1 |
| SPDT (left/right) | `getSPDTStateNew/set` | `spec:spdt_left` `spdt_right` | Partial (Unsupported on the dongle path) | P1 |
| Accidental button press prevention (LiftOff debounce) | `getLiftOffDebounceNew/set` | `spec:liftoff_debounce` | Not done | P1 |
| Sleep time | `SleepTime` | `spec:sleep_min` | Done | P1 |
| DPI indicator / LED state / effects | `setDPIIndicator` `setLedState` `DPIEffectMode/State` | `spec:dpi_indicator` `spec:dpi_led_effect` | Partial (wired only) | P1 |
| Button assignment (all kinds) | `Ya` (Disable/MouseKey/DPISwitch/FireKey/ShortcutKey/Macro/ReportRateSwitch/DPILock/UpDownRoll) | `Change::Button`, `MouseCaps::button_kinds` | Partial (up/down roll and profile switching are Unsupported) | P1 |
| Macros (max 3, name, count / until released / until pressed again) | `Macro` 768+ | `MouseState::macros` / `Change::Macro` | Done (dongle) | P1 |
| Magnetic buttons: trigger / quick trigger / vibration / left-right calibration / press depth display | `Left/RightTrigger` `FastTrigger` `TactileFeedback` / `StartCalibr(46)` / op10 notifications | `spec:ms_*` / `act:calibrate_buttons` / `live_buttons` | Done (calibration UI run and real-time press depth display both verified on real hardware) | P1 |
| Pairing | op 5/6 | `act:pair` | Done | P1 |
| Settings reset / factory reset | `ClearSetting(9)` `resetProfile` `defaultDevice` | `act:reset_profile` `act:factory_reset` (`reload:true`) | Not done | P1 |
| Key combination feature | `getButtonCombine/set` | `spec:combine_key` | Not done | P2 |
| 20K FPS / sensor virtual center / sensor switching / tracking mode | `SensorFPS20K` `SensorVirtualPos*` `setSensorIndex` `setTrackingMode` | `spec:sensor_fps_20k` `sensor_center` `sensor_index` `tracking_mode` | Not done | P2 |
| Long-range mode | op 22/23 | `spec:long_range` | Done | P2 |
| Device light (solid/breathing/spectrum/wave + brightness/speed) | `SetLightEffect` `SetLightness` / `LightEffects` | `LightArea{id:0}` | Not done (only models with LightEnablePID in the model table) | P2 |
| Receiver light / battery display | the "Receiver light effect" and "Battery indicator" labels | `LightArea{id:1, name:"Receiver"}` | Not done | P2 |
| Onboard profiles | `getProfileList` `setProfileID` (op 14/15) | `ProfileCaps` / `Change::Profile` | Not done (Unsupported on the dongle) | P2 |
| Receiver button assignment | `ReceiverKeyBindingSeq` | Needs a contract addition → §6 | Not done | P3 |
| Firmware update (DFU) | `enterBL` `erase` `program` `verify` `exitBL` | `act:firmware` + Progress | Partial (distribution source check + notification implemented in devhub-update. Writing not done) | P3 |

### 2.3 ATK F1 Ultimate

It is not in mouse.fit's model table. Take the VID/PID/usage from the real device and decide the family with `docs/ADD_DEVICE.md §7`.
If it speaks an existing protocol, just add it to the registry. If not, add a `DriverKind` and a new driver. P1.

## 3. Order of work

1. **P0 keyboard**: add performance (read/write/save), polling rate, stroke test and calibration to `xsyd.rs`.
   Resolve the one-character constants (`Or` `kr` `Ar` …) at the top of the esm to make a table of the byte sequences the SDK's `F([...])` builds,
   and **save the frames to fixtures before writing the code**.
2. **P0 mouse**: verify the wired path and the HTX Ultra on real hardware. Correctly narrow the polling rate candidates by dongle type.
3. **P1**: configurations 1-4, advanced keys, macros, combination/macro assignment, decoration lighting, OS/Win lock,
   scoped reset, unimplemented mouse settings, remaining button kinds, magnetic button calibration, ATK F1.
4. **P2 / P3**: the rest. Do the firmware update **last**, and if you implement it, without changing the Web version's procedure by even one byte.

## 4. Contract promises (the frontend behaves like this, so return it like this)

- **The UI looks only at caps**. A feature a model lacks disappears from the UI just by **not returning** a spec/action/caps.
  "Declared but can't be read" is the worst (the row is shown disabled). Don't declare what you don't understand.
- Make generic setting ids and units match demo.rs and the names in the §2 table (for compatibility of exported files).
  When you create a new id, add it to §2.
- `apply` is fine-grained. The UI thins out consecutive changes to the same target before sending. **If a write fails, always return `Err`**
  (the UI shows a toast and re-reads). Never pretend to succeed.
- **Undo / applying a preset**: the UI turns the difference from the "target state" into a sequence of `Change`s and sends them in order.
  `KeyPerf` sends keys with the same value as one item, so implement bulk writes of multiple keys to be fast.
- **Calibration**: `act:calibrate` (start) → the UI calls `live_travel()` at 30Hz and shows each key's maximum depth →
  save with `act:calibrate_finish` (if present). `live_travel` returns values in **layout order, in mm**.
- Actions ending in `*_finish` do not appear in the list (wizard only).
- **Progress**: long operations (firmware update, bulk writes, calibration) send `HubEvent::Progress { id, label,
  fraction, done }` via the `watch` callback. A progress bar appears in the UI header. Finish with `done: true`.
- **Notification of changes made on the device**: if you can tell that DPI / profile was changed with the device's button, charging started, etc.,
  send `HubEvent::StateChanged(id)` / `HubEvent::Battery(..)`. The UI re-reads (something the Web version can't do = a superset point).
- When the device is asleep over wireless and only the dongle responds → `DeviceInfo::asleep = true`. When it wakes, returning `asleep=false` from `scan` makes
  the UI reopen it.
- Write error messages as English `HubError` strings (the UI shows them in a toast as is).

## 5. What the backend must do to be a superset

Things the frontend can't do alone:

1. **Speed**: pipeline the reads (send the requests together, then collect the responses). Per-key performance for a whole keyboard also in under 1 second.
2. **Reliability**: read back after writing and confirm it matches (inside `apply`; if it differs, `Err(Protocol)`). Don't forget the save command.
3. **Auto recovery**: `scan` must not create a new ID after replug, wake from sleep or dongle swap (keep the serial / path stable).
   Notify hot-plug with `watch` (just enumerating `hidapi` every second and looking at the diff is acceptable).
4. **Pick up changes made on the device**: detect DPI changes and battery changes in the thread that reads input reports continuously and send `HubEvent`.
5. **Detect simultaneous launch with the Web version**: when another process holds the device and it can't be opened,
   give a sentence that makes the cause clear, such as `HubError::Io("Another app, such as the browser's settings page, is using the device")`.
6. **Safety**: make firmware update and factory reset able to refuse after checking in advance the battery level (wireless) and the connection type (wired required, etc.).

## 6. When extending the contract (devhub-core)

- Scalar setting → just add a `SettingSpec` (no contract change, appears in the UI automatically).
  - Settings that depend on other settings (e.g. LAMZU's 20K FPS only when competitive mode is ON) use
    `.enabled_when("hyper_mode", SettingValue::Bool(true), "Enable competitive mode first")`.
    While the condition isn't met, the UI dims the row, shows the reason and doesn't allow operation. The driver must also always refuse (`Err(Invalid)`).
  - The curve for speed-linked DPI is `MouseCaps::adaptive_dpi: Option<AdaptiveCaps>` + `MouseState::adaptive` + `Change::Adaptive`
    (points = speed u16 and multiplier f32). Declaring it makes a curve editor card appear in the Performance tab.
- A single button operation → just add an `ActionSpec`.
- New features with structure (screen GIF, audio, handle mode, receiver button assignment, etc.) → **add** a type and a `Change`
  variant to `model.rs` (don't change the existing ones). Implement it in `demo.rs` too, and **write the UI request in `docs/FRONTEND_TODO.md`**
  (the frontend owner builds the screen). Format: what feature it is / the new type / which caps show it / where in the Web version's screen it is.

## 7. Development and verification procedure

```bash
cargo run -p device-hub                    # real device (demo if HID can't be opened)
DEVHUB_DEMO=1 cargo run -p device-hub      # demo
cargo run -p devhub-hid --example enum     # HID enumeration
cargo run -p devhub-hid --example readback # read-back check (existing)
cargo test -p devhub-hid
```

- Verify one item at a time: "read → change → read back → check in the Web version → restore".
- Frames go in `crates/devhub-hid/fixtures/<model>/<name>.req.bin` / `.resp.bin` + a description `.md`.
- **Don't operate with the app while the Web version's tab is open** (they fight over HID). Close the tab after checking in the Web version.
- Verify resets and firmware updates last, after confirming with the user.
- Don't edit `crates/device-hub` (the frontend).

## 8. What to update when done

- The "Status" column and verification dates in §2 of this document
- The support table in `docs/ADD_DEVICE.md` and `verified` in `registry.rs`
- `docs/PROTOCOL_NOTES.md` (newly learned byte sequences, units, caveats)
- The "Current status" in `AGENTS.md`
- `docs/FRONTEND_TODO.md` if there are requests to the frontend
