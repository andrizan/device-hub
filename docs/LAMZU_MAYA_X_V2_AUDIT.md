# LAMZU MAYA X V2 / LM20 audit — 2026-11-17

The targets are Device HUB `e75c453` and [LAMZU Aurora v1.0.38](https://www.lamzu.net/#/project/items).
The [official model definitions](https://www.lamzu.net/Config/env-models.json), the distributed shared bundle
`index-DxSz_fAU.js`, and the settings screen `index-Bn9q780G.js` matched, byte for byte, the local comparison files fetched last time.
The [official product name](https://lamzu.com/products/lamzu-maya-x-v2) is **LAMZU MAYA X V2**, and Aurora's model row is
**Maya X(LM20)**. The old `37B0:001F` "MayaX V2" is a different revision and out of scope this time.

**Verdict: the basic reads work. It cannot be called equivalent to the official HUB.** The priority fixes are rejecting invalid or stale responses,
forbidding writes when the active profile is unknown, and fixing the values sent for button assignments.

## What was observed on the real device

Only the wireless `37B0:0048` was enumerated on this Windows machine. The wired one could not be re-read this time, so
the [existing wired frames](../crates/devhub-hid/fixtures/lamzu/lm20-wired-active.txt) were used for the comparison.
For wireless, after closing Aurora and Device HUB, only GETs of the official frames and the app's `scan / driver / info / read` were run.

| Target | What was read on the real device |
|---|---|
| Receiver / paired device | `LAMZU Orbital 8K Dongle` `37B0:0048`. `[1,6,0,139] [2]` → `A1 [02 01 37 B0 00 44]`. The paired device is `37B0:0044`. |
| FW / sensor / battery | The receiver FW at target `0` and the mouse FW at target `2` are both `0.0.3.0`. Sensor code `4` = PAW3955. Battery 100%, no charging flag. The app sets the receiver FW to `None`. |
| Profile / DPI / polling | Obtained profile 1, 8,000 Hz **setting value**, DPI 400/800/1600/3200/6400, the 3rd stage selected, per-stage RGB, and LOD 0.7 mm. Not a measurement of the communication interval. |
| Buttons | Left, right, wheel click, and the 2 side buttons are already assigned keys (HID usage `31` / `30`). Read only. |
| Performance | Motion Sync / straight-line correction / ripple OFF, competitive mode / 20K FPS ON, angle `1°`, left fast trigger ON, wheel function OFF / 200 ms, Adaptive DPI OFF. |
| Receiver LED | LED 1/2/3 are `10/11/6` (polling rate / DPI / battery). The master mode value is `FF`. Aurora's `K0()` judges ON if there is a known LED setting and the master value is non-zero, but the app only checks `==5` and **shows OFF incorrectly**. How it actually lights is unobserved. |
| Mouse DPI indicator | Even wireless: `[2,2,2,132] [1]` → `A3`. The official model definition has `DPIIndicatorEnable=0`. The DPI stage color and the receiver LEDs are separate functions. |
| Glass mode | Official model definition `GlassModeEnable=1`. Wireless: `[2,1,1,142]` → `A1 [00]` (OFF). The saved wired frames' GET is also ACKed. Neither write nor effect was verified this time. |

## Fix priorities and grounds

| Priority | Finding | Behavior and grounds |
|---|---|---|
| P1 | Response matching and length checks are insufficient | `lamzu.rs:30–87, 231–244, 748–769`. `cmd()` matches only the group and command, and does not check the target, profile, button number, or `7+a[4]`. Consecutive button GETs share the same group/command, so a stale neighboring button response may be adopted. Short DPI/button responses may panic on slicing. A DPI count=0 is rounded to 1, so an abnormal value like the previous p4 is converted into normal data. Separate the matching fields for GET/SET, verify length, range, and agreement with the request, and do not finalize a write result by the ACK alone. |
| P1 | Writes and initialization to profile 1 are possible when the profile is unknown | `lamzu.rs:222–227, 803–804, 1009–1015`. A communication error or an invalid active ID is replaced with 1. If another profile was being edited, profile 1 is changed, and "Reset profile" may erase profile 1, which is not the one being handled. Make obtaining active fallible, and report failures for both reads and writes. The settings UI keeps the last good state and makes a profile whose read failed non-writable. Showing 4 slots and judging whether a slot is initialized are separate matters. |
| P1 | Button send values differ from the official | `lamzu.rs:346–349, 395–396`. Aurora's `jM()` uses type `7` + `[6]/[1]/[2]` for DPI cycle/+/−, `[5,Xhi,Xlo,Xhi,Xlo]` for lock, and type `8/13` + `[3]` for profile/polling cycle. The current implementation sends type `1` + `[20/18/19]` for DPI switching, `0,0` for the lock's Y, and `[0]` for cycling. Keep compatible input/output with the existing code separate from the official frames sent to a connected LM20. |
| P1 | A shared receiver is treated as MAYA X V2 without checking the paired device | `registry.rs:181–194`, `lamzu.rs:105–122, 597–620`. `0048` is shared with Maya / Maya M / Orcus V2 etc. in the official table. This unit was measured to be a MAYA X V2, but the current driver does not use `GetDevicePID` and returns the same name, caps, and `supported=true` even when disconnected or a different model. For wireless, use the receiver name as the initial value, and finalize the product name and caps after confirming the paired device's VID/PID. Re-judgment is also needed on swap, reconnection, and no response. |
| P2 | Incomplete DPI update remains | `lamzu.rs:836, 978–995` and `pages/mouse.rs:441`. The app's X/Y toggle changes only the flag, but the official `$2()` also rewrites each stage's X/Y at the toggle, and sets Y=X when OFF. The DPI page's Y field looks only at the model's `xy_split`, not the current flag. Also, one `DpiStages` is 3 commands of value → color → enabled stages, so a later failure leaves the earlier writes on the device. An empty list accesses out of range after `clamp(1,5)`. Validate input first, and a mid-way failure requires a re-read. |
| P2 | Ranges / dependencies that differ from the official | `lamzu.rs:447–534, 807–889`. The LM20 wired and Orbital 8K choices are 500/1000/2000/4000/8000 Hz. 125/250 are out of scope but selectable in the app. The PAW3955's LOD is 0.7/1.0/1.3/1.5/1.7 mm and debounce is 0–15 ms. The app offers extra LODs and debounce 0–32 ms. The frequency codes `2/1/32/64/128` match the **LM20-side** `fM()`. Do not use `uM()`, which is for another manufacturer. In the official app, competitive mode OFF disables 20K FPS and Glass Mode ON stops LOD editing, but the app lets you operate them independently. |
| P2 | Wireless LED cap and on/off handling | `lamzu.rs:433–439, 683–708, 866–883`. The DPI indicator is `A3` even wireless, so the condition "expose it if via Orbital" is wrong. The GET of the receiver LED's individual modes matches, but the master judgment is shown wrongly on the real device. Aurora uses `SetLightness(1,profile,0,255)` for Orbital ON and `SetLightEffect` index `255` / mode `0` for OFF, whereas the app uses `SetLightEffect` of a different length for both ON and OFF. SET was not executed this time. |
| P2 | Macro assignments are shown but their contents cannot be managed | `lamzu.rs:317–324, 386–393, 577–583, 795` and `overlay.rs:560, 901`. Macros are offered as assignment candidates, yet with `macros: None` and always an empty list, editing, transfer, and deletion are unimplemented. The official `MacroI/II/III` distinguish count / until release / until re-press with type `16/17/18`, but the current implementation uses type `16` uniformly and converts the latter two to `254/253`. A representation from another protocol is being reused. Until it can be fully handled, treat it as unsupported and do not overwrite an existing assignment with something different. |
| P2 | Wireless lifetime management / metadata | `lamzu.rs:132–176, 597–620, 1032–1037`. Notifications pick the first IF with the same VID/PID, so with multiple receivers, events of a different unit may be linked. The official connect/disconnect notification type 6 is ignored. Even if the mouse does not respond, it does not become asleep/disconnected. The 4th byte of the FW is shown as a fixed `0`. The receiver FW is not read. If `0.0.x.y` ever appears, it will be shown wrongly. |

## Judgment of features and names

| Official feature / name (Aurora's Japanese labels, in English) | Device HUB's current state | Verdict |
|---|---|---|
| Product name `LAMZU MAYA X V2` / model row `Maya X(LM20)` | `LAMZU Maya X V2`, wired `maya-x-v2` / wireless `lamzu-dongle` | The notation difference is minor. Capitalizing the product name matches the official one. For wireless, the paired device should really be identified and the internal model should match too (preset candidates are searched by `info.model`). |
| `Left click` / `Right click` / `Wheel click` / `Back` / `Forward` | `Left` / `Right` / `Wheel click` / `Side (back)` / `Side (forward)` | The 5 physical slots match. The names can be unified to "Left click" and "Right click". A DPI switch assignment is not an additional physical button. |
| Motion Sync, straight-line correction, ripple | Motion sync, straight-line correction, ripple control | These are separate, correct commands. Only the names differ. |
| `High-speed mode` (when OFF) / `Competitive mode` (when ON), `20K FPS` | `HyperMode`, `Tracking mode (20K FPS)` | For the former, "competitive mode" makes the role easier to grasp. The explanation "high-speed tracking mode" is confusable with the latter. 20K FPS is sensor scanning, separate from 8K USB polling. |
| `Fast trigger` / `Rapid Trigger` | `Rapid fire (left button)` / `Rapid fire (right button)` | **The meaning is different.** The official one is response optimization for left/right clicks, and explains the possibility of unintended double clicks. Auto-fire is a separate button assignment `X_Click`. Also distinguish it from the magnetic switch's press-depth setting. |
| `Debounce time`, `Angle adjustment`, `SensorCenterOffset` | Click debounce, sensor angle, sensor offset | The functions mostly match. The official offset UI is −5 to +5. `mm` cannot be confirmed from the official screen code, so it is not asserted to be a real distance. |
| Wheel direction and `Wheel response time` | `B-hop (wheel misoperation)`, `B-hop time` | Directions 0/2/3 and times 200–1000 ms in 200 ms steps match. No evidence was obtained this time to assert the effect as "suppresses erroneous input". |
| `AdaDPI`: speed-to-multiplier curve, presets, custom | Only ON/OFF of `Adaptive DPI` | Partial support. It cannot be said that "the curve can be edited". On change the old curve is resent, so the number of points and the length of the response must be verified before writing. |
| Sleep 1/5/10/15/20/25 minutes | The same minutes | The candidate minutes match. `0/0xFF00/0xFFFF`, which the official app interprets as OFF, are currently converted to 1 minute or a large number. |
| Button assignment: disabled, click, wheel, key + modifier, media, DPI, cycle, macro, etc. | Basic assignments and some media. No LOD cycle or competitive mode toggle | Partial support. Unsupported official codes are shown as `Disabled`, unknown mouse codes as left click, and unknown media as play/pause, so they may be replaced when saving and restoring settings. |
| Device DPI stage colors / Orbital's 3 receiver lights / independent RGB effect | Stage colors and 3 receiver lights shown, no independent RGB | The way the functions are divided is correct. Stage color SET and LED master SET were not verified this time. There is no basis for adding independent RGB. |
| Profiles 1–4, reset, receiver pairing / FW update | Slots and reset exist. No LAMZU-specific pairing / FW update / update notifications | The slot count and the way it is offered as a general feature are reasonable. It does not mean each slot switches safely on the real device. Distinguish FW writing from the design scope of this app as a whole. |

The existing [protocol analysis §5](PROTOCOL_NOTES.md#5-lamzu-aurora-wwwlamzunet--maya-x-v2-lm20-verified-on-a-real-device) is consistent with this check regarding
the wired 64B feature, PAW3955, DPI values and colors being separate, the p4 caveat, the macro `u16BE slot`, and the shared receiver PID.
The **confirmation of model ID / FW / basic-settings reads via 0048** from this time was also appended to the analysis document. Receiver writes and recovery, changing DPI colors,
enabling Glass Mode, writing to uninitialized slots, macros, and notifications with the current firmware were not tried this time.

`cargo test -p devhub-hid --lib` passed 47 tests, and `cargo check -p device-hub` and `cargo clippy -p devhub-hid --lib` also passed.
The 47 do not include LAMZU-specific unit tests. The existing `lamzuread` writes to the real device midway and has no guarantee of reverting to the original values on failure, so it was not run in the audit.
Neither the product code nor the real device's settings were changed.
