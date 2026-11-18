# Next task brief v4 (2026-11-14)

> Audience: the implementing agent (**this time everything, frontend included**).
> Prerequisite reading: `AGENTS.md` → `docs/FRONTEND.md` → `docs/BACKEND_BRIEF.md` → `docs/PROTOCOL_NOTES.md`.
> Rules (`AGENTS.md`): the UI branches on caps only / the contract (`model.rs`) is additive only / `paint_backdrop` goes through
> `glass.rs` / do not touch a real device while the Web version's tab is open.

---

## 0. Current completeness (assessment from reading the code and docs)

| Area | Completeness | Basis |
|---|---|---|
| Frontend (all screens, undo, presets, search, test, calibration) | 90% | The screens are all there. What's left is this round's requests (§1-§5) and the §6 improvements |
| SU66 Max (xsyd) | 80-90% | P0/P1 are mostly verified on real hardware (per-key performance, RT, polling rate, 4 configurations, advanced keys, macros, decoration lighting, per-key colors, calibration, stroke test). Writing advanced keys / macros still awaits verification with real data. Compatibility mode, light linking and firmware update are not done |
| HTS Plus Pro (mouse.fit) | 70% | Wired and wireless: DPI / buttons / macros / main settings are verified on real hardware. Not done: hyper mode, DIY debounce, accidental-press prevention, key combinations, sensor details, device/receiver lighting, profiles, reset, button calibration, firmware update |
| HTX Ultra | Unverified | Only registered from the model table |
| ATK F1 Ultimate | 0 | VID/PID not determined (not in mouse.fit's model table) |
| Following changes made on the device itself (e.g. changing DPI with the device's own button) | 0 | The Web version does this (§6.1). Without it the app is not a "superset" |

**Day-to-day features reach 80-90% of the Web version. In terms of all Web version features, about 70-80%.**
The "Status" column of `docs/BACKEND_BRIEF.md` §2.2 (mice) is outdated, so update it to reality at the start of the work.

## Order of work

1. §1 Remove the click feel (small)
2. §3 Battery in the sidebar (medium)
3. §2 Real-time display of magnetic button press depth (large, needs hardware investigation)
4. §5 Fonts (medium)
5. §4 Firmware update notification (medium)
6. §6 Improvements (following changes made on the device itself first)

When every "completion condition" of a section is met, mark that section done and update the current status in `AGENTS.md`.

---

## 1. Remove the "click feel" from the HTS Plus Pro

User request: **"Left button click feel" and "Right button click feel" are not needed.**

- Remove the `SettingSpec`s `ms_tactile_left` / `ms_tactile_right` from the caps in `crates/devhub-hid/src/mousefit8.rs`
  (around lines 803-815). The matching read (around line 1011) and apply (around line 1271) can be removed too.
- The same content is also in `crates/devhub-core/src/demo.rs` (the demo's `mag_vibration` / `mag_sound`), so remove it from the demo too.
- Keep the other magnetic button settings (trigger position, quick trigger).

**Completion condition**: on both real hardware and the demo, the magnetic switch card in the Performance tab shows no click-feel (vibration / click sound) rows.

## 2. See magnetic button press depth in real time

User request: **I want to see in real time how far I'm pressing the mouse buttons (press depth).**

### 2.1 Data available from the real device (found from mouse.fit's JS)

The report-8 format confirmed from [mouse.fit](https://www.mouse.fit/)'s notification handling.
Notification offsets are counted from the start of the buffer that includes the report ID.

| Condition | Content |
| --- | --- |
| Offset 1 is 10 | Spontaneous notification from the device |
| Offset 6 is 1 / 2 / 64 | DPI / polling rate / battery changed |
| Offset 6 is 0, 4, 8, 16, 32 | Offset 14 is left, offset 15 is right. The low 7 bits are the press depth (0-100%) |

The official UI's actuation points move in 5% steps, and the step it shows is the trigger value read from the device plus 1.

The feature-report family has similar notifications (the `ka` handler, report id 4: `04 01/04` DPI, `04 03` battery,
`04 02` profile, `04 06 00/01` disconnect/connect, `04 07` LOD, `04 08` polling rate, `04 0f` MotionSync, `04 18` sensor).

**First check on the real device** (create `btnprobe.rs` in `crates/devhub-hid/examples/`):
1. Keep reading input reports without sending anything, and see whether `r[14]/r[15]` of op 10 change on left/right clicks.
2. If they don't change, see whether they change after `StartCalibr(46)`, or after the command mouse.fit sends when its magnetic button screen is opened
   (recording mouse.fit's traffic with the WebHID hook in `docs/ADD_DEVICE.md §2.3` is the reliable way).
3. On both wired (5219) and dongle (3854). Write the results in `docs/PROTOCOL_NOTES.md`.

### 2.2 Contract additions (`crates/devhub-core`)

```rust
// model.rs
/// One button that can report its press depth (magnetic switches etc.).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ButtonDepth {
    /// 0.0 (released) to 1.0 (fully pressed).
    pub depth: f32,
    /// Actuation point 0..1 (if known). The UI draws a line at this position.
    pub trigger: Option<f32>,
    /// Whether the device considers the button "pressed" (if known).
    pub pressed: Option<bool>,
}

// Add to MouseCaps
/// Slot numbers of the buttons that can report press depth (indexes into `buttons`). If empty, the UI shows nothing.
pub live_buttons: Vec<usize>,

// Add to DeviceDriver in backend.rs (default is Unsupported)
/// Latest press depths, in the same order as `MouseCaps::live_buttons`. The UI calls this at 60Hz on the Test screen.
/// For devices that deliver data by notification (push), just return the latest value saved by the receive thread (do not hit HID here).
fn live_buttons(&self) -> HubResult<Vec<ButtonDepth>> { Err(HubError::Unsupported("live_buttons")) }
```

Drivers and demos that build `MouseCaps` as a struct literal need to add `live_buttons: vec![]`.
The demo (the HTX Ultra with magnetic buttons in `demo.rs`) returns depths as a sine wave.

### 2.3 Backend (`mousefit8.rs`)

- **Receive thread**: command responses and notifications are mixed in the same input report. Use a single receive thread that reads all input reports and
  dispatches "responses (op matches the command)" to the command side over a channel and "notifications (op 10 / report 4)" to the notification handler.
  Rework the current design where `cmd()` does its own read into this shape.
- Store the L/R depths received by notification in a `Mutex<[ButtonDepth; 2]>`, and have `live_buttons()` return it.
  Fill `trigger` from the values of `ms_trigger_left/right` (5% step stage → 0..1).
- For models/states where notifications do not arrive, do not declare `live_buttons` in caps.

### 2.4 Frontend

- `store.rs`: using the same mechanism as the current stroke test (`Job::Travel`), read `live_buttons()` at 60Hz (16ms) for mice.
  ON only while the Test tab is open, OFF when leaving (same handling as the current `set_travel`).
- Add a **"Button press depth" card** to the mouse's Test page in `pages/test.rs`:
  - Two vertical bars, left and right (0-100%). A line at the actuation point; the bar uses the accent color while actuated and gray when not.
  - Current value (%), maximum during this operation, and number of actuations.
  - Line chart of depth over the last 5 seconds (two lines, left and right). Draw with `canvas` + `PathBuilder`.
  - Place it alongside the existing click-count card (same Test tab).
- In the magnetic switch card of the Performance tab, also show small live bars (left and right) next to the trigger position slider
  (lets you set the actuation point while moving it, a use the Web version lacks).

**Completion condition**: on the HTS Plus Pro (wired and wireless), pressing the left and right buttons slowly makes the bars follow, and the color changes past the actuation point.
Releasing returns it to 0. Leaving the Test tab stops the reads (CPU goes down).

## 3. See the battery in the sidebar

User request: **Devices that have battery data should show the remaining charge in the list on the left.**

### 3.1 Current problems (from reading the code)

1. `HubBackend::scan` returns `battery: None`, and `Msg::Scanned` in `store.rs` replaces the whole list with `self.devices = list` every 2 seconds,
   so a battery value that was obtained disappears at the next scan.
2. `Hub::poll_battery` only queries sessions that are open, ready and have `info.battery.is_some()`.
   Devices that are not open, and drivers whose `info()` does not fill battery, are never read.

### 3.2 How to fix

- **Contract**: add `pub battery: bool` to `Capabilities` (true for models that have a battery).
  Driver struct literals should use `..Capabilities::empty()` or add the field.
- **store.rs**:
  - In `Msg::Scanned`, carry the `battery` / `asleep` held in the previous list and the sessions over to the same ID (do not overwrite with None).
  - At startup and on plug/unplug, **open in the background** the sessions of all `supported` devices (without opening a screen). Query those with
    `battery: true` in caps via `battery()` every 60 seconds (reuse the existing 30-second interval mechanism).
  - If the driver sends `HubEvent::Battery`, reflect it immediately (mouse.fit notifies via op 10 `b == 64` / `04 03` — pick it up in the §2.3 receive thread).
- **Backend**: `mousefit8.rs` puts `battery: true` in caps, and when the device is asleep `battery()` returns the last obtained value instead of `Err`
  and sets `DeviceInfo::asleep`.
- **UI (sidebar `app.rs::side_device`)**:
  - On the line under the device name, a battery icon (4 levels by charge) + `85%`. While charging, a lightning icon and "Charging".
  - 20% or below is orange, 10% or below is red. While asleep, show "Asleep · last 85%" in a dimmer style.
  - Show nothing for a device whose value has never been obtained (not even `—`).
  - Make the Home device cards show the same.

**Completion condition**: just launching the app (without opening the device screen) shows the HTS Plus Pro's charge in the sidebar,
and it does not disappear on the 2-second scan. Plugging in a charging cable switches to the charging display within 60 seconds (immediately if there is a notification).

## 4. Notification of official firmware updates

User request: **Notify me when the official firmware has an update.** (Notify only; do not write it automatically.)

### 4.1 Official distribution sources (investigated)

| Family | Where to look | How to decide whether it is new |
|---|---|---|
| mouse.fit (G-Wolves) | `https://www.mouse.fit/Config/<UIFolder>/FWLINK.txt` — the content is one line, the zip's URL. As of 2026-11-14: HTS Plus Pro (`JM_052_3955`) → `https://www.mouse.fit/firmware/10-052-HTS_PLUS_Pro_8K/3955/HTS_PLUS_Pro_8K_3955JM.zip`, HTX Ultra (`GW_056`) → `https://www.mouse.fit/firmware/HTX_Ultra_54XG.zip`, (`JM_056_3955`) → `.../02-056-htxultra/3955/htxultra-3955JM.zip`, (`JM_056_3950`) → `.../02-056-htxultra/3950/htxultra-3950JM.zip` | The zip file name has no version. Treat it as "new firmware published" when **the URL in FWLINK.txt changes**, or the zip's **ETag / Last-Modified / Content-Length** changes. In addition, once `DeviceFWFile_00` / `Receiver4K8KFWFile_00` of `env-models.json` (currently empty) are filled in, derive the version (`DeviceNewFWVer`) from them and compare it with the device's FW |
| xsyd / Sparklink (SU66 Max) | `GET https://api.sparklinkplayjoy.com/api/v1/getKeyboardFirmware` (the SDK's `getFirmwarePack`). Pin down the arguments by finding the caller in the dshk firmware screen chunk (grep for `getFirmwarePack(`; probably VID/PID/boardId etc.) | Compare the version in the response with `appVersion` from `getDeviceInfo` (`DeviceInfo::firmware`), and notify if it is newer |

UIFolder comes from the model row in `env-models.json` (keep it in the model table of mousefit8.rs so it can be looked up from the registry's model).

### 4.2 Implementation

- **Contract**: add `pub firmware_feed: Option<FirmwareFeed>` to `Capabilities`.
  ```rust
  #[derive(Clone, Debug, PartialEq)]
  pub enum FirmwareFeed {
      /// mouse.fit FWLINK.txt (UIFolder name).
      MouseFit { ui_folder: String },
      /// Sparklink API (values needed for the query).
      Sparklink { vid: u16, pid: u16, board_id: Option<u32> },
  }
  ```
- **New crate `crates/devhub-update`** (no GUI dependency, easy to test): `fn check(feed, current_fw, last_seen) -> Result<UpdateStatus>`.
  HTTP uses `ureq` (the same `ureq = { version = "2", features = ["json", "native-tls"] }` as the reference implementation).
  5-second timeout. Offline fails silently. Result:
  ```rust
  pub struct UpdateStatus { pub new_available: bool, pub version: Option<String>,
                            pub url: Option<String>, pub fingerprint: String, pub checked_at: u64 }
  ```
- **Saving state**: in `%APPDATA%\DeviceHub\firmware.json`, "the last seen fingerprint per model and whether it was notified".
  **The first time, only record it as a baseline and do not notify** (do not show "update available" just because the app launched).
- **Frequency**: at startup, if 24 hours or more have passed since the last check, check on a background thread. In App settings, add
  "Check for firmware updates" (default ON) and a "Check now" button.
- **How to notify**:
  - Toast "A new firmware for the HTS Plus Pro has been published".
  - A small dot (badge) on the device row in the sidebar.
  - A card at the top of that device's General tab: version (if known), check time, "Open official page"
    (open the URL in the default browser, e.g. `open::that`) and "Dismiss this notification".
  - An option to also show it in the Windows notification center (`notify-rust`, proven in the reference implementation). Default OFF.
- Writing the firmware itself stays as P3 in `docs/BACKEND_BRIEF.md` (not done here).

**Completion condition**: after editing the fingerprint in firmware.json by hand and pressing "Check now", the notification, badge and card appear.
It runs as usual with no error shown even when offline. No query is made on a restart within 24 hours.

## 5. Change the font

User request: **The current font (Yu Gothic UI) is ugly, so I want to change it.**

### 5.1 Approach

- **Bundle** fonts and let them be selected in App settings (with a preview). Change the default too.
- Reference: in the reference implementation's `src/main.rs`, `BUNDLED_FONT_Z` / `bundled_font` / `register_ui_font`
  (put zlib-compressed TTFs in the exe, expand only the selected typeface and register it with `cx.text_system().add_fonts`),
  the `FONTS` list in `src/ui.rs`, and the compression tool `tools/compress-fonts.py`. The same mechanism can be ported as is
  (add `flate2` as a dependency). The font files and licenses can also be reused from `assets/fonts-z/` and `licenses/`.
- For Latin fonts that have no Japanese glyphs (Geist etc.), always set a fallback to a Japanese font
  (check how `ui_font()` / `native_font_family()` in the reference implementation handle it).

### 5.2 Candidates and default

| Display name | Contents | Use |
|---|---|---|
| **Modern (default)** | Latin/digits **Geist** + Japanese **Noto Sans JP** (or IBM Plex Sans JP) | A slim sans-serif that suits Liquid Glass. Closest to an Apple-like look |
| Rounded | Geist + Zen Maru Gothic | Soft |
| M+ Rounded | M PLUS Rounded 1c | Round and light |
| UD Gothic | BIZ UDPGothic (standard on Windows) | Readability first |
| Yu Gothic UI | Yu Gothic UI (standard on Windows, the current default) | As before |
| Meiryo UI | Meiryo UI (standard on Windows) | |

- Noto Sans JP / IBM Plex Sans JP are OFL. Limit them to a variable font or the three weights Regular/Medium/Bold, and bundle them compressed
  (to keep the exe from bloating; the reference implementation keeps about 11MiB resident for all typefaces).
- Large numbers (DPI, battery %, Test values) use Geist's tabular (monospaced) digits (`ui::big_number`).
- In `theme.rs`, change `FONT` / `FONT_DISPLAY` from constants to "values determined by settings", and add `font: String` to `prefs.rs`.
- In the "Appearance" card of App settings, add a "Font" row: dropdown + a preview of sample text (a few Japanese characters plus "ABC 1600 DPI"). Changes apply immediately.

**Completion condition**: the default becomes Modern (Geist + Noto Sans JP), with no tofu (□) for Japanese or Latin text.
Switching in settings applies to all screens immediately and persists across restarts.

---

## 6. Improvements (make it more useful than the Web version)

In priority order. Each item's scope is backend (B) / frontend (F).

1. **Follow changes made on the device itself (B+F, top priority)**: pick up the device's DPI button, profile switch, LOD/polling rate change, charging start and
   sleep/wake from notifications (op 10, report 4: the list in §2.1) and send `HubEvent::StateChanged` / `Battery`.
   The UI re-reads the affected part, and the "in use" stage on the DPI screen moves immediately. Today the screen stays stale even when the change is made with the device's own button.
2. **Game-linked presets (F)**: watch the foreground process (e.g. `VALORANT-Win64-Shipping.exe`) every 2 seconds and
   automatically apply the linked preset (register "apps that use this preset" in the General tab). Revert when the game is closed.
   The Web version has no such thing. `GetForegroundWindow` + `QueryFullProcessImageNameW` from the `windows` crate.
3. **Light startup (F)**: a setting (default OFF) to stay resident in the system tray, and faster startup for the "close after configuring" workflow
   (first enumeration and rendering within 1 second).
4. **Real-device support for the HTX Ultra and ATK F1 (B)**: `docs/BACKEND_BRIEF.md` §2.2/§2.3.
5. **Remaining mouse features (B)**: hyper mode, 4 DIY debounce values, accidental-press prevention, key combinations, sensor details,
   device/receiver lighting, onboard profiles, reset, magnetic button calibration.
6. **Docs upkeep (B)**: update the status column of `BACKEND_BRIEF.md` §2.2 to reality and fill in the verification dates.

## 7. Overall completion conditions

- Meet all completion conditions of §1-§5 (real devices are the HTS Plus Pro and SU66 Max; all screens also work in the demo).
- `cargo clippy --workspace --all-targets` has 0 warnings and `cargo test` passes.
- Update `docs/FRONTEND.md` (new screens and components), `docs/PROTOCOL_NOTES.md` (op 10 / report 4 notifications and firmware distribution sources) and
  `AGENTS.md` (current status).
