# GPUI Device HUB audit and work handoff (2026-11-15)

The latest DirectDrive review, fixes and feature ideas have been split out into the [DirectDrive re-audit](DIRECTDRIVE_AUDIT.md). D/G below are earlier verification records.

The next agent should start from **the remainder of D "make live features stoppable and reconnectable" (re-discovery on the OBS side) and the remainder of F**. A, B, C and E are implemented (below). P1 is a problem that breaks settings or blocks recovery, P2 is a malfunction of an individual feature, P3 is an operations and maintenance improvement. This round's run checks were on the Windows demo and existing fixtures. Real devices, the OBS host and vendor APIs were out of scope for this verification.

## Fixed this round

| Location | Fix and how it was checked |
| --- | --- |
| [`store.rs` / `coalesce_jobs`](../crates/device-hub/src/store.rs) | A later single EQ band change was erasing a preceding all-band setting. Thinning is now limited to consecutive changes to the same target, keeping both the all-band and the single-band change. Jobs are not dropped across a load, a different change, or a DirectDrive stop. Thinned-out writes are also subtracted from `pending`. 3 regression tests. |
| [`app.rs` / `stop_device_preview`](../crates/device-hub/src/app.rs) | When navigating to Home, Settings or another device, asks the device being left to stop live measurement and DirectDrive. The previous periodic processing only looked at the destination selected after the move. |
| [`app.rs` / `after_msg`](../crates/device-hub/src/app.rs) | On first launch, after selecting a device and going back to Home, the periodic enumeration sent the user back to the device screen. The restore candidate is now taken once from the startup settings and consumed on the first successful enumeration or a manual action. Removed the thread-local `AUTO`. |
| [`devhub-obs/src/live.rs` / `loop_body`](../crates/devhub-obs/src/live.rs) | A mutex shared with the render side was held while waiting for the HID response. Frames are now built in the worker, and the lock is taken only when publishing the result. |

## What was checked per feature

| Feature | What was checked this round | Additional acceptance checks |
| --- | --- | --- |
| Startup / navigation | The demo Home shows 5 devices, and the ATK F1 Ultimate is shown as "Detected · driver not implemented". After the fix, going back to Home on first use stays there after the periodic enumeration. On restart, the HTS Plus Pro that was open last is restored. | Check recovery from no real device, HID initialization failure and a temporary enumeration failure, with empty settings and with existing settings. |
| Mouse settings | Confirmed the battery display and a DPI stage change (400→1200) on the demo HTS Plus Pro. Existing tests verify the conversion of DPI stages, buttons, etc. | HTS Plus Pro wired and wireless, wireless asleep → wake, rate candidates for a different dongle, HTX Ultra read and a round trip of one restorable item. |
| Keyboard | Confirmed the SU66 Max Key mapping screen and the P1→P2 switch on the demo. Undo reproduced the bug with the steps in A. Existing fixtures cover keymap code conversion, performance, advanced keys, lighting, etc. | On real hardware, check individually the key mapping of each profile, live measurement, macros, calibration, and the firmware lighting after the lighting preview ends. |
| TP55 / PEQ | Applied "Bass boost" on the demo. The EQ curve and preamp changed, and "Undo" restored both (`2 items`). Existing tests verify the APO text and PEQ frames. | On real hardware, edit one band right after a preset and read back all bands. Check after securing the volume and the pre-work settings. |
| OBS / update notification | OBS is compiled during the workspace test run. For the update side, ran existing tests for fingerprint changes and VID/PID formatting. | Adding, scene switching, removal and plug/unplug in real OBS. Verify update notifications first with the fixed responses of E. |

`cargo test --locked --workspace` passed 39 (36 existing, 3 added). `cargo clippy --locked --workspace --all-targets` succeeded, with warnings in 3 existing HID verification examples. The changed Rust files pass `rustfmt --check`. The normal debug output location was held by an existing process, so the demo was run by copying the linked `target/debug/deps/device_hub.exe` to a temporary directory. The demo `%APPDATA%` was also isolated.

## Remaining tasks

