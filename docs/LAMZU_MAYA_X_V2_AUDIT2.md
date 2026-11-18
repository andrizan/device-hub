# LAMZU MAYA X V2 (LM20) re-audit — 2026-11-17 (2nd)

Target: Device HUB `249b806` (after the fixes from the previous audit [LAMZU_MAYA_X_V2_AUDIT.md](LAMZU_MAYA_X_V2_AUDIT.md)) and the official LAMZU Aurora.
On 2026-11-17 15:30 JST, the distributed files of the [official Aurora](https://www.lamzu.net/#/project/items) were re-checked.
The Maya X (LM20) model definition is unchanged from last time. LM20's initialization goes through the branch where fetching the sensor model is enabled,
and involves converting the polling rate.
The `K2()` / `uM()` path for another manufacturer (`vid==13652`=0x3554) is not used for LM20.
This time **no code was changed** and nothing was written to the real device (because another agent is editing `lamzu.rs` at the same time).

## Status (night of 2026-11-17 — all fixed)

The findings in §2 to §5 were implemented in this session (write tests on the real device were not done; confirmed with the demo and the read path).

| Finding | Response |
|---|---|
| P1 Sleep OFF | The choices are only 1/5/10/15/20/25 minutes. Invalid read values are not put into the settings |
| P1 Full reset | Sent only once with `send_only`; for wireless, also sent to target 0 (the receiver) after 1 second. The automatic resend in `cmd()` is limited to commands other than resetProfile(13) |
| P2 Wireless 500 Hz | Caps now differ by connection type (wireless 1K–8K), `16` → 1000 Hz |
| P2 20K FPS dependency | Turning competitive mode OFF also writes OFF for 20K FPS. The UI also shows it disabled via `SettingSpec::enabled_when` (new) |
| P2 Read failure | On failure, not put into settings |
| P2 Paired device model | Model table in `devhub-core/src/lamzu.rs` + GetDevicePID. Unknown ones are `supported=false`; added 0050/0052/0056/0046/0054 to the registry |
| P3 Rapid fire | Left click only |
| Glass mode | Restored with the official frame (if it cannot be written, an error on read-back). LOD is shown disabled when ON |
| AdaDPI | Added `AdaptiveCurve` / `AdaptiveCaps` / `Change::Adaptive` to the contract. Curve editing on the Performance tab (3 presets + drag) |
| Display names §4 | All changed to the official names |
| Analysis document §5 | Corrected PROTOCOL_NOTES §5.4/§5.5, added §5.8 |
| Still unsupported | Additional button functions (LOD cycle etc.), macros, FW update notifications |

## Conclusion

- **The protocol (sent/received byte sequences) matches the official.** The previous P1s "response matching", "write when the profile is unknown", and "button send values" are fixed correctly.
- **There are still 4 behaviors that differ from the official (P1×2, P2×2)**, **6 missing features**, and **12 display-name mismatches**. Fix instructions follow below.
- The analysis document (`PROTOCOL_NOTES.md §5`) has **4 places with errors or misleading statements**.

## 1. Items confirmed to match the official (byte for byte)

| Feature | Official (function) | App | Verdict |
|---|---|---|---|
| Frame | `sendFeatureReport(0, t)`, `t[2]=target,t[3]=sub,t[4]=f,t[5]=cmd,t[6..]=args` | the same layout from `w[3..]` | Match |
| Response | `r[1]=A1`, value at `r[8]` (new protocol) | the same + matches target/f/cmd/devID/button number/length | Match (stricter than the official) |
| Polling | `[2,2,1,128/0,prof,(v)]`, `fM`: 500→2, 1K→1, 2K→32, 4K→64, 8K→128 | the same | Match |
| LOD (PAW3955) | `T2`: with sensor=4 the candidates are `0.7;1;1.3;1.5;1.7`. `setLOD`: <1 → `v*10\|0x80`, 1<v<2 → BCD, otherwise → the value | the same | Match |
| Motion sync / straight-line correction / ripple / competitive mode / 20K FPS / X-Y axis separation | 137/9, 132/4, 138/10, 139/11, 147/19, 141/13 of f1 | the same | Match |
| Angle adjustment | `[2,2,1,148/20]`, negative values are `256-|v|`, UI is −30 to 30 | the same | Match |
| Center offset | `[2,3,1,149/21]`, set uses `o[8]` (o[7] is free), UI is −5 to 5 | the same | Match |
| Debounce | `[2,2,0,136/8]`, UI is 0 to 15 ms (`rM` rejects out of range) | the same | Match |
| Sleep | the LM20 path uses **seconds** (`x2("1min…")` = 60…1500), `[2,3,0,135/7]` u16BE | seconds | Match (except the OFF choice → §2) |
| Fast trigger | `[2,3,0,154/26,prof,L,R]` | the same | Match |
| Scroll Bhop | `[2,4,0,153/25,prof,direction,ms16]`, direction 0/2/3, 200 to 1000 ms | the same | Match |
| AdaDPI | `[2,9,1,154]` read (`a[8]`=enabled, `a[12]`=number of points, point = speed u16 + multiplier×100 u16), write `[prof,en,0,N,…]` sub=4+4N | the same | Match (enable/disable only) |
| DPI stages | read `[2,10,1,129,prof,6]`, write `[2,26,1,1,prof,N,(X,Y)×N]`, color `[2,19,2,129/1]` (`aM` uses `t[8..]`) | the same | Match |
| Buttons | read `GetButton(prof,btn,255,[],10)` = `[2,15,3,128,prof,btn,0,255,10]`, type `r[10]` / length `r[11]` / value `r[12..]` | the same | Match |
| Button values (`jM`) | DPI = type7 + 6/1/2/5, lock `[5,Xh,Xl,Xh,Xl]`, profile cycle type8+[3], polling cycle type13+[3], macro 16/17/18 | the same | Match (the previous P1 is resolved) |
| Battery | `getBatPer` → `[o[7], o[8]]` = (charging, %) | the same | Match |
| Paired device | `GetDevicePID` `[1,6,0,139,2]` → VID `r[9..10]`, PID `r[11..12]` | the same | Match |
| Dongle LED | `T2`: shown when `!isWired && pid==0x48`. ON=`SetLightness(1,prof,0,255)`, OFF=`SetLightEffect(1,prof,255,0,…)`, each light = idx 5/3/4, values 6/11/10/12, read from `r[9]` | the same | Match |
| Profile reset | `resetProfile` `[2,1,0,13,prof]` | the same | Match |

## 2. Behaviors that differ from the official (fixes needed)

| Priority | Finding | Grounds and fix |
|---|---|---|
| **P1** | **Choosing sleep "OFF" writes 1 minute** | `sleep_min` in `lamzu.rs` has the choice `0 = OFF`, and `apply` writes `clamp(1,60)*60` = 60 seconds. The official `SleepTimeGrade` is only 1/5/10/15/20/25 minutes, and there is no way to write OFF (0/0xFF00/0xFFFF are only **displayed as** "disabled" **when read**). → Remove OFF from the choices. When the read value is OFF, only show "Disabled (on the device)" as the display (do not make it a write candidate). |
| **P1** | **Full reset waits for a response, resends, and does not reset the dongle** | The official `defaultDevice()` is fire-and-forget (no `getReport`). For wireless, `Xv()` **also resets the receiver** with `defaultDevice_id(0)` after 1 second (LM20, non-Mini). The app waits up to 3 seconds in `cmd()` and **resends twice** in the meantime (`i==8,16`) before returning a timeout error → the device is already reset but the UI shows failure, and the receiver remains. → Make `factory_reset` a dedicated send-only function that sends once, and for wireless sends the same frame to target 0 after 1 second. Limit `cmd()`'s automatic resend to commands where "resending the same value does no harm". |
| P2 | **500 Hz can be chosen on wireless** | The official uses `PollingRateWired` (500 to 8000) when `isWired`, and for wireless `PollingRateWireless` = **1000/2000/4000/8000**. The app offers 500 for both wired and wireless. → With `build_caps(transport)`, remove 500 for wireless. Interpret the read value `16` as 1000 Hz as the official does (currently it is shown as "16"). |
| P2 | **20K FPS can be operated independently of competitive mode** | Official `s2()`: turning competitive mode OFF sends `setTrackingMode(false)` to disable 20K FPS (`trackingModeDisable`). The hint text also says "enable Competition Mode first". → When changing to competitive mode OFF, also write OFF for 20K FPS. In the UI, attach a dependency condition (`visible_if`/disabled) to `SettingSpec`. When turning 20K FPS ON, show a battery-consumption confirmation as the official does. |
| P2 | Read failures turn into default values | `self.get(2,1,128,dev).unwrap_or(1)` in `read()` **shows 1000 Hz** when it cannot be read. → On failure, do not put it into `settings` (the UI shows "cannot be read"). |
| P2 | **Maya X features are shown even when the paired device is not a Maya X** | `0048` is shared with Maya (0050), Maya M (0052), and Orcus V2 (0056). `info()` always returns `supported=true`, and there is one kind of caps. For Orcus V2 the official has `SensorOffsetEnable=0`, `DynamicSensivtyEnable=0`, `GlassModeEnable=0`. → Build caps from a per-paired-PID model table (the env-models values), and make unknown PIDs `supported=false`. Also show Maya / Maya M / Orcus V2 as the product name. |
| P3 | Target button of rapid fire (X click) | The official `xClickToolStripMenuItem` is always the left click (`o[1]=LeftButton`). The app can write buttons other than left. To avoid writing values that are unverified on the real device, either fix it to left as the official does or label it "experimental". |

## 3. Features the official has and the app lacks

| Feature | Official | Proposal |
|---|---|---|
| Glass mode | Shown when `GlassModeEnable=1`. `setGlassModeDetect(v)` = `[2,1,1,14,v]` (no profile), ON disables LOD editing | The app hides it because "set has no effect". Re-verify with the same frame as the official (`[v]` only, sub=1), and if it still has no effect, showing it with a note "not applied on FW 0.0.3.0" is more faithful to the official. Show LOD as disabled when ON. |
| AdaDPI curve editing | Presets Classic / Natural / Leap / Custom, sends per-point (speed 0–105, multiplier 0–1.5) with `setAdaptiveDPICurve` | Currently only enable/disable. The values of the 3 presets are in the bundle (Classic 0/20/40 × 1.0, Natural 1.5/0.75/0.1, Leap 0.1/0.75/1.5). The point editor can be built with the same components as the EQ screen (frontend owner) — the contract needs the addition of `AdaptiveCurve { points: Vec<(u16 speed, f32 ratio)> }`. |
| Button functions | LOD cycle (type14+[3]), competitive mode toggle (type20+[0]), left double click (type2 `[1,1,2,0,100]`), media player (387), WWW home (547) | Reads are already held as `Unsupported`. Add them to the write candidates (add to `MouseFn`/`MediaFn`, or a generic "official function"). |
| Macros | 3 kinds (count / until release / until re-press), up to 30, `AllocateMacroDataSize`/`SetMacroData` 50B at a time | Remains unsupported (as before). |
| Pairing / FW update | Pairing procedure (3 buttons for 3 seconds), update of `DeviceFWFile_00` (v0.0.3.0) | Writing can stay out of scope, but **update notifications** are possible with `firmware_feed`: take the version from the file names of `DeviceFWFile_00` / `Receiver4K8KFWFile_00` in `env-models.json` and compare it with the device's version. |
| Notice on the first debounce change | A confirmation dialog only the first time (recommended 2–4 ms) | Attach wording equivalent to `SettingSpec::confirm`. |

## 4. Mismatches with display names (the official Aurora's Japanese display)

The official strings come from the Japanese dictionary of `DxSz` (`JP:"Japanese",HOME:"(Home)"…`). Items without a Japanese translation (AdaDPI, center offset) are shown in English even in the official app. In the "Official" column the Japanese labels are rendered in English.

| App (current) | Official | Proposed fix |
|---|---|---|
| Motion sync | **Movement sync** (MotionSync) | Movement sync (Motion Sync) |
| Ripple control | **Ripple** | Ripple |
| Lift-off distance | **LOD** | LOD (lift-off distance) |
| Tracking mode (20K FPS) | **20K FPS** | 20K FPS |
| DPI X/Y separate | **X-Y axis separation** | X-Y axis separation |
| Sensor angle | **Angle adjustment** | Angle adjustment |
| Sensor offset | Chinese "center offset" / English SensorCenterOffset | Sensor center offset |
| Time until sleep | **Sleep time** | Sleep time |
| B-hop (wheel misoperation) / up / down | **Scroll Bhop** / scroll up / scroll down / off | Scroll Bhop |
| B-hop time | **Wheel response time** | Wheel response time |
| Dongle LED's "signal quality" | **Signal** | Signal |
| Reset this profile / Restore factory state | **Reset profile** / **Full reset** | Match the official names |
| Adaptive DPI | AdaDPI (Chinese "DPI adaptive") | AdaDPI (speed-linked DPI) |

Differences in descriptions: for ripple the official says "eliminates wavy pointer jitter during fast mouse movement" (the app says "fine jitter at high DPI").
For Scroll Bhop the official says "disables the first scroll signal, and does not scroll if there is no second one within the response time".
For 20K FPS the official says "20kHz scanning, higher power consumption than competitive mode, enable competitive mode first".
Names that match: debounce time, straight-line correction, polling rate, fast trigger, competitive mode, dongle LED, left click / right click / wheel click / back / forward.
Product name: the official model row is `Maya X(LM20)` and the product page is `LAMZU MAYA X V2`. The app's `LAMZU Maya X V2` is acceptable.

## 5. Corrections to the analysis document (PROTOCOL_NOTES.md §5)

1. §5.5 describes `SpeedEnable` as "the **rapid fire** function for left and right each" → the official name is **fast trigger** (fastest key response, possibility of accidental double clicks). Rapid fire (X click) is a separate function (button type2).
2. §5.4 "LOD: the official writes 0.7 as value 3" → that is the path for another manufacturer (`K2`). **LM20 (sensor 3955) writes `0x87`** (`T2` + `setLOD`).
3. In the `defaultDevice` row of §5.5, add "for wireless, also resets the receiver with `defaultDevice_id(0)` after 1 second (`Xv`)" and "does not read the response".
4. In §5.1, add the wireless polling candidates (1000–8000, no 500) and that 20K FPS depends on competitive mode (`s2`).

## 6. Work instructions for another agent (in this order)

1. Remove OFF from `sleep_min` (P1).
2. Make `factory_reset` send-only + also the receiver for wireless (P1). Limit `cmd()`'s resend to read-type and idempotent writes.
3. Remove 500 from the wireless polling candidates and read `16` as 1000 Hz (P2).
4. When competitive mode is OFF, also set 20K FPS OFF (P2).
5. Do not fill failed `read()` results with default values (P2).
6. Paired PID → model table (Maya/Maya M/Orcus V2) and caps selection, unknown ones `supported=false` (P2).
7. Change the display names to the proposals in §4 (labels and help of `build_caps`).
8. Re-verify Glass mode with the official frame. Write the AdaDPI presets (discuss the contract addition with the frontend).
9. Correct `PROTOCOL_NOTES.md §5` as described in §5.
10. Do real-device checks with both Aurora and this app closed, **save the current values before writing**, and read back one item at a time. Do not test the full reset.