### G · Real-device verification of all lighting modes — **Done (2026-11-16)**

**Method**: film the keyboard continuously with OBS Virtual Camera, rewrite the mode, palette, brightness, direction, speed and zone in turn with the probes (`litparam`/`litreact`/`litm16`/`litcolor`/`litone`), quantify the changes in frame brightness / hue / centroid / per-cell values, and also visually check representative frames.

**Confirmed mapping (wire mode → real-device behavior)**: the table in the "Lighting (group 5)" section of `docs/PROTOCOL_NOTES.md` is the source of truth. Key points:
- mode 0 = fixed color — fixes the selected palette color over the whole surface. **sel=0 is the firmware's rainbow multicolor and the saved RGB is ignored** (the official "more" slot). sel1-7 are solid slot colors
- mode 1..15 = correspond to SDK enum Y1[2..16] (wire is one less than the enum): 1 = rainbow horizontal flow ... 14 = light rain, 15 = bouncing light
- mode 16 = custom (only isCustom cells light, others off — confirmed on the real device by painting/clearing)
- mode 17-19 = key press reaction: 17 = block around the pressed key, 18 = pressed row, 19 = wide area around the press (all off while unpressed — confirmed by the user typing)
- modes 20-23 are clamped to readback 19 by the firmware (out of range does not exist)
- all dynamic modes use the selected palette color (sel=1=red makes all modes red). Brightness is effective in all modes. Direction is thought to be a reversed sweep, but a definitive real-device confirmation is still pending

**Code changes**: updated the effect list in `xsyd.rs`/`demo.rs` to the verified results (the old labels were all shifted by +1 and 16-19 were completely different things). Changed slot0 in the palette UI to show a rainbow swatch. Also fixed F1-F5 around DD (pin tracking / `dd_pre` evacuation / release / isCustom exclusion / sel consistency on import) at the same time — the tests replay reads with `mock_dd_pre_empty` and all pass.

**Preview (added 11-15, in response to the user report "it differs from the real device")**: dropped the formula approximation and **replaced it with the same mechanism as the official HUB**. Analyzing the current bundle `lighting-CVjtByxR.js` + the store (tooltip chunk) showed that the official HUB periodically polls `getKeyCustomLighting` (= `[5,3]` read of all cells) and **shows the device's current lit colors on all keys as they are**. The real-device probe `litlive` confirmed that `[5,3]` returns the live drawn color with flag=0xF0 even for non-custom cells (the old code only picked up `===255`). → Added `DeviceDriver::live_lighting` to the contract (xsyd = real-device readback, others = Unsupported); the worker in `store.rs` sends `Msg::LiveLight` at ~6Hz while the Lighting tab is shown, and `lighting.rs` draws `session.live_rgb` if present, falling back to the old `preview_rgb` approximation only while it is absent. When live is enabled the "Preview on device" button is not shown (it has no meaning). By definition, pattern, motion and color now match the real device.

**Bug fix (added 11-16, in response to the user report "completely different")**: `live_lighting` used the flag byte to decide whether a cell is lit, so in a session with no writes all cells had flag=0 and **it always returned an empty map, so the approximation fallback kept showing**. Real-device verification established that the flag is a marker, not a lit indicator (0xFF=custom / 0xF0=written by firmware / 0=unmarked) — BGR always returns the current frame's color. Fixed the decision to `flag!=0 || BGR!=0` and confirmed with litpoll that live colors for 66 keys are obtained.

### H · Comprehensive analysis of the official HUB/SDK/firmware — **Done (2026-11-16)**

**Method**: 4 parallel analyses (all mouse.fit chunks / protocol layer of xsyd-sdk.js ≡ keyboard-BQAULKwa.js ≡ dshk-index-XpTpGr6X.js / audit of the internal implementation / all app chunks of hub.xsyd.top) + exhaustive group/sub probes on a real SU66 Max (grpsweep/grpdeep/kbdstat/bootapp).

**New facts confirmed on the real device**:
- **group 8 FirmwareUpgrade**: `[8,1]` AppToBoot → the device moves to the bootloader (response signature: only `[1,1]`/`[1,2]`, `[1,3]` UNSUP, groups 2-13 all dead). After `[8,4]` BootToApp it fully recovers in ~4.5s — verified that **recovery is possible without unplugging and replugging**. `[8,2]` Flash / `[8,3]` ValiDate were not sent because their packet formats are unverified (firmware writing will not be implemented in the future either unless there is a safe design)
- **`[2,15,0]` ECHO** (a gap number in the enum but the handler remains, values are all 0), **`[2,17,0]` ECHO** — arg=0 is safe. With arg>0 a detach was observed, and **GFSOpenWebDriver (transition to compatibility / WebDriver mode) is the most likely cause** — sending non-zero arguments remains forbidden
- **`[10,*]` odd subs (3,5,7,9,11) ECHO** — an OEM extension slot (possibly the shell color / keyboard color family), data all 0
- Groups 0,11,12,14,16-20 are UNSUP on this device. Group 9 seems to be haptics-related but only ECHOs with no hardware

**New facts confirmed by bundle analysis**:
- **Macro key code space = 0xF500-0xF50F (62720-62735)** — implemented bidirectionally in `code_to_action`/`action_to_code` (`KeyAction::Macro` is no longer Unsupported)
- **All categories of special key codes settled** (list in the key code section of PROTOCOL_NOTES.md): layers 61696-61699, system 61952-61961, main light 62208-62216, decoration areas 62224-/62240-/62256-, ThreeMode 62466-, mouse 16384+256n, movement 17920/18176/18432/18688, wheel 18945/19201
- **Leftover DeviceInfo bytes**: `r[12..16]`=PCB id, `r[17..29]`=serial ASCII, `r[29..41]`=manufacturing timestamp (whether to expose them via caps/model needs consideration)
- **`[3,5]` GetKeyLayoutStyle** self-describes the physical layout with `{row:5,col:7,ratio:4}`×~15 (ratio 12=knob, 13-15=screen) — the constants already exist for the SU66, and will be needed for a future model with a different layout
- **The official import writes both the WIN and MAC tables** (both OSes' worth by switching `setSystemType`) — we only write the current OS side (known difference)
- **The official export is AES-256-CBC+EVP_BytesToKey encrypted** — decrypting import is implemented, but official-compatible encrypted export is not
- **The official preview = `[5,3]` real-device readback polling** (the same conclusion as §G, confirmed from the SDK side too)
- Remaining mouse.fit items: ClearSetting(op9), magnetic button calibration (op46), tactile feedback (EEPROM 243/249), SensorVirtualPos (6984/6986), and some of the HyperMode / scroll-scan family

**Fixes implemented this time** (in response to the internal audit findings):
- Fixed `read_lighting` swallowing per-chunk partial success → a read failure now makes the whole thing an error (prevents the accident where an incomplete setting is treated as complete and a later write wipes the device's custom colors)
- Fixed `live_axis` zero-filling all errors; it now propagates Disconnected etc. (the problem where it kept returning 0 after disconnection as a zombie measurement and never reached `TravelStopped`)
- Added `live_lighting` to capabilities (removes the exception to the principle that the UI looks only at caps)
- Bidirectional `KeyAction::Macro` ↔ 0xF500+i (16 slots, connected to the UI's macro group `kc.macros`)
- TP55 demo eq_def declaration inconsistency, HTX Ultra SPDT declaration/handler inconsistency (dropped the declaration), `4..0` slice panic when `ee_read` fails in `mousefit8.rs`

**Remaining practical differences (in priority order)**:
1. Writing both WIN/MAC keymaps (import/reset_keymap only handle the current OS — real harm for users in MAC mode)
2. Turning the `[3,5]` layout self-description into caps (prerequisite for supporting models with other layouts)
3. Per-cell custom writes for the Decorate area (area1) (import doesn't parse custom, only area0 is supported)
4. Official-format export (being able to write encrypted JSON would give round-trip compatibility with the official import)
5. `[5,6]` Caps Lock indicator color (write-only, still needs consideration)
6. `[2,9]` default axis slot, `[3,3]` single-key read, exposing SpecialLightingQuery, the axis-name HTTP API (`api.sparklinkplayjoy.com` getAxisList → currently shown as "Axis N")
7. Group 16 Handle / compatibility mode (low priority since the SU66 has gamepad ext=0) and final identification of `[2,17]`=GFSOpenWebDriver (need to confirm the re-enumerated mode)
8. OBS side: per-device re-discovery, showing the healthy side when one side errors, ABI/texture verification on a real OBS host (§D)

### A · P1: keep Undo and presets from writing across onboard configurations — **Implemented (2026-11-15)**

**Implementation**: `Job::Profile(u8)` is a dedicated job that bundles the switch + a full re-read. `Hub::apply` routes `Change::Profile` to it, and while `Session.switching_profile` is set, edits within a configuration are stopped both in the UI (`Shell::apply`, with a toast) and in `Hub::apply`. If the switch or the re-read fails, the held values are discarded (`state=None`, `switch_failed`) and re-read. `diff_changes` does not emit `Profile` (it does emit `ProfileName` — renaming can now be undone). Undo/redo/preset application to another configuration is queued in `Shell::pending_restore`, and after the switch + re-read succeeds only the diff against the new configuration's state is written. On failure it returns to the history with 0 writes. Presets/import reject a different model or a mismatched EQ band count with `state_compatible`, warn about the number of unsupported items, state explicitly that it is "applied to the current configuration", and then normalize `profile.active` to the current value. The demo backend keeps a saved state per configuration and returns the factory state for configurations not yet visited — the same behavior as a real device where P1/P2 have different values.

**Verification**: 5 `store.rs` tests — separation per configuration, blocking edits during a switch + re-read (through the real worker), `ProfileName` diff, `state_compatible`, `Job::Profile` can't be thinned. All workspace test/clippy pass. Profile switch + undo on a real device is unconfirmed.

**Remaining issues**: the success toast is based on the number of items submitted (a failure is reported via the `Applied` path + re-read). No "Switched to configuration N" toast is shown on a successful switch.

### B · P1: don't roll the UI display back by report order or reconnect — **Implemented (2026-11-15)**

**Implementation**: added `submit_seq`/`read_seq`/`pending_changes` to `Session`. When `Msg::State` arrives, unconfirmed changes submitted after the read was issued are overlaid on the confirmed state (an old read doesn't roll things back). `Msg::Scanned` carries a generation `gen`, and old enumerations are discarded (against backend switching). On `Disconnected`, in addition to the backend's invalidate, the worker-held `driver` is set to `None` — a reopen pulls a new handle.

**Verification**: `state_replays_writes_issued_after_the_read_started` — changes made after the read started don't vanish with an old State. A real-device disconnect → reconnect round trip is unconfirmed.

### C · P1: don't treat partial HID reads as complete settings — **Implemented (2026-11-15)**

**Implementation**: all required items in `xsyd::read` now propagate with `?` — per-key performance (removed defaulting), advanced keys (removed returning a partial list on a mid-way break), macro bodies (removed `unwrap_or_default`), each lighting area, profiles (removed `unwrap_or_default`). Closed the path where "an advanced key that doesn't exist = deleted" is misread in a diff. `mousefit8`'s minimal response when asleep is distinguished by `info.asleep`, so it is left as is.

**Verification**: all existing tests pass. A partial-read test with failure injection is not done (needs fake responses).

### D · P2: live features — **Main part implemented and confirmed on real hardware (2026-11-16)**

**Implementation**: when the worker stops because live measurement failed, it sends `Msg::TravelStopped`, and the UI side resets `travel_on` and the depth buffer and shows a toast. On exit `Shell::Drop` → `Hub::dd_stop_all()`, and in addition `XsydDriver::Drop` tries, on a best-effort basis, to read back and restore the tracked cells. `direct_drive` tracks pin/color, sends only changed cells to DD, releases cells that have gone out via `[5,4]`, and does not send empty/unchanged chunks. Added to Live a hybrid mode, "use the keyboard's effect as the background".

**Real-device verification (OBS Virtual Camera)**: with 30fps DD only the 7 target cells are red, and non-target cells continue the firmware effect. Pins remain after the stream stops (the firmware has no DD timeout). `[5,4]` release / `DdStop` returns the whole surface to the firmware. **Remaining**: per-device re-discovery on the OBS side, showing the healthy side when one side errors, texture/ABI on a real OBS host, an 8K input histogram with the user typing.

**Addition (2026-11-16)**: following the existing 4 modes (press depth / typing heatmap / key highlight / ripple), added 3 modes as "meaningful DD uses" — `Status` (Caps lights on CapsLock + on wireless models the number row becomes a gauge at battery 40% or below and blinks fast at 15% or below), `Audio` (WASAPI loopback → Goertzel 15 bands 60Hz-16kHz → column-height spectrum), `AppFocus` (GetForegroundWindow → automatically selects the preset in `app_map` by exe name). `Engine::frame` receives `FxEnv` (caps_lock/battery/audio/app_focus/fg_app), and `audio.rs` (windows 0.62 WASAPI) was newly added. Invalid-cell judgment in hybrid mode is, as before, matching `off_color`, and in all modes the press flash is superimposed by the common path. Added 3 tests (status_caps_and_battery / audio_column_height / appfocus_uses_resolved_keys).

### E · P2: separate update "available / none / unknown" correctly — **Implemented (2026-11-15)**

**Implementation**: `version_newer` (a.b.c numeric sequence comparison, absorbing differing digit counts with blank padding) — a version that can't be compared is not called "newer". Notification by fingerprint change is **limited to families whose version can't be obtained** — fixes the problem where a header fingerprint update of the same distribution made the card appear/disappear on Sparklink. **Remaining**: extending to represent "unknown" in the type (a three-valued UpdateStatus) is not done. Fixed-response tests are not done.

### F · P3: documentation consistency — **Partially done (2026-11-15)**

**Done**: updated the example list in `AGENTS.md` to the real ones (`enum`/`kbdiff`/`lite2e`/`tp55e2e`). Split the "5219 wired path unverified" in `BACKEND_BRIEF.md` §2.2 into "report-8 verified / 64B feature-report driver path unverified". **Remaining**: splitting the responsibilities of store.rs (separating the worker from pure state diffs), unifying the save handling of prefs/fwstate/presets, organizing/documenting the probe examples.

### D remainder details (old audit notes)

In [`devhub-obs::live`](../crates/devhub-obs/src/live.rs) re-discovery only happens when both targets are `None`. A target found once remains after an error. Even when only one side errors, [`render::draw`](../crates/devhub-obs/src/render.rs) returns only a status, so even the side that is responding gets hidden. OBS should separate per-device presence and transient errors, re-discover differences at a fixed interval, and keep showing the healthy side. Also needs checking: explicit stop at DirectDrive exit / session teardown and reflecting stop failures. The thread for OBS texture creation/destruction and the ABI for applying should be checked separately against the supported OBS's source and a real OBS.

### E remainder details (old audit notes)

`store::handle` shows "up to date" on a manual check whether there is no diff, no distribution, or it was dismissed. Tests that verify old version, same version, new version, current version unknown, no distribution, abnormal response and dismiss with fixed responses remain.

### F remainder details (old audit notes)

- Splitting the responsibilities of [`store.rs`](../crates/device-hub/src/store.rs): separate the worker from pure state diffs, then extract route transitions and history management from `app.rs`. Avoid large moves.
- Unify the settings-directory resolution and save handling of [`prefs.rs`](../crates/device-hub/src/prefs.rs), [`fwstate.rs`](../crates/device-hub/src/fwstate.rs) and [`presets.rs`](../crates/device-hub/src/presets.rs). Preset saving unconditionally overwrites a same name after normalization, so return conflicts to the UI.
- Organize the probe examples (`probe*.rs`, `kbdump`, `kbraw`, etc.) or state their purpose with something like an `examples/README`.

## Conditions to hand to the agent

1. Read `AGENTS.md`, and the UI branches on caps. Scalars are `SettingSpec`, operations are `ActionSpec`. Changes to the core contract are additive only, and the demo is updated too. GPU backdrop and `vendor/` are outside the scope of this series of tasks.
2. Demo or a fake backend first, then a real device only if necessary. On a real device, close the Web HUB and the HID sessions of other Device HUB / OBS instances, export the settings, then check the read-back. Do not include initialization or DFU in the normal smoke tests.
3. Proceed A→B→C in order. The B fix and the C UI integration touch `store.rs`, so don't edit them simultaneously. OBS is independent as part of D, and the update decision logic and tests are independent in `devhub-update`. For each implementation, return the reproduction steps, the result after the fix, and the range confirmed on a real device.
4. Basic commands: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets`. For a manual demo, set `$env:DEVHUB_DEMO = '1'` in PowerShell and then run `cargo run --locked -p device-hub`. If a `target\debug\device-hub.exe` is left open, it can't be overwritten at the normal output location because of a Windows file lock, so close that instance or use a different output location.

## §I OBS plugin fixes (2026-11-15)

A record of eliminating the "source appears but is transparent" problem with E2E.

- **Placement**: OBS 30.2+ doesn't scan `%APPDATA%\obs-studio\plugins`.
  `C:\ProgramData\obs-studio\plugins\<module>\bin\64bit\<dll>` is the proper location.
- **`obs_module_load` false**: `gs_blend_push/pop` don't exist.
  The correct ones are `gs_blend_state_push/pop`.
- **`OBS_SOURCE_CUSTOM_DRAW` = `1<<3`**. `1<<7` is `DO_NOT_DUPLICATE`
  (when unset, video_render is called from inside a pass of the default Draw technique,
  our own technique_begin is nested, and the drawing is discarded).
- **`GS_DYNAMIC` = `1<<1`**. `1` is `GS_BUILD_MIPMAPS` — a non-dynamic texture makes
  `gs_texture_map` fail → `set_image` silently skips → transparent texture.
- **`obs_source_draw` is valid only inside a pass**. The canonical pattern:
  `while (gs_effect_loop(effect, "Draw")) obs_source_draw(tex, 0,0,0,0,false);`
- Isolation procedure: record create/tex/first draw in `%TEMP%\devhub_obs.log` +
  check whether `gs_clear` can fill the whole RT (it could → the target is fine,
  which established that it was a texture/sprite-side problem).

### Key press display flicker (same day)

- **Cause**: the SU66's `[4,3,kind,row]` axis data is an **autonomous stream on the device side**
  (cycling through all 20 sets in ~16ms). `cmd()` in `xsyd.rs` "treats the first group-4
  packet that arrives as the response", so it picked up frames of a different kind/row, the ADC value got mixed into
  `AxisSample.status` → the renderer wrongly showed a press → the orange outline flickered.
- **Fix**: `axis_row` receives by itself without going through `cmd`, and keeps reading until a frame whose echo (r[2]=kind, r[3]=row)
  matches. Since `route_mm`/`live_axis` both use the same path,
  the Test page's press depth is fixed too.
- Verification probes: `example axraw` (raw response dump, measured stream ~1270pkt/s),
  `example axstat [seconds×10]` (status/route transitions on press/release).

### Source separation / properties (same day)

- Registered 3 sources: `devhub_live` (both) / `devhub_live_kb` / `devhub_live_mouse`.
  Per-mode canvas height (Both 380 / Kb 250 / Mouse 200).
- The poller is shared in-process via `live::shared()` (one HID session even with 3 sources).
- Properties: title / label / accent color + a "Per-key display toggle" group
  (generates `hk{i}` checkboxes from the layout, checked = hidden) + "Per-button…"
  (`hb{i}`, real names only when a mouse is connected. When not connected, a guidance text).
- `update` scans `hk{i}`/`hb{i}` up to 0..160/0..24 — the default false = shown,
  and unset is also false, so picking up only the saved true values lets it restore even if get_properties was never run.
- Renderer overhaul: rounded keycaps + depth fill (accent, opacity proportional to depth)
  + glow on press + a horizontal fader for the mouse (track / trigger line / %).
