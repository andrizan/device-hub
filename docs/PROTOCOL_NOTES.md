# Protocol Notes — Web configurator analysis results

Facts gathered on 2026-11-14 by opening the two Web versions in Claude's built-in browser and reading the JS bundles.
**Primary source for backend implementation and for adding new devices.** Bundle file names (hashes) change with every deployment, so
if the names listed here are missing, search again with the same procedure (docs/ADD_DEVICE.md §2).

---

## 1. www.mouse.fit (G-Wolves-family mice)

### 1.1 Served files

| URL | Contents |
|---|---|
| `/static/js/index-D8T-BrOM.js` (1.8MB) | **Main bundle.** i18n dictionaries (ja/en/zh), protocol class `cm`, EEPROM address table `Ge`, WebHID calls |
| `/Config/env-models.json` | **Model table.** VID/PID, which features each model has, value ranges. This largely determines how caps are built |
| `/static/js/index-BeTfQRF9.js` and others | Screens (Vue route chunks) |

`sendFeatureReport` / `receiveFeatureReport` / `sendReport` / `oninputreport` appear only in the main bundle.

### 1.2 env-models.json (source of per-model caps)

The first element is app info (`AppTitle: "G-Wolves"`, `AppVer 1.0.0.51`). The second element onward are models. Key list:

```
ModelEN ModelCN XVI PollingRateWired PollingRateWireless UIFolder GoldenDongleNames GoldenDonglePIDs
DefaultDPIColors DefaultSleepTime VIDWired PIDWired VIDWireless PIDWireless VIDWireless4K8K PIDWireless4K8K
LightEnablePID LightDeviceID LightEffects CoreVersion IsNewProtocol IsSensorModelCmd DPIMax LOD
HyperModeEnable CombineKeyEnable DPIXYEnable SleepTimeEnable SleepTimeGrade ButtonDebounceTimeMax
WheelDebounceTimeMax SensorSwitchEnable SPDTStateEnable LongDistanceModeEnable SensorPosEnable
SensorAngleEnable LiftOffDebounce CompetitiveEnable ButtonDebounceAllDiy ButtonDebounceLevel
DPIIndicatorEnable KeyBindingSeq ReceiverKeyBindingSeq ReceiverKeyBindingDeviceID DPIMaxStageNum BtnMaxNum
PollingInfoLevel DeviceLightingOnOff WiredDeviceID MacroEnable _1KDongle _4KDongle _8KDongle _8KDongleIDVD
_1KDonglePollingRate _4KDonglePollingRate _8KDonglePollingRate commonDelay MID ButtonType
```

Excerpt of target models (all VIDs are `33E4`):

| ModelEN | PIDWired | PIDWireless | 4K8K | UIFolder | DPIMax | LOD | Stage | Btn |
|---|---|---|---|---|---|---|---|---|
| **HTS Plus Pro** | 5219 | 3854 | 3854 | JM_052_3955 | 40000 | 0.7;0.9;1.2;1.4;1.6 | 7 | 5 |
| **HTXU (=HTX Ultra)** | 5608 | 5617 | 5617 | GW_056 | 30000 | 0.7;1;2 | 7 | 5 |
| HTXU (3950) | 5618 | 3854 | 3854 | JM_056_3950 | 32000 | 0.7;1;2 | 7 | 5 |
| HTXU (3955) | 5619 | 3854 | 3854 | JM_056_3955 | 40000 | 0.7;0.9;1.2;1.4;1.6 | 7 | 5 |
| HTS Plus | 5408/5418/5419 | … | | | | | 7 | 6 |

Others include HTM Plus / HSK Pro 2.0 / Fenrir family / VUK / HT-S2 / HTR / Lycan / WARG, 36 models in total.
`PollingRateWired = 125;250;500;1000;2000;4000;8000`; wireless depends on the dongle used
(`_1KDonglePollingRate` / `_4KDonglePollingRate` / `_8KDonglePollingRate`).

**ATK F1 Ultimate is not in this table.** It is probably either the target of mouse.fit's "separate driver link (LinkJump)" or ATK's own
configurator. Decide after capturing the real device's VID/PID (docs/ADD_DEVICE.md §7).

### 1.3 Commands, button types and EEPROM addresses

Source: analysis of the communication code of [the mouse.fit configuration tool](https://www.mouse.fit/). For the report-8 mappings confirmed on real devices, see §1.4.

| Operation | Command number |
| --- | ---: |
| Clear settings | 9 |
| Switch profile | 15 |
| Set long-distance mode | 22 |
| Read long-distance mode | 23 |
| Start calibration | 46 |

| Assignment | Type byte |
| --- | ---: |
| Disabled | 0 |
| Mouse button | 1 |
| DPI switch | 2 |
| Rapid fire | 4 |
| Shortcut | 5 |
| Macro | 6 |
| Polling rate switch | 7 |
| DPI lock | 10 |
| Scroll up/down | 11 |

| Setting | EEPROM offset (decimal) |
| --- | ---: |
| Polling rate | 0 |
| Max DPI stages | 2 |
| Current DPI stage | 4 |
| Button action | 8 |
| LOD | 10 |
| DPI value | 12 |
| DPI color | 44 |
| DPI LED effect | 76 |
| DPI LED state | 82 |
| Press debounce | 169 |
| Motion sync | 171 |
| Sleep | 173 |
| Sensor angle | 175 |
| Ripple suppression | 177 |
| Competitive mode state | 181 |
| Performance setting | 183 |
| Angle snapping value | 189 |
| Angle snapping state | 191 |
| 20K FPS | 225 |
| Wheel debounce | 227 |
| Release debounce | 229 |
| Left actuation point | 239 |
| Left Rapid Trigger | 241 |
| Left tactile feedback | 243 |
| Right actuation point | 245 |
| Right Rapid Trigger | 247 |
| Right tactile feedback | 249 |
| Button assignment | 96 |
| Shortcut | 256 |
| Macro | 768 |
| DPI for 3955 | 6912 |
| Enable virtual sensor position | 6984 |
| Virtual sensor position | 6986 |

When sending a macro, XOR 0x55 is applied from offset 12 of the 65-byte buffer onward. Macro names are GBK.

Feature scope of the official tool: FW, battery and receiver info, profiles, DPI and LED, polling rate, sensor and power saving, magnetic buttons and calibration, assignments and macros, pairing, firmware update. For per-feature support status see [BACKEND_BRIEF.md](BACKEND_BRIEF.md).

### 1.4 Verified on real devices — report-8/EEPROM path (JS `Die` class)

**Important: feature reports are unusable on every interface observed** (measured 2026-11-15).
The 8K dongle (33e4:3854) and the **wired HTS Plus Pro (33e4:5219)** both fail:
`HidD_SetFeature`/`GetFeature` fails on every usagePage among ff02/ff04/ff05/ff06 (ff04 shows signs that the report itself
exists (0x57), but every size is rejected). The JS `setReport`/`getReport` family (64B feature
report) should fail the same way in Windows WebHID (swallowed by try/catch in the code).
→ **The only working configuration path is report-8/EEPROM** (responds reliably on ff02/02).
The driver for the new feature-report protocol (`cm` class) is kept in `mousefit.rs`, but
devhub-hid routes every mouse.fit model to `mousefit8.rs`.

- output report ID `8`, 16B command `[op, 0, arg…, cksum]`, the response is a 17B input report ID `8`
  `[id][op echo][status][arg…][data@recv[5..]][cksum]` — **data starts at `r[6]` (=recv[5])**.
- Checksum: `t[15] = 85 - sum(t[0..15]) - reportID` ⇔ the sum of all 17B (including id) ≡ 0x55.
- op: `4`=battery, `18`=FW (main unit) — real device `[18,0,0,0,2,5,48]` → recv[5]=5/recv[6]=48 → **"5.30"**
  (recv[4]=2 is the length field. Data always starts at recv[5]=r[6] — we long referenced r[5] and wrongly displayed "2.05"),
  `29`=FW (dongle side), `8`=EEPROM read, `7`=EEPROM write,
  `14`=getProfileID (recv[5]+1), `15`=setProfileID (`[15,0,0,0,1,id-1]`),
  `22/23`=long-distance mode set/get, `5`=enterPairMode (`[5,0,0,0,2,0,0,11]`),
  `6`=pairResult (recv[5]), `1`=EID/donglePR (put the nonce in [5..8]; recv[10]=EID, recv[11]=PR),
  `9`=ClearSetting, `46`=StartCalibr.
- EEPROM read: `[8,0,addrHi,addrLo,len]` (len ≤ 10). Write: `[7,0,addrHi,addrLo,len,data…]` in 10B chunks.
  Scalar values are 2B pairs of `(v, 0x55-v)` (`Set_Device_Eeprom_Value(addr,v,2)`).
- Battery response: `recv[5]`=percent (measured 85), `recv[6]`=state, `recv[7..8]`=voltage BE (measured 4031mV).
- **ReportRate@0 code table** (verified by OpenMouse @openmouse/protocol gwolves with real-device captures, and also
  confirmed by write-back on this device): ≤1K is the period in ms (8=125, 4=250, 2=500, 1=1000), >1K is bit4/5/6 one-hot
  (**16=2K, 32=4K, 64=8K**). ⚠️ The official JS table (32/64/128) is off by one bit, and the FW rejects 128 as an invalid value
  silently — the write response comes back but the value does not change. Invalid values and a sleeping mouse are ignored in the same way, so
  when going wireless, wake the mouse before writing.
- DPI: `maxDpiStage@2` (count), `CurrentDPI@4` (**0-based**; the JS get returns +1).
  The 3955 family uses a 6B record @`Sensor3955DPI=6912` `[xLo,xHi,yLo,yHi,flags,crc]`, value=(raw+1)*step,
  flags: bit2-3=x upper 2 bits, bit6-7=y upper 2 bits, bit0/4=×2.
  Non-3955 uses a 4B record @`DPIValue=12` `[x,y,flags,crc]` step=50.
  Color is a 4B record `[r,g,b,crc]` @`DPIColor=44` regardless of model.
- Buttons (KeyFunction @96, 4B/slot): `[type, paramHi, paramLo, crc]`, param is
  taken from the ctx of `SetButton` as `(ctx[0]<<8)+ctx[1]` (same layout as the Ya encoding).
  The body of type5 (ShortcutKey) is at @`ShortcutKey=256`+slot*32 (`[count][press×k][release reversed][crc]`,
  crc = `85 - sum(all preceding)`, writes are a compact sequence of `count*3+2` B).
  Macros use `param=(macroIdx<<8)+runMode`, runMode: count n / 254=while held / 253=toggle.
- Macro body @`Macro=768` + slot*384B: name@+0 `[len, GBK name ≤30B]`, ctx@+31 `[count][5B×≤70][crc]`.
  Event 5B: `[flags|type, valLo, valHi, delayHi, delayLo]` — flags upper 2 bits (2=down,1=up),
  type lower 4 bits (1=mouse,2=keydown,3=keyup,9=mod down,10=mod up). delay is BE.
- Magnetic buttons (ButtonType=1): `LeftTrigger@239 / RightTrigger@245` (raw value 0-19,
  **the official UI shows it as 1-20 (+1)**, writes use -1),
  `LeftFastTrigger@241 / RightFastTrigger@247` (**bit7=enabled, lower 7 bits=sensitivity** —
  the official set is `(en<<7)+(val&127)`, get is `byte>>7`/`byte&15`, UI sensitivity 1-10.
  **The value remains even when disabled** (real device: 0x04 = disabled + sensitivity 4). The official UI shows RT
  only when `nButtonType==1 && isWired` — **at the protocol level, even via the dongle
  the same EEPROM write goes through, so this is a UI-side policy** (round trip via the dongle also confirmed on a real device).
  only when `nButtonType==1 && isWired` — **at the protocol level, even via the dongle
  the same EEPROM write path (cmd7 relayed transparently; other addresses confirmed by wireless round trip) goes through,
  so it is a UI-side policy, not a hardware constraint**.
  `LeftTactileFeedback@243 / RightTactileFeedback@249` (likewise bit7=enabled, lower bits=value).
- Sleep @173: JS `setSleepTime` uses `r>=60 ? r/60*6 : r/10` — seconds divided into 10-second units, i.e. **wire = min×6**.
- Sensor angle: write `AngleTuneState@191=1` first, then `AngleTune@189`
  (>0 as is, ≤0 as `255-|v|+1`). On read-back, `v-256` if `>=128`.
- Other scalars (all (v,85-v) pairs): DebounceTime@169, DebounceReleaseTime@229,
  WheelDebounceTime@227, MotionSync@171, Angle@175 (angle snap), Ripple@177,
  SensorFPS20K@225 (competitive/tracking mode), PerformanceState@181/Performance@183 (HyperMode).
- **While the mouse is asleep, no response to any command is returned** (re-confirmed on real device 33E4:3854, 2026-11-15):
  The reality is a **complete timeout**, not "dongle ACK + all zeros". Furthermore the op10 stream
  flows only while active (stops after ~10 s idle), and `04 xx` notifications are not observed even when listening on all vendor IFs
  (silent even on power OFF/ON). → **asleep detection = command timeout + all packets silent for ≥5 s**,
  wake detection = packets resume or a command succeeds (implementation: `last_pkt`/`set_asleep` in `mousefit8.rs`).
  While asleep, `read()` returns early after a single probe with an empty state (avoids timing out for every setting).
- Real-device values (HTS Plus Pro wireless, 2026-11): fw 2.05 / battery 85% 4031mV / polling code 1 (1000Hz) /
  LOD index 1 (0.7mm) / competitive on / ripple on / trigger L=7 R=8 / sleep wire=6 (1 min) /
  DPI stage 1600 #ff0000 / side buttons really assigned keys "2" "1". All write paths ACKed + read-back matched.
- Real-device values (HTX Ultra 3950 wired 33e4:5618, 2026-11): fw 2.05 (matches with op18 `[6]=2,[7]=5` — reading at a different
  offset makes it look like 5.33, which is wrong) / DPI stage 400/1600/1600/3200 / 5 buttons
  (side = keys "2" "1" really assigned) / polling rate 125–8K measured ~5949Hz / LOD@10 is non-index
  type {0.7mm→3, 1.0mm→1, 2.0mm→2} (as in the JS `setLOD` map) / SPDT@`KeyOperation=8`
  bit0=left bit1=right (same as `getSPDTStateNew`/`setSPDTStateNew`). SPDT and polling rate
  UI writes ACKed + read-back matched, LOD round trip confirmed with a probe. Purpose: SPDT is a mode that registers clicks on both contacts
  to reduce latency (not anti-chatter).
- **Identifying the model behind the 3854 dongle (confirmed on real device 2026-11-16)**: when op1 is sent as `[1,0,0,0,8,nonce4B]`
  the response returns the EID in `r[11]` (the official `getEID` `receivedData[10]` = the array index with the report id
  removed = our `r[11]`). The EID is the **MID field** of env-models.json, and
  the official HUB also looks up the model via `a(MID)` (the 3854 is shared by 8 models including HTS Plus Pro / HTXU-3950/3955 /
  HTS Plus / HTX Mini). Measured: with an HTXU-3950 pair, `r[11]=5` (MID=5),
  `r[12]`=donglePR. **0 while asleep** (the official tool falls back 0→2; we treat it as undetermined and
  retry after waking). MID table: 2=HTS Plus(054_3950) 4=Lycan 5=HTXU-3950 6=Lycan-3955
  7=HTXU-3955 8=HTS Plus-3955 9=FenrirPro 10=HTXMini 11=HTS Plus Pro 12=WARG
  13=HTXMiniAsym 15=VUKMini 17=FenrirAsym 21=FenrirMaxAsym.
  The `GetDevicePID` of Die is a stub (always 0x8888) — the PID itself cannot be obtained, only via MID.
- **Unsolicited notification op10 (confirmed on real device 2026-11-15)**: input report `[8][10]…` arrives even if nothing is sent
  (also confirmed via the dongle). `r[6]` is the kind: **0/4/8/16/32=magnetic button depth** (`r[14]&127`=left %,
  `r[15]&127`=right %, measured to track 0→90%+), **1=DPI change**, **2=polling rate change**, **64=battery change**
  → changes on the device side are re-read by the UI via `HubEvent::StateChanged`/`Battery`.
  Actuation point check (JS): `depth/5 >= getMSButtonTrigger()+1` → pressed (in 5% steps).
- feature-report notifications (report id 4): `04 01/04`=DPI, `04 02`=profile, `04 03`=battery,
  `04 06 00/01`=disconnect/connect, `04 07`=LOD, `04 08`=polling rate, `04 0f`=MotionSync, `04 18`=sensor.

### 1.5 Feature list readable from the UI text (i18n ja)

Performance: polling rate (wired/wireless), high-speed mode (HyperMode), competitive mode, long-distance mode, combo-key function.
Sensor: lift-off distance, motion sync, angle snapping, ripple control, sensor angle, sensor change, sensor virtual center point, 20K FPS.
Click: click debounce (4-value DIY for before press/after press/before release/after release, plus low/medium/high presets), wheel debounce,
left/right button SPDT, button misoperation prevention (LiftOffDebounce), scroll speed.
Magnetic micro-switch models: trigger point, quick trigger, vibration strength, click sound, left/right calibration.
Power: sleep time, DPI indicator, LED status, battery display. Light effects: solid/breathing/spectrum/wave + brightness/speed.
Buttons: left/right/middle/back/forward/scroll/X click/double click/DPI cycle, +, −/DPI lock/rapid fire/
keyboard (key code)/Windows shortcut/media (play, stop, next track, volume, mute, calculator, My PC, home, mail, brightness, refresh)/
macro (set count, until released, until pressed again)/polling rate switch/scroll up/down/disabled. Constraint: "at least one button must be the left button".
Macros: up to 3 (depends on the model), recording (record delay/fixed/none), insert, macro name.
Other: settings reset, restore defaults, firmware update (DFU), pairing, receiver button binding.

---

## 2. hub.xsyd.top/keyboard (Sparklink, SU66 Max)

### 2.1 Served files

| File | Contents |
|---|---|
| `/keyboard/js/main-*.js` | App main bundle, **WebHID filters** (`Ur=[{vendorId:7334,productId:…,usagePage:65456,usage:1},…]`) |
| `/keyboard/js/esm-*.js` (260KB) | **Protocol SDK.** One request, one response via `sendReport(0, data)` + `inputreport` |
| `/keyboard/js/tooltip-*.js` | i18n dictionary (ja, 641 entries) |
| `KeyPerformance-*`, `advanced-key-*`, `lighting-*`, `custom-key-*`, `config-*`, `firmware-*`, `screen-*`, `voice-*`, `handle-*` | Screens |

VID `7334 = 0x1CA6`, usagePage `65456 = 0xFFB0`, usage 1. SU66 Max has PID `12297 = 0x3009`.
The filters list more than 100 PIDs under the same VID (other vendors' keyboards using the same SDK can be handled by the same driver).

### 2.2 SDK enumeration values (top of esm-*.js)

**Protocol groups (dt, complete)**: Device=1 Global=2 LayoutAndKey=3 Performance=4
Lighting=5 HigherKey=6 Macro=7 **FirmwareUpgrade=8** CustomCommand=10
Displayer=12 ThreeMode=13 Voice=14 Touch=15 Handle=16 ThreeD=17

```
GFS commands: ResetFactory=1 SaveParam=2 ConfigSwitch=3 SystemSetting=4 ReportRateSetting=5
  CalibrationSetting=6 AxisLibraryQuery=7 EffectAreaQuery=8 ModifyDefaultAxis=9 DoubleLightingQuery=10
  SpecialLightingQuery=11 RtPrecisionQuery=12 SleepTimeQuery=13 MacroSpaceInfoQuery=14
  ShakeOptimizationSwitch=16 USBModeSetting=18
Reset scope: All=0 Calibration=1 Performance=2 Lighting=3 Layout=4 HigherKey=5 Macro=6 Axis=7 HighPollingRateReset=15
OS: WIN=0 MAC=1
Polling rate: R8KHz=0 R4KHz=1 R2KHz=2 R1KHz=3 R500Hz=4 R250Hz=5 R125Hz=6
Config: Config1..4 = 0..3
Light area: Keyboard=0 Decorate1..5=1..5
Dynamic effects: ripple=1 EbbAndFlow=2 RippleGently=3 RotatingStorm=4 LuckyRainbow=5 ShiningRainbow=6
  Shining=7 MovingWindow=8 WaveTransformation=9 Shift=10 SineCurve=11 CloudFlow=12 Blooming=13
  Rainbow=14 Rainfall=15 Jump=16
Light type: Static=0 Dynamic=1 Custom=2   Decorative light: Close=0 OpenDown=1 OpenUp=2 Open=3
USB mode: USB2_0=0 USB3_0_8K=1 USB3_0_16K=2 USB3_0_32K=3   (GetStatus=1 GetStoredMode=2 SetMode=3)
```

### 2.3 SDK methods (= all features; checklist)

```
getKeyLayout/setKeyLayout getKeyCode/setKeyCode getDefaultKeyLayout
getPerformance(row,col) / setPerformance{mode,normalPress,normalRelease,rtFirstTouch,rtPress,rtRelease,
   pressDeadStroke,releaseDeadStroke,axis,row,col,calibrate,axisV2Id,axisRangeMax,axisCoefficient}  ← values are mm×1000 (official order: rangeMax@23,24 / coef@25,26)
getADC getRoute getCalibrate getKeyStatus  (stroke test, calibration)
getLightingBase/Palette/ColorCorrection (+set)  getLightingCustom saveLightingCustom setLightingDirectDrive
  setLightingCaps getDecorate1Custom getLightingArea getDoubleLighting getSpecialLighting getLightingSleepTime/set
getHigherKeyData / setHigherKey{NONE,DKS,MPT,MT,TGL,END,SOCD,RS}  (SOCD/RS take 2 packets.
  The XSYD SDK switch has only these 8 kinds — MDS/TURBO is a mistaken entry derived from walkplay)
setMacroData getMacroData setMacroMode getMacroMode getMacroSpaceInfo
getConfigList getConfigName/setConfigName  (configs 1-4) setImportData (import of a config code)
getRtPrecision getAxisList getCurrentAxis setDefaultAxis/getDefaultAxis
setShakeOptimizationSwitch/get  getUSBModeStatus getUSBModeStored setUSBMode  highPollingRateReset
getKeyboardColor/set getDeviceShellColor/set getKeyboardMode getSystemTypeList setSystemType
getBlackoutStatus (open/close)  getBasicInfo{mode,charge,battery} setSleepTime{shallow,deep} getSleepTime
getDeviceInfo{type,subType,boardId,appVersion,pcbVersion,runModeVersion,sn,timestamp} getDeviceFeature getDeviceProtocol
getAppToBoot getBootToApp getFlash upgrade (firmware update)
displayer: getDisplayerInfo setDisplayerBoot setDisplayerFlash setDisplayerGIF (models with a screen)
voice / haptic / animation / three-mode / handle (gamepad mapping) — model-specific
```

### 2.4 Key code space (from i18n keyTip_<code>)

| Range | Meaning |
|---|---|
| `0x0000` empty / `0x0001` transparent | |
| `0x0004..0x00E7` | HID Keyboard usage as is |
| `0x2000 | consumer` | Media (e.g. `8373=0x20B5` next track, `8418=0x20E2` mute, `8303/8304` brightness ±) |
| `0x4000..0x4Bxx` | Mouse (`16640` left, `16896` right, `17152` middle, `17408` forward, `17664` back, `18945/19201` wheel, `17920..18688` cursor movement) |
| `0x5xxx`, `0xCxxx`, `0xDxxx` | Gamepad (handle mode) |
| `0xF100..0xF103` | Layer switch (main / Fn1..Fn3) |
| `0xF200..0xF209` | System (initialize, Win/Mac layout, magnetic-switch calibration, configs 1-4, Win lock) |
| `0xF300..0xF318` | Lighting control (mode/color/brightness/speed/ON-OFF/direction, decorative light) |

### 2.5 Verified on real devices (from legacy/xsyd.rs)

- 64B frame (in hidapi a leading report-id 0 is prepended, making 65B). `[group][sub][args…]` → `[group][sub][data…]`, `sub==0xFF` is unsupported.
- group: 1=Device 2=Global 3=Layout/Key 4=Performance 5=Lighting 6=Higher key 7=Macro 10=Custom 12=Displayer 13=Three-mode.
- `1,2` device info (ASCII `49142`, build `2026101216:41:03`), `4,1` performance, `5,1` lighting respond.
- **There are 2 layers.** Key position is `(row<<4)|col` (row 1..5, col 0..14), and `row*30+col` in the key code stream.

### 2.7 Confirmed frames (dshk.xsyd.top SDK + SU66 Max real-device verification 2026-11-15)

All u16 are **little-endian**. mm values are **u16 ×1000**.

**Device (group 1)**: `1,1` Protocol → `r[2],r[3]` (real device 1.3). `1,2` info → `r[2]`=type
(0=magnetic wired,1=magnetic 3-mode,2=magnetic+screen,3=mechanical wired,4=mechanical 3-mode,5=mechanical+screen,6=optical wired,7=optical 3-mode,
8=optical+screen,9=inductive wired,10=inductive 3-mode,11=inductive+screen,12=magnetic full screen,13=magnetic haptic,14=magnetic voice,
15=magnetic haptic+voice,16=magnetic gamepad), `r[3]`=subType, `r[4..8]`=boardId BE, `r[8..12]`=appVersion.
`1,3` DeviceFeature → **feature bitmask**: `r[2]`=axis {mechanical:1,magnetic:2,optical:4,
inductive:8,magnetic3D:16}, `r[3]`=connection {usb:1,2.4G:2,ble:4}, `r[4]`=basic {rgb:1,knob:2},
`r[5]`=extended {smallScreen:1,fullScreen:2,haptic:4,voicePlay:8,voiceRec:16,gamepad:32,dotMatrix:64}.
SU66 real device = magnetic / USB only / RGB / no extensions. **The old implementation that read `r[2]` as the layer count is wrong
(on the SU66, magnetic=2 and layer count=2 merely coincided)**.

**Layout/Key (group 3)**:
- `[3,1,layer,row]` → `r[4..64]` = 30×u16 (key-stream row)
- `[3,2,layer,row,…30×u16]` row write / `[3,4,layer,row,col,codeLE]` single-key write (**verified on real device**)
- `[3,3,layer,row,col]` → `r[5,6]`=keycode

**Performance (group 4)** — **verified by read/write on real device**:
- `[4,1,row,col]` → `r[4]`=mode (**0=normal/1=RT continuous/2=IntervalRT non-continuous** — official enum
  `vn={Normal:0,RT:1,IntervalRT:2}`. The official UI's "continuous RT" toggle switches 1↔2),
  `r[5,6]`=normalPress, `r[7,8]`=normalRelease,
  `r[9,10]`=rtFirstTouch, `r[11,12]`=rtPress, `r[13,14]`=rtRelease, `r[15,16]`=pressDeadStroke (upper DZ),
  `r[17,18]`=releaseDeadStroke (lower DZ), `r[19]`=axis, `r[20]`=calibrate, `r[21,22]`=axisV2Id,
  `r[23,24]`=axisRangeMax (full stroke, mm×1000), `r[25,26]`=axisCoefficient
  (**Official decoding confirmed: rangeMax comes first, coefficient after. The old implementation misread the coefficient 25.7 as the full stroke**)
- `[4,2,row,col,…]` writes with the same layout → then `[2,2,2]` (SaveParam:Performance)
- `[4,3,kind,row]` → 30×u16 (in col order, row=1-5). kind (sub-kind inside AxisData, confirmed from the SDK enum):
  0=ADC (raw sensor value ~2200-2500), 1=Route (mm×1000, **used by live_travel**. 0 when not pressed —
  the correct baseline after calibration), 2=Calibrate (calibrated when per-key status=2), 3=KeyStatus
  (all 0 when not pressed, non-zero while pressed — used for the press display of the OBS overlay).
  **Note: kind comes first, row second**. `[4,3,row,kind]` is interpreted as a different query and returns wrong data (confirmed on a real device).
- **Axis data is an autonomous stream (confirmed on a real device 2026-11-15)**: a device that has received a live axis query
  **continuously delivers `[4,3,kind,row]` frames on its own** (cycles through all 20 combinations in ~16ms, measured ~1270 pkt/s).
  "Responses to a query" cannot be told apart, so the receiver must **keep reading until it gets a
  frame whose echo r[2]=kind/r[3]=row matches**. If the first group-4 packet is taken as the response, data of another row/another kind
  is picked up and the values drift (real harm seen: ADC values leaked into the KeyStatus field → the press display flickered).
- **Arming/disarming the stream (2026-11-16 real-device probes `dupread`/`disarm`/`streamlife`/`disarmcmd`)**:
  - A single `[4,3]` starts autonomous delivery at ~28fps, and it **does not end by itself for 150 seconds or more**
    (it continues even if the armed handle is closed — device-side state).
  - **Any non-axis command disarms it immediately** (`[4,1]`/`[2,3]`/`[4,4]`/`[0,0]` all work.
    Only `[4,3]` re-arms. `[1,2]` device-info slows it down but does not stop it).
  - **Input reports are fanned out to all open handles** (responses to another client are visible).
    → If the official HUB (sendAndWait treats the first frame as the response, no echo check) and
    the OBS plugin are used together, an armed stream corrupts the official full read.
    `live_axis` was made safe to use alongside by detecting external traffic (other parties' command responses) and going passive
    (`drain_axis`/`note_foreign`/`cmd_self` in `xsyd.rs`).

**Global (group 2)** — the first argument is mostly an op (0=list/start, 1=read/end, 2=write):
- `[2,1,scope]` factory reset / `[2,2,scope]` save to flash (scope: 0=all,1=calib,2=perf,3=light,4=layout,5=higher,6=macro,7=axis)
- `[2,3,0]` number of configs / `[2,3,1]` current config / `[2,3,2,i]` switch / `[2,3,3,i]` get name (32B UTF-8) / `[2,3,4,i,…]` set name
- `[2,4,1]` get OS / `[2,4,2,0|1]` set Win/Mac
- `[2,5,0]` polling rate choices / `[2,5,1]` current / `[2,5,2,code]` set (code: 0=8K,1=4K,2=2K,3=1K,4=500,5=250,6=125)
- `[2,6,0|1]` start/end calibration
- `[2,7,0]` axis list (u16 id) / `[2,7,1]` current axis / `[2,9,0|1]` default axis set/get
- `[2,8,0]` light areas (`r[3]`=count, each {index,count,rows,cols}) / `[2,10,0]` double lighting → `r[3]` /
  `[2,11,0]` special lighting → `r[3]`. **On the SU66 real device both are `r[3]=1` (existence confirmed by probe8)** —
  sending arg=1 returns `r[2]=0xff`, unsupported. Always use arg=0 for queries
- `[2,12,0]` RT precision → `r[3]`/1000 = minimum step in mm
- `[2,13,0,u16]` / `[2,13,1]` light sleep time (minutes)
- `[2,14,0]` macro area → `r[3]`=slot count, `r[4,5]`=max event count
- `[2,16,0,v]` / `[2,16,1]` anti-chatter optimization
- `[2,18,*]` USB mode — **the SU66 real device responds 0xFF (unsupported)**. Not present on wired-only models

**Lighting (group 5)** — verified by real-device read/write (checked against the official `lightingBaseData`/`setLightingBase`,
E2E `lite2e` confirmed write→readback→restore 11-16):
- `[5,1,area,0]` → `r[4]`=open, `r[5]`=mode, `r[6]`=brightness 0-100, `r[7]`=speed 0-100,
  `r[8]`=direction (**only 0=Forward/1=Backward**), `r[9]`=palette selected color index
- **The mode byte is an index into the `lightingEffectModes` array** (the official `applyBase{mode:i}` sends
  `i` as is). The SU66 has **20 modes (L1-L20 in the UI; modes 20-23 are clamped by the
  firmware to 19 on write — confirmed by real-device readback)**.
  **Mapping confirmed by real-device camera verification (2026-11)**: the SDK enum `Y1` is 1..16
  (ripple,EbbAndFlow,RippleGently,RotatingStorm,LuckyRainbow,ShiningRainbow,Shining,
  MovingWindow,WaveTransformation,Shift,SineCurve,CloudFlow,Blooming,Rainbow,Rainfall,
  Jump) but the real device's wire value corresponds to the one **before** this order:
  - **0=solid color** — displays the selected palette color across the whole keyboard. **sel=0 is the firmware's
    multicolor (horizontal rainbow gradient)** and the stored RGB of slot0 is ignored
    (in the official UI too, index0 is the "more" rainbow icon). sel1-7 display the slot color as a solid
  - **1..15 = dynamic effects** (corresponding to Y1[2..16]): 1=Rainbow side flow (rainbow scrolling left and right),
    2=Slow color change (uniform color + slow hue rotation), 3=Fast color change (fast hue rotation),
    4=Rainbow checkerboard (rainbow checker), 5=Rainbow cycle (hue-cycling gradient),
    6=Rainbow blink (hue cycling + blinking), 7=Faint window (moving light band),
    8=Gentle morph (uniform color slowly morphs), 9=Color band shift (color zones move),
    10=Rainbow swell (multi-band rainbow wave), 11=Diagonal light band (diagonal band sweep),
    12=Confetti (scattered rainbow patches), 13=Rainbow mosaic (fine rainbow checker),
    14=Light rain (colored dots fall), 15=Bouncing light (lit blocks hop and move)
  - **16=custom** — only cells painted with isCustom=255 light up, the rest are off
    (display mode of the official "Decorate" paint area; same meaning as J1.Custom=2)
  - **17-19=key press reactions** (the rest of L17-L20): 17=lights a ~4×4 block around the pressed key,
    18=lights the whole row of the pressed key, 19=lights a wide area around the pressed key. Everything is off when no key is pressed
  - All modes use the selected palette color (confirmed: sel=1=red turned every mode red).
    Brightness works in all modes including solid color. The direction byte is not exposed in the official UI, but
    is thought to reverse the sweep direction in wave-type modes (a definite reversal has not been confirmed on a real device)
  - The category enum `J1={Static:0,Dynamic:1,Custom:2}` is UI-internal only and does not appear on the wire
- **The open byte is a 4-value enum `jo`** (`{Close:0, OpenDown:1, OpenUp:2, Open:3}`):
  on a DoubleLighting model's Keyboard area it indicates whether the upper or lower part is lit. **DoubleLighting is not
  fixed; it is queried with `[2,10,0]` (GFSDoubleLightingQuery)** — true when `r[3]==1`.
  The official tool sends `open=3` (Open = both zones) when lighting on. **Writing 1 lights only the lower zone**, so
  sending the boolean as is is wrong. SingleLighting models treat it as a boolean, non-0=Open (official parser:
  `lamp==="DoubleLighting" && area===0 ? jo[r[4]] : r[4]===0?"Close":"Open"`).
  On the model side, `LightArea.zones=[upper,lower]` + `LightingState.zone_on` map to the UI's
  upper/lower light switches, and are combined into jo on write (both=3, upper only=2, lower only=1)
- `[5,2,area,0,open,mode,brightness,speed,direction,sel]` write (same order as the response layout) → then `[2,2,3]`.
  Not sent if it equals the current value (in E2E, verified with write → wait 80ms → readback —
  until the flash write of save takes effect, the device may return old values)
- The color for solid colors / colored effects goes through the **palette**: `[5,1,area,1]` → `r[4..]`=`{B,G,R,H}`×8 colors,
  `[5,2,area,1,…32B]` write (read-modify-write on the selected slot). The model side represents it with
  `LightingState.palette` (all 8 slots) + `palette_sel` + `color` (= color of the selected slot) —
  selecting a slot only writes sel, editing a color rewrites the selected slot (the same separation as the official
  `selectStaticColor`/`setStaticColor`)
- Custom display is **mode=16** — only cells with isCustom=255 light up in their painted color, other cells are off
  (confirmed by real-device camera: only the cells painted row by row lit up, clearing turned everything off).
  isCustom=255 is also an overlay on top of the base effect in other modes
  (observed on a real device: effect=0, solid color black + 10 custom cells). The official parser treats only `isCustom===255`
  as valid
- Custom colors (sub numbers confirmed from the SDK enum): `ReadBack=3` / `Custom=4` / `DirectDrive=5` / `Caps=6`
- `[5,3,area,offset]` → `r[4..]` = 15 cells×4B `{B,G,R,flag}`. area0 is 9 packets=135 cells
  - **BGR is always "the color being drawn right now"** (the current frame of the effect). flag is a **marker**:
    `0xFF`=user/DD custom, `0xF0`=cell written/confirmed by the firmware, `0`=unmarked.
    **Do not use flag to decide whether a cell is lit** — in a session with no writes all cells have flag=0
    yet colors flow normally (confirmed on a real device with litpoll 11-16. "0=unlit cell" was wrong).
    While lit, the check is `flag!=0 || BGR!=0` (unlit cells have BGR all 0 too).
    The official HUB's lighting preview works by displaying this read for every cell as is —
    it periodically polls `getKeyCustomLighting` (= reads all `[5,3]` chunks)
    (not a JS-side simulation). This app is likewise implemented with `live_lighting`.
    Only the `isCustom===255` check still uses flag (custom cells are held with flag=255).
- `[5,4,area,offset,…60B]` custom write (persistent). `[5,5,area,offset,…60B]` DirectDrive
  (no response, live display only).
  **`per_key` on the model side is an absolute state** — unspecified cells are overwritten with isCustom=0
  (they cannot be erased if left via read-modify-write). Color bytes keep their existing values and only the flag is dropped.
  `write_per_key` sends only changed chunks (verified in E2E `lite2e` with write→clear→restore 11-16)
- `[5,6,area]` caps → real device responds with all 0
- **Saturation (ColorCorrection, Za=2)**: `[5,1,area,2]` → `r[4,5,6]` = R,G,B,
  `[5,2,area,2,R,G,B]` write. The official UI shows "Saturation" as R/G/B **three 0-100 sliders**.
  SU66 real-device values: area0=(38,35,40), area1=(14,13,15). The model side uses `LightingState.color_correction`
  (verified with the cc round trip in E2E `lite2e` 11-16)
- **Lighting link**: the link/link-unlink icon in the official UI is not a device setting but
  UI-local — when ON, the upper and lower switches move together. We implement it the same way

**Cell-to-key mapping for custom colors (confirmed by visual inspection on a real device 11-15, verified with `example litest`)**
- LED grid = **6 rows × 21 columns** (matches the SDK default `rows:6, cols:21`). Physical rows 1-5 are grid rows 1-5,
  grid row 0 (cell 0-20) and 126-134 are unused
- Rows 1-4: `cell = row*21 + floor(visual x)` matches all observations (Esc=21, 9=30, W=44, '=74, Enter=75,
  LShift=84, V=89, B=90)
- Row 5 is compressed on the right side, so use the measured table (`xsyd.rs::led_cells`):
  LCtrl=105, LWin=106, LAlt=107, **under the space bar=109-113 (5 LEDs under one keycap)**,
  RAlt=114, Fn=115, ←=117, ↓=118, →=119. **Dead cells: 108, 116, 120-125**

**Higher key (group 6)**: `[6,1,row,col,0]` read → `r[4]`=mode (0=NONE,1=DKS,2=MPT,3=MT,4=TGL,5=END,6=SOCD,7=RS).
`[6,2,row,col,mode,…]` write. Offsets are positions in the response r (the write packet corresponds to r[2..], so subtract 2).
- DKS: 4×kc u16@5..12 + 4×trp u8@13..16 + db u16@17 + db2 u16@19 (db=min stroke, db2=max stroke).
  **trp is an 8-bit mask**: b0=press-start Tap b1=press-start Hold b2=bottom-out Tap b3|b4=bottom-out Hold b5=return Tap b6=return Hold b7=all-release Tap
- MPT: 3×kc@5..10 + 3×db u16@11..16 (mm×1000) / MT: kc@5..8 + time u16@9..10
- TGL: kc@5..6 + **time u16@7..8** (press time in ms of the toggle output)
- END: **kcs@5..8 = [key on press, key on release]** + delay u16@9..10. A position field like `point` does not exist on the wire
- SOCD: **2 packets** `[6,2,r1,c1,6,r2,c2,kcA,kcB,delay@11..12,x0]` + `[6,2,r2,c2,6,r1,c1,kcB,kcA,delay,x1]` (pair, keycodes swapped).
  socdMode: 0=last wins (x0=0,x1=0) 1=A priority (1,2) 2=B priority (2,1) 3=neutral (3,3). **delay u16@11..12 is the delay of the reversal decision**
- RS: same structure as SOCD (no trailing mode byte), also delay u16@11..12
- The model-side `Advanced::{Tgl,Socd,Rs,End}::delay_ms` corresponds to these time/delay fields (verified by real-device round trip 2026-11-15)

**Macro (group 7)**: `[7,1,id]` → `r[3]`=valid, `r[4,5]`=actNum, `r[6,7]`=repNum, `r[8]`=mode.
`[7,2,id,valid,actNumLE,repNumLE,mode]` write.
`[7,3,id,offset]` → `r[4..]` = 4B actions × 15/packet. `[7,4,id,offset,…]` write.
Action u32 LE: **bit31=status (1=keydown/0=keyup), bits16-30=delay_ms (≤32767), bits0-15=keycode**.
mode: 0..3=run on click (count), 4=repeat while pressed (stop immediately), 5=repeat while pressed (stop after completion).

**FirmwareUpgrade (group 8)** — official OTA (both online and local .bin exist):
- `[8,1]` AppToBoot: switch from app mode to the bootloader
- `[8,2,addr u32 LE,len u16,data...]` Flash: write firmware data by address
- `[8,3]` ValiDate: write verification packet
- `[8,4]` BootToApp: bootloader → return to the app (the device restarts on success)
- **Verified on a real device (grpsweep/bootapp, 2026-11-16)**: `[8,1]` moves the device into the bootloader.
  Response signature while in boot: only `[1,1]`/`[1,2]` ECHO, `[1,3]` is UNSUP,
  function commands of groups 2-13 all time out, `[8,0]`/`[8,10]` are UNSUP.
  `[8,4]` returns to the app and after ~4.5s re-enumeration all commands recover (no replug needed).
- Distribution API is `api.sparklinkplayjoy.com/api/v1/getKeyboardFirmware` (board_id +
  appVersion → latest version URL). **We only notify and do not implement writing**

**CustomCommand (group 10)** — passthrough for other features:
- Blackout: status `[10,1,0]` → `r[3]` (255=enabled/0=disabled),
  open `[10,1,1,255]` / close `[10,1,1,0]` (di enum {Mid:1, Open:255, Close:0}).
  The SU66 real device responds (r[3]=0) — a mode that temporarily turns off all LEDs.
  Implemented as the action `blackout` (reads the state and toggles). Real-device round trip confirmed

**Settings export/import**: the official tool downloads `exportConfiguration` →
`encryptData(settings JSON)` as `{data:...}`; import is done by
`setImportData` writing back with each set* command. We use our own JSON via the `export_config` /
`import_config` actions (`{app,model,name,state:DeviceState}`,
serde as is) — **incompatible with the official encrypted format** (decryption key not analyzed)

**ThreeMode (group 13)** — tri-mode models only (on the SU66 wired model, 0xFF unsupported was measured):
- `[13,1]` BasicInfo → `r[2]`=mode (USB=0/2.4G=1/BT1=2..), `r[3]`=charge, `r[4]`=battery
- `[13,2,shallow u32,deep u32]` SetSleepTime / `[13,3]` GetSleepTime → `r[2..10]` same structure

**SpecialLightingQuery `[2,11,0]`** → maps `r[3]` to `[1,3,5]` (value of the special lighting feature,
on the SU66 real device 1→3). Informational only, unused in the UI

**Calibration enum Xo**: `{Open:0, Close:1}` — `GFSCalibration("Open")` → `[2,6,0]`,
`("Close")` → `[2,6,1]` (matches our calibrate/calibrate_finish)

**Scope (GFSRestore/SaveParam)**: All=0 Calibration=1 Performance=2 Lighting=3
Layout=4 HigherKey=5 Macro=6 Axis=7 (HighPollingRateReset=15 also exists in the enum)
**Advanced-key and macro writes are verified on a real device** (TGL write/read/clear, macro write/read/delete, 11-15 hktest).

**Key codes**: basically u16 LE. Combination key = `0x1000 | mods<<8 | usage` (confirmed on the custom-key screen,
mods are HID modifier bits bit0-3=left Ctrl/Shift/Alt/Win).
**Special key code space (fully confirmed from the J_ catalog in the tooltip bundle 2026-11-16)**:
- 0=unassigned, 1=transparent, 61696-61699=layer switch (main/Fn1-3)
- 61952=initialize (long press), 61953/61954=Win/Mac layout switch (long press), 61955=magnetic-switch calibration,
  61957-61960=config 1-4 switch, 61961=Win key lock
- 62208-62216=main light control, 62224-62232/62240-62248/62256-62264=decoration areas 1-3 control
- 62466-62469,62471=ThreeMode switch
- **62720-62735 (0xF500-0xF50F)=macro slots 0-15 → implemented**
- 16384+256n=mouse actions, for the media/system/xboxHandle/classHandle categories see J_
- Mouse movement: 17920=↑ move 18176=→ move 18432=↓ move 18688=← move 18945/19201=wheel up/down

**Three-mode (group 13)**: `[13,1]` → `{mode,charge,battery}` (reported as unsupported on the SU66 wired model).
`[13,2,…]` sleep (shallow/deep, each u32 LE), `[13,3]` get — wireless models only.

### 2.6 Feature list readable from the UI

Key mapping (basic/mouse/media/lighting/control/system switch/combo/macro, main + Fn layers),
Performance (trigger mode: normal/RT/first trigger, trigger travel press/release, RT press/release sensitivity,
upper/lower dead zones, continuous RT, axis (switch) selection, RT precision), calibration, stroke test,
advanced keys (DKS, MPT, MT, TGL, END, SOCD (last wins/A priority/B priority/neutral), RS),
macros (run on click with a count, repeat while held (stop immediately/stop after completion), start/stop on click, up to 32768ms),
lighting (main + decoration, 16 effects + solid color + custom, brightness, speed, direction, color, auto off, link),
configs 1-4 (switch, name, import/export), polling rate, USB mode, anti-chatter optimization, compatibility mode,
Win/Mac, sleep (wireless models), firmware update, factory reset, screen GIF, voice and haptics (supported models only).

## 3. WalkPlay / peq.szwalkplay.com — TP55 (USB DAC)

Bundle analysis of `peq.szwalkplay.com` + TP55 real-device probe (3302:4383, consumer control IF
up=0x000C u=01) confirmed it. Class taxonomy is the CB family (KT02H20/KT0231H use a different [sub,0,0,0,'R'|'W'] 10B frame,
KT1213 uses report 87).

### Transport

- **report 0x4B** (64B for both IN/OUT). A wakeup report 0x54 also exists but is unnecessary in normal mode.
- Read: `[0x80, sub, params...]` → response `[0x4B, 0x80, sub, …]` (the official dispatcher
  branches on `e[1]`=sub, with the value in `e[3..]`).
- Write: `[0x01, sub, params...]` (no response). Save `[1,1,0]` (refereshToFlash).
- **⚠ `[1,255]` = switch to bootloader** (0666:0888 "USB20" rescue (brick recovery) mode. Can only be recovered by replugging).
  Triggered by mistake twice already — never fire off writes casually.

### Subcommand table (response confirmed on a real TP55)

| cmd | Meaning | read value position | write |
|---|---|---|---|
| `[80,0c]` | fw version | ASCII | — |
| `[80,09,0,0,band]` | EQ band read (**the band selector is the 5th byte**; `[80,9,x]` always gives band0) | `[4B,80,09]` + param echo `[0,0,band]` + write frame v[5..35] (raw [8..27]=coefficients, [28..29]=freq, [30..31]=q, [32..33]=gain, [34]=type, [36]=tag) | — |
| `[80,02]` | Mic offset | `e[3..4]` int16 **LE**, UI value = round(raw×15/32767), range -15..15 | `[1,2,2,lo,hi]` (raw = v×32767/15) |
| `[80,03,2,0,0]` | **EQ volume** (Volume slider. `readDacOffset`/`setDacOffset`) | `e[4]` raw, UI value = raw-16, range -16..+6 | `[1,3,2,0,raw]` (raw = v+16) |
| `[80,11]` | DAC filter | `e[3]`-1 → 0..4 | `[1,17,1,idx+1]` |
| `[80,16,2,ch,0]` | Channel balance | `e[5]` (0 or 256-v), ch0/ch1 independent | **2 frames**: `[1,22,4,att side,0,-v,0]` + `[1,22,4,opposite side,0,0,0]` (the attenuation of the side not being written remains, so always clear it to 0) |
| `[80,19]` | Output gain | `e[3]` 0=Low/1=High | `[1,25,1,v]` |
| `[80,1d]` | DAC work mode | `e[3]` 0=Class-H/1=Class-AB | `[1,29,1,v]` |
| `[80,0e]` | 4B (serial/features?) | — | — |
| `[80,18]`/`[80,1e]`/`[80,1a]` | meaning unconfirmed | — | — |
| `[80,0d,0,addr32]` | **Code memory 32B read** (for firmware dumps. Settings are not here) | | |

Unsupported on the TP55 (timeout): ENC `[80,1b]`, filter bank `[80,25,1,n]`, `[80,22,24,26,d3,da]`.

### TP55 official UI (confirmed with real-device screenshots)

- **EQ Effect**: 10-band PEQ (freq/Q/type/gain) + Volume (-16..+6dB).
  Presets DEF/CUSTOM/ONLINE/SHARE. The TP55 is the **CB1300D (s2) class** in the bundle
  (reportId 75, `getBandCount()=10`).
  - **Band frame 36B** (common layout for read and write):
    `[1,9,24, 0,band, 0,0, <5×i32 LE coefficients>, freq u16LE, q×256 u16LE, gain×256 i16LE, type, offset, tag]`
    - Coefficients are a biquad of `[b0,b1,b2,-a1,-a2]`. **Computed at 96kHz** (confirmed by a byte match with the real-device response).
      PK/LS/LP/HP use **Q30**; only HS (the official X1 class) uses **Q27** + ±(2^27-1) clamp +
      gain×0.85 headroom + Q 0.5..5 clamp. Denominator coefficients are stored with inverted sign.
    - `type`: **LS=1, PK=2, HS=3, LP=4, HP=5** (getFilterIndexs)
    - `offset` (v[34]): a scalar field in the official tool, always observed as 0
    - `tag` (v[35]): echoes e[35] of the read response as is (0x65 on the real device, the
      hardcoded value of the DEF preset is 1)
  - **Read**: `[80,9,0,0,band]` (confirmed on a real device: `[80,9,x]` returns band0 for any x.
    The 5th byte is the band selector). The response is almost the same as the write frame, but the param echo
    `[0,0,band]` adds 3 extra bytes, so the meta positions are **+1** (freq is raw [28..29]).
  - **Apply**: send all band frames → `[1,10,4,0,0,255,255]` (refereshToMemery, takes effect immediately)
  - **Save**: then ~1 second later `[1,1,0]` (refereshToFlash). Same order as the official saveEqInfo.
  - ⚠ **Real-device note**: after the EQ apply sequence, `output_gain` (cmd 25) was observed reverting
    to the flash value (unsaved values of the live registers get discarded). The official tool uses
    the same sequence, so it is judged to be a hardware specification. The driver does nothing special.
  - **DEF preset** (hardcoded in the bundle): 8 bands
    `freq 50/200/500/1000/2500/5000/10000/15000, PK, Q 0.75, 0dB`. Bands 8/9 are not touched.
  - The coefficient implementation is in `devhub-core/src/peq.rs` (shared by the UI graph and wire generation).
- **MIC**: one mic offset (range -15..15), listen, AI ENC (the TP55 says "nonsupport").
  The L/R bars are a VU meter of the browser mic and not a setting (there is no L/R gain).
- **DAC Filter**: DAC work mode (Class-H/Class-AB), output gain (Low/High),
  channel balance, 5 digital filters (FAST-LL/FAST-PC/SLOW-LL/SLOW-PC/NON-OS).
- There is a Firmware Update button (a rescue/update flow separate from the group system).

### TP55 real-device E2E (write → read-back round trip)

Round trip confirmed for all 6 settings: dac_filter / dac_work_mode / output_gain / channel_balance (±30/0) /
eq_volume (-3/+2) / mic_offset (±5). The real device has a quirk where a read right after a write is occasionally dropped once
→ the driver's `cmd` resends exactly once after 40ms on timeout.

Implementation: `devhub-hid/src/walkplay.rs`. 6 fixture tests (scalar read/write, EQ read decoding,
36B write wire, PeqAll sequence, byte match of coefficients with the real device).
The channel balance polarity follows the official dispatcher: **ch0 attenuation = positive, ch1 attenuation = negative**
(positive = the left is attenuated and the sound shifts to the right).

### 2.8 Undocumented commands found by scanning (probe3/probe4, SU66 real device 2026-11-16)

Scanned all groups × subcommands. Response rules: `[grp,0xff,…]` = unsupported, timeout = write-only/no response,
`[grp,sub,…]` echo = exists. sub 0 echoes in all groups (generic ping).

**Exist, unused by the official UI / not implemented by us:**
- `[3,3,layer,row,col]` GetKeyCode → read a single key (Esc=0x29 confirmed on the real device)
- `[3,5,page]` GetKeyLayoutStyle → **page-split physical layout description** (confirmed by probe7):
  up to ~15 **u16LE cells** from r[3]. Each u16 = `{row:5bit,col:7bit,ratio:4bit}`,
  row/col are in quarter units (÷4), ratio is a raw value (12=knob, 13-15=screen).
  Real data in pages 1-5 (row0 is empty). The official tool draws the keymap canvas with this (we hardcode the layout)
- `[3,6,(system<<4)|fn,row]` GetKeyLayoutDefault → **factory keymap** (4 fn × 2 OS = 8 sets,
  30×u16 per row). Real device: fn0/win row1 = Esc,1..9 (0x29,0x1e-0x26). Usable for "restore factory layout"
  → **`reset_keymap` / `KeyMapReset` have been changed to read this via factory_stream and write it back**
  (previously they overwrote the Fn layer with all-transparent, destroying the factory Fn1 number row → F1-F12)
  The echo format is `[3,6,sel,row]` → `r[2]=sel, r[3]=row` (confirmed by probe8, checked by factory_stream).
  The fn1 factory value row2 has 0xf3xx-type media codes (F7,F0,F1,F17,F10). Sometimes the same row has the same value on sys0/sys1
- `[5,5,area,chunk,…15×(B,G,R,flag)]` **DirectDrive** — per-LED direct drive with no response
  (SDK sendDataNoResponse = just writes the report). The official SDK exposes it but the UI does not use it.
  Confirmed on the real device/OBS camera (2026-11-16):
  - `flag=255` = sets isCustom and pins the cell. **The last color remains even after the stream stops** —
    there is no firmware-side DD timeout/keepalive. Explicit release is required.
  - `flag=0` = **do not touch this cell** (neither released nor turned off). Cells not included in DD / with flag=0
    are driven by the firmware animation as is → **hybrid is possible** (7 red DD cells +
    the firmware's light-blue effect in the background displayed at the same time).
  - Releasing a cell is done by setting isCustom=0 with a `[5,4]` custom write. `direct_drive` tracks pins/colors,
    for cells that dropped out it reads back, preserves the other cells, and releases with `[5,4]`; chunks with only unchanged cells/flag=0 are not resent.
  - `direct_drive_stop` = `[5,4]` release all + `[5,2]` resend → full return to firmware confirmed on the real device.
  **Implemented**: Lighting page "Preview on device" + Live hybrid → the real device LEDs sync.
- `[5,6,B,G,R]` Caps = setLightingCaps — write accepted (echo). The read `[5,6,arg]` only
  echoes r[3]=arg and returns no value (write-only). Presumed to be the CapsLock indicator color (not visually confirmed)
- `[2,15,*]` — a gap in the enum (Wa: …MacroSpaceInfoQuery=14, ShakeOptimizationSwitch=16) but
  `[2,15,0]` responds with ECHO (all 0) — the handler itself remains (measured 2026-11-16)
- `[9,1]`/`[9,2]` (Touch/haptic = dt.Pi): GetHapticParams/GetHapticListAndPosition respond but
  DeviceFeature ext=0 → no hardware. The remaining subs are 0xff
- `[10,3]`-`[10,11]` (Custom): handler present, only echoes arguments, data all 0 — an OEM extension slot, meaningless
  (2026-11-16 re-sweep: subs 0,3,5,7,9,11 ECHO, 13,15 are UNSUP — only odd subs are implemented)
- `[2,17,0]` ECHOs (all 0) and does not detach (the detach in the "Dangerous" note below is under a different condition —
  still never send arg>0)
- `[5,1]` area enumeration: the SU66 has only area 0 (keys)/1 (decoration), area2 onward are UNSUP (confirmed by measurement)
- groups 0/11/12/14-20 are UNSUP or time out on the SU66 (grp12 Displayer has no handler since
  ext=0 even with axisType=2 — as in the SDK's feature table)
- Custom resource enum (`mr`): {?=2:ImageResources, UpgradeFlash=3, GIF=4, bootAnimation=5}
  — asset upload command group for models with a screen
- **grp14 Voice (speaker-equipped models only, not applicable to the SU66)** — decoded from the SDK:
  `[14,10,ctl,index,…60B]` = playVoiceRealTime real-time voice stream.
  ctl enum: Stop=0 Start=1 TransmitDirect=2 (raw PCM) TransmitADPCM=3 RestartDirect=4 RestartADPCM=5.
  Others: 1=GetVoiceParams 3=StartVoiceUpdate 4=SendVoiceFile 5=StopVoiceUpdate 6=ReadVoiceInfo
  7-9=ReadVoiceFile family 11=PlayVoice 12=VoiceSpaceManagement 13=WriteKeyVoice 14=ReadKeyVoice
  15=ReadKeyVoiceByPosition — the key voice assignment feature. Error enum: 249=TidySpace
  250=InsufficientSpace 251=CommandRejected 252=ChecksumError 253=ParameterError
  254=IndexNotFound 255=PeripheralFault
- The grp12 Displayer enum names share `Jr`, but what the SDK actually uses is the Voice stream side
  ("Displayer" is a separate group of screen-related functions such as getDisplayerInfo — grp12 is boot/GIF/
  flash asset management for models with a screen). Even with the SU66's axisType=2 (WithScreen), DeviceFeature ext=0 means no screen features

**The layer count is fixed at 4 (confirmed by probe5, 2026-11-16):**
- The SDK reads `fn0..fn3` as a **fixed 4 layers**. Real device: layers 0-3 all have real data, layer4 is all 0.
  layer1 = factory Fn layer (number row→F1-F12, Esc→`), layer2/3 = empty custom layers (all transparent 0001)
- Official UI layer names: index0=Main layer, 1-3=Fn layers. Switch key codes 0xF100-0xF103 (61696-61699)
- **We previously misinterpreted DeviceFeature r[2] as the layer count** (r[2]=axisType=2 and layer count=2 coincided)
  → fixed to `xs.layers = 4`, layer names are Main/Fn1/Fn2/Fn3
- Full axisType enum: 0=MagneticAxisWired, 1=MagTriple, 2=**MagneticAxisWiredWithScreen (SU66)**,
  3-5=Mechanical family, 6-8=Optical family, 9-11=Inductive family, 12=MagWiredFullScreen,
  13=Vibration, 14=Voice, 15=VibrationVoice, 16=Gamepad
- DeviceFeature `[1,3]` fully decoded: r[2]=axisType, r[3]=connectionMode,
  r[4]={bit0:rgbLighting, bit1:knob}, r[5]={bit0:smallScreen, bit1:fullScreen, bit2:haptic, …, numpad, dotMatrix}

**Dangerous — never send:**
- `[2,17,*]` — **the firmware detaches from USB** (0x48F). Reproduced twice, recovered by replug/reconnect.
  Suspected to be a DFU/OTA boot entry or a debug command
- Detach was also observed when `[3,3,0,1,0]` was hammered with 3 arguments — do unknown-command hammering
  with reopen-per-probe

**Features in the official UI that we have not implemented (in order of value):**
1. ~~Factory default keymap reference (`[3,6]`)~~ → **Implemented** (used for reset via factory_stream)
2. ~~DirectDrive software RGB (`[5,5]`)~~ → **Implemented** (preview on the real device)
3. keyLayoutStyle (`[3,5]`) — physical layout self-description (knob/screen detection, for future model support)
4. Caps color (`[5,6]`) — CapsLock indicator color (write-only, whether to expose it in the UI needs consideration)

## 4. ATK HUB (hub.atk.pro / ATK HUB 3.2.x) — ATK F1 V3 ULTIMATE+ (373b:12a7)

Analysis source: `C:\Program Files\ATK HUB` (Electron) + `https://desktop.vxe.com/v3hub` + `https://hub.atk.pro`
(hub-v3/production/3.2.33). The desktop app's UI is a remote Web app, and HID I/O is done directly by
`window.nodeHid` exposed by the preload = equivalent to WebHID (`navigator.hid`).
`resources/service/atk-hub-service.exe` is a generic HID relay agent (Rust+hidapi, pipe),
`atk-mouse-system-api*.node` is for OS mouse settings and does not contain the device protocol.
Only the firmware ISP is separate (sn_isp_lib_bg.wasm, for SN32F24xB).

### 4.1 Transport (confirmed on a real device 2026-11-15)

- **Collection: usagePage 0xff02 / usage 2 (MI_01 Col05), report ID 0x08**
- Writes are output reports of a 16B packet + reportId=8 (the padding length does not matter).
- ff03/ff04/ff05 have no output report (WriteFile is impossible). ff06 is a different report system.
- Request:  `[0]=cmd [1]=0 [2..3]=addr u16BE [4]=len [5..14]=data [15]=checksum`
- Response:  `[0]=0x08(reportId) [1]=cmd echo [2]=status [3..4]=addr [5]=len [6..]=data [16]=checksum`
- checksum = `0x55 - ((8 + sum(bytes[0..14])) & 0xff)` (JS `Si` class, the constant 8 is reportId)
- status: Success=0 / Fail=1 / EEPROMError=2 / NotSupport=255

### 4.2 Command enum (D)

```
DownLoadData=1 DownLoadDriverStatus=2 GetWirelessMouseOnline=3 GetBatteryLevel=4
SetWirelessDonglePair=5 GetWirelessDonglePairResult=6 SetEEPROM=7 GetEEPROM=8
RestoreFactory=9 ReportMouseStatus=10 EnterUSBUpgradeMode=13
GetCurrentConfig=14 SetCurrentConfig=15 GetMouseCIDMID=16 GetMouseVersion=18
DongleExitPair=19 Set4KRGBMode=20 Get4KRGBMode=21 SetFarDistanceMode=22
GetFarDistanceMode=23 SetDongleLightMode=24 GetDongleLightMode=25 GetDongleVersion=29
ReportMouseUpgradeError=90 ReportMouseUpgradeStatus=91
```

### 4.3 EEPROM address map (JS `y.Address` + real-device values)

All records are `value, crc` pairs (crc = 0x55 - value). Written with SetEEPROM, read with GetEEPROM.

| addr | Contents | Real-device value |
|---|---|---|
| 0 | reportRate block (10B): rate,maxDpi,currentDpi,bhop,buttonOpMode + crc for each | `[64,21,4,81,2,83,0,85,0,85]` = 8KHz, 4 stages, current stage 2, bhop/btnop off |
| 10 | silentHeight (LOD) | `1` |
| 12,20,28,36 | dpi1/3/5/7 legacy record `{xDpi,yDpi,dpiEx,crc}` | `[7,7,0,71]` `[31,31,0,23]` |
| 44,52,60,68 | dpi1/3/5/7 color RGB+crc | `[255,0,0,86]` |
| 76-95 | DPI RGB light (effects,brightness,speed,enable / articleLamp R,G,B,effects,longBright,breathSpeed,energySave) | — |
| 96+4k | key0..15 button assignment (4B) | `[fn,param,0,crc]` fn:Mouse=1 Dpi=2 FirepowerKey=4 ShortcutKey=5 Macro=6 ReportRate=7 ConfigFile=9 DpiLock=10 Wheel=11 TurnAround=12; param bit: Left=1 Right=2 Middle=4 Side1=8 Side2=16 |
| 160 | decorationLight block | — |
| 169 | stabilizationTime (debounce?) | 0 |
| 171 | motionSync | 0 |
| 173 | closeLedTime (LED off, minutes) | 6 |
| 175 | linearCorrection (angle snapping) | 0 |
| 177 | rippleControl (ripple correction) | 0 |
| 179 | moveCloseLights (LEDs off while moving) | 0 |
| 181/183/185 | sensorEnable / sensorTime / sensorMode (sleep-related) | 0,1,6 |
| 187 | rfTxTime | 2 |
| 189 | angle | — |
| 227 | rollingDelay (wheel debounce?) | 30 |
| 256+32k | keyShortcuts0..15 (32B each, **slot = physical button index**) | 0xff=empty |
| 768+384k | macro0..15 (384B each) | — |
| 6912+6k | **paw3955Dpi1..8** (PAW3955 models use this rather than the legacy dpi records) | 6B/stage: `xdpi u16LE, ydpi u16LE, ex, crc=0x55-sum(5B)`, **value is dpi-1** |
| 6960 | dynamicSensitivity (enable + point list) | — |
| 6964 | dynamicSensitivityPoints | — |
| 6984/6992 | sensorCenterPoint / v2 | — |
| 6988 | fastTriggerDebounce | — |

- ReportRate values (bitmask): Hz125=8 250=4 500=2 1000=1 2000=16 4000=32 8000=64
- Key fn enum: Mouse=1 Dpi=2 FirepowerKey=4 ShortcutKey=5 Macro=6 ReportRate=7 Chandeliers=8 ConfigFile=9 DpiLock=10 Wheel=11 TurnAround=12
- Button bits: Left=1 Right=2 Middle=4 Side1=8 Side2=16

### 4.4 Real-device responses (2026-11-15, F1 V3 ULTIMATE+ wired)

```
cmd16 CIDMID  -> [01 3e 16] CID=1 MID=62 (= device DB "1,62" = F1 V3)
cmd18 version -> [01 21]     (FW 1.33?)
cmd4  battery -> [5f 01]     95% + charging state 1
@6912 dpi     -> [143,1 143,1 0 53]=400 [31,3]=800 [63,6]=1600 [127,12]=3200 x5 stages
                 → actually active are the 4 stages from @0.maxDpi=4: 400/800/1600/3200
```

### 4.5 Device DB entry (useRemoteConfig)

`{vendorId:14139(0x373b), productId:4775(0x12a7), usagePage:65284(0xff04), usage:2,
name:"ATK F1 V3 ULTIMATE +", sensor:PAW3955MASTER, mouseCidMid:"1,62",
firmwareMark:"f1", upgradeTogether:true, BottomButton:false, DpiRgb:false, DpiToggle:true}`

Note: the DB's usagePage=0xff04 is for the enumeration filter; the actual command path is ff02/u2.
Models of the same generation (mouseCidMid "1,x") all seem to use this 16B/ff02 scheme. The older generation "2,x"
(VXE R1 family) filters on usagePage 0xff05/usage1 and may use a different protocol.

### 4.6 Additional specs verified on real devices (2026-11-15, second round)

- **ShortcutKey (keyfn type5)**: `keyShortcuts{physical button index}` @256+slot*32.
  Real-device dump (HTS Plus Pro, after a write from the official Hub): `02 81 1f 00 41 1f 00 53 00…`.
  Format `[count][press entry×k][release entry×k (reverse order of press)][crc]` —
  entry = 3B `{keyState, v1, v2}`, keyState = `0x80|type` (press) / `0x40|type` (release).
  **crc = `0x55 - sum(count + all entries)` is placed at index `1+3*count`** —
  writes are only `count*3+2` B (the rest is not written. Without the crc the device does not accept it).
  type: **0=modifier (v1=mod flag 1/2/4/8/16/32/64/128), 1=Keyboard usage LE, 2=media (consumer code)**.
  The press order of modifiers is [Ctrl,Alt,Shift,Win], release is the reverse of press.
  Real device: side buttons = Keyboard usage 31/30 (type1, no modifiers).
- **closeLedTime @173**: the wire value is in **10-second units** (6 = 60s = sleep 1 min). Write round trip confirmed.
- **stabilizationTime @169**: click debounce in ms. Round trip confirmed.
- **bhop @0 byte6**: wheel misoperation prevention (official `time/10`). Round trip confirmed.
- **moveCloseLights @179**: LEDs off while moving. Round trip confirmed.
- **Profile cmd14/15**: `cmd14 → data[0]=active index`, switch with `cmd15 data=[index]`.
  Round trip confirmed on a real device for index 0–3 (4 profiles).
- **Long-distance mode cmd22/23**: on wired, cmd22 ACKs ([2]=1) but cmd23 is always all 0 →
  a **dongle-side feature** that is not reflected on the wired path. Not exposed in settings.
- **RestoreFactory cmd9**: factory reset (ActionSpec, with danger confirmation).
- Not implemented: format of the macro @768 area (384B/slot), dynamicSensitivity @6960+,
  sensorAngle @189, sensorEnable/Time/Mode @181/183/185, BT.

### 4.8 Via the dongle (373b:1278 "Wireless mouse 8k dongle-L", verified on real devices)

- **The same 16B/report-8 protocol is relayed transparently** — all commands work on the ff02/u2 collection.
- cmd16 → CID=1, MID=62 (identifies the inner F1 V3). The registry side does not show the receiver name as is but
  determines the model from CID/MID (`maybe_detect`; undetermined while asleep → retry after waking).
- **cmd29 = dongle FW** (real device: 5,1 → 5.1) — the dongle itself responds even while the mouse is asleep.
- cmd3 GetWirelessMouseOnline — dongle-side response (always returned).
- **Asleep detection**: a relayed command (cmd16/18/EEPROM) timing out → asleep.
  It falls asleep after tens of seconds idle. The dongle's own commands (cmd3/29) respond even while asleep.
- LOD @10: **1–11 → 0.7–1.7mm (0.1mm steps)** — write + official display correspondence confirmed on a real device
  (@10=5 → official display "1.1mm"). 0 and 12 or more can also be written to EEPROM but are outside the official range.
- All EEPROM reads/writes and cmd14/15 profile switching also work via the dongle (round trip confirmed).

### 4.7 Other

- Firmware: obtained from `api-hub-cloud.atk.pro/firmware/{vid}/{pid}/{mark}`.
- Via a wireless dongle: `GetWirelessMouseOnline=3` / pairing cmd5/6/19, battery cmd4,
  dongle FW cmd29. Responses have the same 16B structure.
- Some models have an `is54H20` flag (another MCU variant?).
- Probe: `cargo run -p devhub-hid --example atkprobe` (read only).

## 5. LAMZU Aurora (www.lamzu.net) — Maya X V2 (LM20) verified on a real device

Re-checked 2026-11-17: the [official Aurora](https://www.lamzu.net/#/project/items)'s
[model definitions](https://www.lamzu.net/Config/env-models.json) are v1.0.38. The fetched configuration-handling and screen files matched the previously fetched ones in the audit comparison.
Wired `37B0:0044` / FW `0.0.3.0` reads can be reproduced with `lamzuprobe`.
Past write verification and this read-only confirmation are kept distinct.
The subsequent [app implementation audit](LAMZU_MAYA_X_V2_AUDIT.md) confirmed wireless `37B0:0048` on a real device and
cross-checked the paired mouse `37B0:0044`, mouse and receiver FW, receiver LED, and the GET of the app's basic settings.

### 5.1 Device identification (env-models.json + real device)

`./Config/env-models.json` is the model definition. **The real device is the row "Maya X(LM20)"**
(product string `LAMZU Maya X V2`, FW file name `…Maya_X_V2_8K_Mouse_54LM20…` —
the "MayaX V2" PID `001F` row in the table is a different revision):

- VID `37B0` / PID wired **`0044`** (confirmed on a real device). 8K dongle: `37B0:0048`.
- `IsNewProtocol: "1"`, `IsSensorModelCmd: "1"`, **DPIMax: 50000**,
  `PollingRateWired: 500;1000;2000;4000;8000`, `LOD: "0.7;1;2"`,
  `DPIMaxStageNum: 5`, 5 buttons, `LightEffects: "1,2,4,5"`,
  `SleepTimeGrade: 1min;5min;10min;15min;20min;25min` (not in 10s steps).
- Sensor model command response `4` → **PAW3955** (JS: 1=3395, 2=3950, 4=3955.
  With the 3955 the LOD candidates are `0.7;1;1.3;1.5;1.7`, DPI is 50–50000 in steps of 1).
  `GetDPIMax` (`f=1,cmd=140`) returned `A3` with no arguments, so the upper limit uses the official model definition and sensor detection.
- The profile slots are fixed at 1–4 in the UI, and `devID` = **profile number (1-based)**.
  Even if a slot is shown, it is not necessarily initialized on the real device (§5.6). In per-profile commands
  sending `devID=0` may be rejected. FW, battery, sensor, Glass Mode etc. have different layouts.
- Other model rows have the same format (Maya/Maya X/MayaX V2(001F)/Orcus/Atlantis/Thorn etc.).

### 5.2 Transport — on a real device, 64B feature only

Interface layout of the real device 37b0:0044:

- if0 `0001:02` mouse / if1 `000C:01`, `0001:80`, `0001:06`, `FFA0:01`, `FFFF:01`
  / **if2 `FFFF:00`** ← this is the configuration channel.
- In past real-device verification, sending an output report failed on every collection, and the official code also
  selects the 64B feature helper for `37B0`. The 16B `sendReport(8,…)` is a separate implementation.
  The official code also selects the same 64B helper for the `0048` 8K dongle,
  using target `2` for the mouse FW and target `0` for the receiver FW. The paired mouse is identified by
  the response of `GetDevicePID`. `0048` is shared by several LM20 models.
  The real-device response is `A1`, valid payload `[02,01,37,B0,00,44]` (confirmed as MAYA X V2).
- **All settings use the 64B feature channel**: `send_feature_report(buf65=rid0+64B)`
  → response via `get_feature_report` (~10-30ms; stable with a 120ms sleep + 1 read).
- Send frame (as in the JS): `data[2]=2, data[3]=sub, data[4]=f, data[5]=cmd,
  data[6]=devID, data[7..]=args` (only lighting uses `data[2]=devID`)
- Response frame (Windows hidapi, measured `hidIndex=0`): `a[0]`=report ID 0,
  `a[1]`=status (`A1`=ACK), `a[3]`=normally `data[2]`, `a[4]`=response payload length,
  `a[5]`=f, `a[6]`=cmd. The payload is **`a[7..7+a[4]]`**. For profile-targeted
  commands, normally `a[7]` is `devID` and `a[8..]` is the value. `a[4]` is distinct from the sent `sub`.
  For example, the DPI list has `a[4]=22` with 5 stages in the response for a sent `sub=10`.
- **Writes are applied immediately; no separate save command is needed** (new value confirmed by a get right after a set,
  polling 2↔128, LOD 1↔0x87, MotionSync 0↔1, activeDPI 1↔3,
  writing the same value to a DPI stage — all confirmed by round trip).

### 5.3 16B channel commands (separate helper. Not applicable to the LM20)

JS analysis only (unusable on a real device):

| cmd | Direction | Format |
|---|---|---|
| 7 | write | EEPROM write `[7,0,addr_hi,addr_lo,len(≤10),d0..d9]` |
| 8 | read | EEPROM read `[8,0,addr_hi,addr_lo,len]` → response `b[5..5+len]` |
| - | write | Single-value pair `[7,0,addr_hi,addr_lo,2,v,85-v]` (Set_Device_Eeprom_Value) |
| 4 | read | Battery `[4]` → `b[5]=%`, `b[6]=charging flag`, `b[7]<<8|b[8]`=voltage |
| 5 | - | Pairing mode `[5,0,0,0,2,0,0,11]` |
| 9 | - | ClearSetting (factory reset) |
| 14 | read | Current profile → `b[5]+1` |
| 15 | write | Profile switch `[15,0,0,0,1,profile-1]` |
| 18 | read | FW version → `b[5].hex.b[6].hex` |
| 29 | read | FW version (DeviceID 0x0 path) |
| 1 | read | Dongle EID/PR `[1,0,0,0,8,rand×4]` → `b[10]`/`b[11]` |
| 4 (notification) | ← | Autonomous device notification `[4,type,...]` (below) |

**Notification frame `[4,type,...]`** (UI handler measurement + real-device observation):
- **Confirmed on a real device — on wired it flows in the input report of `if1 up=FFA0 u=01`**
  (not the feature path). On profile switch, `[04,02,02]` (type2,
  b[2]=new profile) and `[04,08,01]` (type8, b[2]=raw rate value of that profile)
  were pushed.
- type1 = DPI switch `b[2]=stage, b[3..4]=X dpi, b[5..6]=Y dpi`
- type2 = profile switch `b[2]=new profile`, type3 = battery `b[2]=%, b[3]=charging`
- type6 = connect/disconnect `b[2]=1/0`, type7 = LOD `b[2]=raw value`
- type8 = polling rate `b[2]=raw value`, type13 = mode `b[2]=HighSpeed, b[3]=tracking`

### 5.4 EEPROM map (Dt — for the 16B path, not applicable to the 0044 wired)

| addr | Contents | addr | Contents |
|---|---|---|---|
| 0 | ReportRate (value table) | 169 | DebounceTime |
| 2 | maxDpiStage | 171 | MotionSync |
| 4 | CurrentDPI (index-1) | 173 | SleepTime |
| 10 | LOD | 175 | Angle (snap) |
| 12 | DPIValue (+stage*4) | 177 | Ripple |
| 44 | DPIColor (+stage*4, RGB+α) | 181 | PerformanceState |
| 96 | KeyFunction (+btn*4) | 183 | Performance |
| 256 | ShortcutKey (+btn*32) | 768 | Macro (+slot*384) |

A single value is stored as a `(v, 85-v)` pair (readMouseFlash reads 0x00-0xFF in one go).

- **ReportRate value table**: 8=125, 4=250, 2=500, 1=1000, 32=2000, 64=4000, 128=8000 Hz
- **DPI stage** @12+i*4: `i=dpi/50-1`, `b[0..1]`=lower part of i,
  `b[2]` bits 2-3/6-7=upper part of i, bit0/1 = ×2 flags (both set gives ×4).
  Actual value = (packed+1)*50*2^flags. When XY are separate, b[0]/b[1] are X/Y.
- **LOD**: `<128 → value=mm` (1=1mm,2=2mm, **3=0.7mm**),
  `>=128 → (v-128)/10 mm`. (Writing 0.7 with value 3 is about the other-vendor path `K2()`. **LM20 / sensor 3955 uses the `T2()` path and writes 0.7 as `0x87` too**).
- **SleepTime**: raw value*10 = seconds (10s→1 … 30min→180).
- **HyperMode**: write the value to PerformanceState@181 and commit with Performance@183=3.
- **KeyFunction** @96+btn*4: `[type,param_hi,param_lo,crc]`.
  `Va` type: Disable0 MouseKey1 DPISwitch2 FireKey4 ShortcutKey5 Macro6
  ReportRateSwitch7 DPILock10 UpDownRoll11 (+UI-specific 14=LOD loop, 20=HPMode loop).
  param: MouseKey 0x0100=L/0x0200=R/0x0400=M, UpDownRoll 0x0100/0x0200,
  DPISwitch 0x0100=cycle/0x0300=down (up is a different value), FireKey=(interval<<8)|count,
  DPILock=dpi/50-1, Macro=index<<8, ReportRateSwitch/loop family=0.
- **ShortcutKey** @256+btn*32: `b[0]=number of entries*2`, 3B entries
  `[flag|type&0xF, val_lo, val_hi]` ×N + crc. flag: **0x80=press, 0x40=release
  (reverse order)**. type: 1=keyboard (val=HID usage), 2=media (val=consumer usage).
  Modifiers are also normal entries (LCtrl=224 etc.). Media example: `[2,0x82,lo,hi,0x42,lo,hi,0]`.
- **Macro** @768+slot*384: `+0`=name len(≤30) + GBK name, `+31`=number of events (≤70),
  from `+32` 5B/event `[flag<<6|type, val_lo, val_hi, delay_hi, delay_lo]`.
  flag as above (press=2<<6=0x80, release=1<<6=0x40).

### 5.5 64B channel command table (verified on a real device — `*` = round trip confirmed)

Send `[data2=2, sub, f, cmd, devID, args]` → the response value is `a[8]` (bool is `==1`).
get/set cmd have a relationship of +0x80/same value. `devID` is the profile number (1-4).

| sub | f | cmd get/set | Function | Real-device response (devID=1) |
|---|---|---|---|---|
| 2 | 1 | 128/0 * | Polling rate (8,4,2,1,32,64,128→125..8000) | `0x80`=8000Hz |
| 2 | 1 | 136/8 * | LOD (get raw value / set: mm value, <1→`*10|128`, 1<v<2→BCD) | `0x87`=0.7mm |
| 2 | 1 | 130/2 * | activeDPI stage | 3 (3rd of 400/800/1600/3200/6400) |
| 10 | 1 | 129/- | DPI stage list get `[dev,6]` → `a[8]`=count, then 4B each X,Y u16BE | 5 stages 400/800/1600/3200/6400 |
| 26 | 1 | -/1 * | DPI stage bulk set `[dev,count,4B×N]` | same-value write → matches |
| 19 | 2 | 129/1 | DPI stage color get `[dev]` → `a[8..]`=RGB×6 / set `[dev,RGB…]` | Read confirmed: `#D00067`, `#00A5D1`, `#0F9B00`, `#FFFFFF`, `#CAC700`. The 6th slot is a spare. set is from the official code only |
| 2 | 1 | 137/9 * | MotionSync | 0→1→0 confirmed |
| 2 | 1 | 132/4 | AngleSnap | 0 |
| 2 | 1 | 138/10 | Ripple | 0 |
| 2 | 1 | 139/11 | HyperMode | 1 (ON) |
| 2 | 1 | 147/19 | TrackingMode (high-speed) | 1 |
| 2 | 1 | 141/13 | DPI XY separate | 0 |
| 2 | 1 | 148/20 | AngleTune angle (negative = 255-\|v\|+1) | 0 |
| 2 | 0 | 136/8 | Debounce | 0 |
| 2 | 0 | 131/- | Battery (no profile argument) → `[a7,a8]` = (charging `==1`, %) | This time `[0,100]`, previously `[1,100]` |
| 3 | 0 | 135/7 | Sleep get→`a8<<8|a9` / set u16BE (seconds) | 60 (=1min grade) |
| 1 | 0 | 133/5 | Profile get→`a[7]` / set | 1 |
| 1 | 0 | 130/- | EID → `a[7]`=EID, `a[9..]`=serial ASCII | 9 |
| 1 | 1 | 143/- | Sensor model (1=3395,2=3950,4=3955) | 4 → PAW3955 |
| 16 | 0 | 129/- | FW version → `a[7..10]` | **0.0.3.0** (matches the FW file name) |
| 2 | 2 | 132/4 | DPI indicator switch (hidden in the LM20's official UI) | Past read value 0 |
| 3 | 3 | 129/1 | ButtonCombine (hidden in the LM20's official UI) | Past read value 1 |
| 3 | 1 | 149/21 | SensorOffset (signed, value in a[9]/o[8]) | 0 |
| 3 | 0 | 154/26 | SpeedEnable = official name **Rapid Trigger** (left and right separately. Maximizes key response, may cause accidental double clicks. Rapid fire = X click is a separate feature via button type2) → `a[8]=left`, `a[9]=right`, set `[dev,L,R]` | Left ON / right OFF |
| 4 | 0 | 153/25 | ScrollHP (B-hop) → `a[8]`=direction (0=OFF,2=up,3=down), `a[9..11]`=time u16BE (200/400/600/800/1000 ms), set `[dev,direction,u16BE time]` | OFF, 200 ms |
| 9 | 1 | 154/26 * | AdaptiveDPICurve — get→`a[8]`=enable,`a[12]`=count,`a[13..]`=(u16BE speed, u16BE ratio×100)×N / set `[dev,en,0,N,data]` sub=4+len | OFF, N=3, speed 0/20/40, all 1.00x (same-value writes only, both past and present) |
| 1 | 1 | 142/14 | GlassMode (value in a[7], no devID) | 0 |
| 6 | 4 | 129/- | GetMacroDataSize `[u16BE slot]` → `a[9..13]`=u32BE size (official code) | A valid slot with a correct frame is unconfirmed |
| 6 | 4 | -/1 | AllocateMacro `[u16BE slot,u32BE size]` | Official code only |
| 7+len | 4 | 131/3 | MacroData get/set `[u16BE slot,u32BE offset,u8 len,data…]` (get has no trailing data) | Official code only |
| 2 | 4 | -/2 | DeleteMacro `[u16BE slot]` | Official code only |
| 5+len | 3 | 128/0 * | Button get/set `[dev,btn,0,type,len,params]` | All 5 are type1 (default mouse button) |
| 6 | 0 | 139/- | GetDevicePID, target `1`, argument `[2]` → `a[9..11]`=VID, `a[11..13]`=PID (u16BE) | Confirmed by reading `37B0:0044` via `0048` |
| 16 | 0 | 129/- | GetUSBPID, target `1` → `a[11..13]`=PID (u16BE) | Official code only |
| - | - | 176/- | enterBL `[0,t,1,0,176]` / exitNormal `…,4,1` | - |
| 2 | 0 | -/0 | defaultDevice factory reset `[2,2,0,0,192,1]` * — the official tool **sends and forgets (does not read the response)**. On wireless, 1 second later it also initializes **the receiver** with `defaultDevice_id(0)` (target 0) (`Xv()`) | All profiles initialized, confirmed on a real device |
| 1 | 0 | -/13 | resetProfile `[2,1,0,13,dev]` * | Factory-initializes the dev profile area, confirmed on a real device |

**Button get/set details** (confirmed from a real device + the JS enum, round trip confirmed):
Send `sub=5+len`, `[dev,btn,0,type,len,param…]`, get uses cmd128+type=255+len=10.
Response `c[9]`=0, `c[10]`=type, `c[11]`=len, `c[12..]`=param — reading, writing and re-reading the stored values was
confirmed by round trip on a real device.
**Important**: if the entry is empty/uninitialized, the read returns the **request echo**
(c[9..11]=req[8..10]). When the button area was broken by probing, all buttons turned into echoes, and
writing with `setbtn` did not recover them; only `resetProfile` (re-initializing the area) did.

**type (`vue` enum)**: 0=OFF, 1=ButtonCode, 2=X_Click, 4=KeyboardCode,
5=ConsumerKeys, 6=PowerKeys, 7=DPI, 8=Profile, 9=FN, 10=Tilt, 12=BatButton,
13=ReportRate, 14=LOD, 15=Paring, 16/17/18=MacroI/II/III, 19=Lighting, 20=HPMode.

**param (by type)**:
- 1=mouse button `[code]`: 1=L,2=R,3=M,4=Backward,5=Forward,6-8=Button6-8,
  16=ScrollUp,17=ScrollDown,18=DPIUp,19=DPIDown,20/21=DPILoopUp/Down
- 2=X_Click `[1,btn,count,int_hi,int_lo]`
- 4=keyboard `[mod,usage]`: mod is a bitmask (LCtrl1,LShift2,LAlt4,LGUI8,
  RCtrl16,RShift32,RAlt64,RGUI128), usage=HID usage ID (1 byte)
- 5=media `[usage_hi,usage_lo]` (Consumer usage u16BE): PlayPause=205,
  Next=181, Prev=182, Stop=183, VolUp=233, VolDown=234, Mute=226, Eject=184,
  MediaSelect=387, Calc=402, MyComputer=404, Explorer=406, WWWHome=547,
  WWWRefresh=551, MyMail=394, LightingUp=111, LightingDown=112,
  SelectApp=419, Copy=539, Cut=540, Paste=541
- 7=DPI `[op]`: 1=StageUp,2=StageDown,5=StageLock(+4B dpi),6=CycleUp,7=CycleDown
- 8/13/14=Profile/ReportRate/LOD `[1=Up,2=Down,3=LoopUp,4=LoopDown]`
- 16=MacroI `[slot_hi,slot_lo,times]`, 17/18=Macro `[slot_hi,slot_lo]`
- 20=HPMode `[0]`
- 15=Paring (pairing), 19=Lighting, 12=BatButton, 10=Tilt, 9=FN, 6=PowerKeys

**Macros**: a dedicated 64B area. `slot` is a device-wide u16BE and carries no profile number.
The official screen transfers at most 50B at a time and concatenates the data located at `a[14..]` of GetMacroData.
The past `A2` response included an extra profile number in the request,
so it cannot be used as evidence that slots 0–2 are empty. Reading and writing a valid macro are unverified on a real device.

**Lighting**: the LM20's `LightEnablePID` is empty, and the official screen shows no independent RGB effect.
Even if `LightEffects=1,2,4,5` or `LightDeviceID=1` are defined, it is not enabled.
The past brightness probes included requests with a target position different from the official one and rejection responses,
so physical light emission or a change of the setting value could not be confirmed. The **DPI stage RGB color** (`f=2,cmd=129/1`) is
a separate feature, shown in the official UI, and its color could be read on the real device this time.

### 5.6 Notes (real-device additions)

- **ACK and the validity of the value are separate things.** This time p1–p3 could be read at first, but p4 returned an invalid rate `0` with `A1`,
  and the declared length of the DPI table exceeded the received size. On re-fetching, the responses of p1–p3 also became temporarily abnormal, and
  fetching the DPI table of p4 produced the Windows HID error `0x1F`. A subsequent fetch of p1 alone was normal.
  The cause and the persistent state of p2/p3 are undetermined. No profile switch, initialization or write was performed on p4.
  **Read from the active profile and do not scan uninitialized slots in bulk**. Check the declared length, command group,
  command ID, target number and value range, and do not overwrite the last readable valid setting with an abnormal value.
- p1 normal response on 2026-11-17: [raw frame](../crates/devhub-hid/fixtures/lamzu/lm20-wired-active.txt).
  8,000 Hz **setting value** (not a measurement of the report interval), DPI 3/5 = 1600, LOD 0.7 mm,
  all 5 buttons at the default assignment. For Adaptive DPI, a same-value write and matching re-read were confirmed with the first existing probe.
  After that, GET only. Reproduce: `cargo run -p devhub-hid --example lamzuprobe -- --details`.
- Do not reinterpret `A2` / `A3` as success or an empty slot. The official WebHID scans all collections
  and looks separately for `featureReports` with `items[0].reportCount==64` and input report ID `4`.
  On the wired real device these are if2 `FFFF:00` and if1 `FFA0:01` respectively.
- **Write persistence confirmed on a real device** — when the settings area was broken during probing, the broken values remained even after replugging
  (power cycling) → 64B feature writes are persistent in flash.
- **Corruption and recovery of the settings area** (completed on a real device):
  - Cause: broad probing writes (probably macro allocate/delete or invalid arguments) corrupted the settings area
    → rate=0, LOD=2, hyper=0x4d, ripple=0x22, stage count=2, all buttons dead.
  - Scalar writes cannot recover it (values could be written but reads kept coming back garbled).
  - **`resetProfile` `[2,1,0,13,dev]`**: factory-initializes that profile's area.
    Confirmed on a real device — the button area and all scalars returned to factory values and reads became normal again.
    An a3 rejection for an uninitialized slot (p4).
  - **`defaultDevice` `[2,2,0,0,192,1]`**: full factory reset. Confirmed on a real device —
    all profiles initialized, EID/serial responses also normalized (the UI's "Factory reset" button).
    Variants `defaultDevice4_7_02_c0` `[dev,2,0,0,192]` / `…,192,2` also exist in the JS.
  - Measured factory defaults: rate=1(1K), stage=2, LOD=1(1mm), sleep=60s,
    5 stages, buttons=type1 (own button), MotionSync=1, HyperMode p1=0x64?? (unstable value).
- **Recovery procedure from corruption** (performed on a real device): `defaultDevice` → wait for re-enumeration →
  write the original values back to each profile → idle for a few seconds → verify by reading back.
- Distinguish confirmation levels. Switching of polling rate, LOD, MotionSync, active DPI, buttons and profiles
  had been verified in the past by changing values and reading back. The DPI list and Adaptive DPI were verified only up to same-value writes.
  DPI color, ScrollHP, SensorOffset etc. were confirmed by reads this time, but writes were not re-verified.
  Macro allocate/transfer/delete, independent RGB, pairing, EID/PID writes, FW update and settings writes over wireless are unverified in this analysis.
- FW update is enterBL → erase → program (.hex, BL=37B0:0044) — no need to implement.

### 5.7 Driver implementation summary (for lamzu.rs)

Minimal implementation confirmed on wired LM20:

- **Detection**: VID `37B0`, PID `0044` (Maya X V2/LM20), if2 `FFFF:00`.
  In the environment table `001F` = a different revision (MayaX V2, DPI 30000) and `0048` = the 8K dongle —
  caps are not split by PID; DPIMax is decided from the sensor model cmd143 and the env values.
- **open**: open the if2 collection as feature-report 64B. `hidIndex=0`.
  devID is the profile number to read (1-based).
- **Response validation**: in addition to `a[1]==0xA1`, check `a[3],a[5],a[6]` against the request's target/group/command.
  For a profile-specified GET also compare `a[7]`. Verify `7+a[4] <= received length`, plus the per-command minimum length and value range.
  Examples: DPI stage count 1–5, DPI payload of `2+4×stages`, DPI value 50–50000.
- **Read**: all scalars use the cmd128-family get (`sub,f,cmd` are in the §5.5 table).
  Buttons use `[2,15,3,128,dev,btn,0,255,10]` → `[type,len,param]` at a[10..];
  **if the entry is empty an echo is returned**, so treat type=0xff as "not set".
  DPI stages and colors are separate requests. First read the active profile with cmd133→`a[7]` and do not scan unused slots.
- **Write**: set is applied immediately + persisted in flash (no save needed). It is believed that it is reliably
  committed after a few seconds idle, but a read right after a write returns the RAM value, so
  it is safer to verify after a few seconds or after reopening.
- **Initialization**: `resetProfile` erases all settings of the specified profile, `defaultDevice` erases all settings.
  Do not use them for automatic recovery from invalid responses. Macros are on hold until a proper backup and restore is verified.
- **Notification frames**: **subscribe to the input report of `if1 FFA0:01`**
  (confirmed on a real device — type2+type8 arrived on profile switch). For HubEvent.
- **Out of scope for the initial implementation**: 0048 wireless, FW update, macros, independent RGB, EID/PID writes.

### 5.8 Corrections and additions (2026-11-17 re-audit → implemented)

- **Polling rate choices**: wired 500/1000/2000/4000/8000, **wireless 1000–8000 (no 500)**. Read values `1` and `16` both mean 1000 Hz.
  Conversion is the official `fM()` (500→2, 1K→1, 2K→32, 4K→64, 8K→128).
- **20K FPS (TrackingMode, get 139) is subordinate to competitive mode (HyperMode)** — the official `s2()` also sends
  `setTrackingMode(false)` when competitive mode is OFF. The app does the same (when OFF it also writes 0 to 20K FPS, and turning 20K FPS ON with competitive mode OFF is rejected).
- **Sleep time** can only be written as 1/5/10/15/20/25 min (official `SleepTimeGrade`). 0 / 0xFF00 / 0xFFFF are "invalid" on read and there is no way to write them.
- **Glass mode**: global `[2,1,1,14,[v]]` / get 142 → `a[7]`. While ON, LOD editing is shown disabled (as in the official tool).
- **AdaDPI curve**: get `[2,9,1,154,[prof]]` → `a[8]`=enabled, `a[12]`=point count N, `a[13..]` = per point speed u16BE + ratio×100 u16BE.
  set is `[prof, en, 0, N, (speed u16BE, ratio×100 u16BE)×N]`, sub = 4+4N. Speed 0–105 (ascending), ratio 0–1.5, 2–5 points.
  Presets: Classic (0,1)(20,1)(40,1) / Natural (0,1.5)(20,0.75)(40,0.1) / Leap (0,0.1)(20,0.75)(40,1.5).
- **Identifying the paired mouse**: via the 0048 dongle, the mouse's own PID is read with `[1,6,0,139,[2]]` (GetDevicePID) and looked up in the model table
  (`devhub-core/src/lamzu.rs` `MODELS`: Maya X V2 0044 / Maya V2 0050 / Maya M 0052 / Orcus V2 0056 / Atlantis Mini V2 0046)
  to build caps. If the VID is not 37B0 it is treated as asleep. A PID not in the table gets `supported=false`.
- **Rapid fire for buttons (X click)** writes only left click, as in the official tool.
- Official Japanese display names (English equivalents): Motion sync / Angle snapping / Ripple / LOD / Glass mode / 20K FPS / Competitive mode / Angle tuning /
  Sleep time / Scroll bhop / Wheel response time / Debounce time / Rapid Trigger / Dongle LED (signal) /
  Reset profile / Factory reset. X-Y axis separation is handled by the X/Y toggle on the DPI page (not shown as a settings row).

## 6. RongYuan (USB VID 3151) — Fantech MAXFIT81, Fantech Tanto PRO WG13P

Driver: `crates/devhub-hid/src/ry.rs`. Probes: `ryprobe` (raw, read-only allowlist), `ryread` (through the driver).

Sources, used for facts only (no code was copied; both projects are GPL-3.0):

- [dniminenn/sharkfin `docs/PROTOCOL.md`](https://github.com/dniminenn/sharkfin/blob/main/docs/PROTOCOL.md) — keyboard command families, receiver relay, hazards.
- [echtzeit-solutions/monsgeek-akko-linux](https://github.com/echtzeit-solutions/monsgeek-akko-linux) — its copy of the vendor web driver's JavaScript
  (`CommonKbYzw` / `CommonKbYc200` / `CommonKbYc500` for keyboards, `CommonMsPan1080` for mice) and its device table.

"HW" below means read (and, where stated, written and restored) on the units on the desk, 2026-11-18.

### 6.1 Transport

- 64-byte feature reports, report id 0, on the collection usage page `FFFF` usage `2` (interface 2). HW.
- Checksum `0xFF - (sum & 0xFF)`: over bytes 0..=6 stored at byte 7 ("Bit7") for everything this driver sends;
  over bytes 0..=7 stored at byte 8 ("Bit8") for the LED commands. HW for Bit7 (the board echoes it: `8f .. 70`).
- A GET answers with the opcode in byte 0. Keymap pages and the mouse key table come back raw, without it.
- One PID is shared by many boards. The model comes from `0x8F` → `[0x8F, id u32 LE]`. HW: MAXFIT81 = 694,
  Tanto PRO WG13P = 3109 (the vendor table calls that platform "Tanto PRO S", PAW3950).
  The registry therefore matches these ids only together with the product string (`SHARED_IDS`), and the driver
  sends nothing but `0x8F` until the id is one it knows.
- Firmware: `0x80` → `(r[2] << 8) | r[1]`, each nibble a decimal digit. HW: keyboard `08 05` = 5.08, mouse `02 03` = 3.02.

### 6.2 Receivers (3151:4011 keyboard, 3151:402d mouse)

The receiver answers `0xF0..0xFE` itself and forwards everything else. HW on 402d:

| opcode | |
|---|---|
| `0xF7` | status: `[0]` 1 = a relayed reply is waiting, `[1]` keyboard battery %, `[2]` mouse battery %, `[3]` 0 = keyboard online, `[4]` 0 = mouse online, `[5]` 1 = will take a packet. Only these bytes are meaningful; the rest keeps the last relayed data. |
| `0xF6 [target]` | relay target, 10 keyboard / 5 mouse. The mouse receiver also relays without it. |
| `0xFC` | hand the waiting reply to the next GET_FEATURE. |

Relayed GET: status (online, can send) → `F6` → the packet → poll `F7` until `[0] == 1` (about 150 ms) → `FC` → read.
A SET leaves no reply. An offline device is reported as asleep. 4011 was only read while its keyboard was on the cable
(status `00 00 00 01 01 01 00`), so the keyboard relay itself is unverified.

### 6.3 Keyboard, yc500 family (MAXFIT81, id 694)

| | GET | SET | layout |
|---|---|---|---|
| profile | `0x85` → `[1]` | `0x05 [n]` | not written |
| options | `0x86 [profile]` → `[2]` bits: 1 Win lock, 8 WASD swap, 16 backlight off; `[4]` power save | `0x06 [profile, bits, r3, r4]` | **not written, see below** |
| LED | `0x87` → `[mode, 5 - speed, brightness, (option << 4) \| flags, R, G, B]`; flags 7 fixed colour, 8 rainbow, 0..6 preset | `0x07`, same bytes, Bit8 | **not written, see below** |
| debounce | `0x91` → `[2]` | `0x11 [0, v]`, 1..10 | HW round trip |
| sleep | `0x92` → four u16 LE seconds at 1..9: sleep BT, sleep 2.4G, deep BT, deep 2.4G | `0x12`, same four at 8..16 | not written |
| auto-OS | `0x97` → `[1]` | `0x17 [0\|1]` | HW round trip |
| base keymap | `0x89 [profile, page]`, 16 slots of 4 bytes per page | `0x13 [profile, slot]`, value at 8..12 | HW round trip |
| Fn keymap | `0x90 [profile, page]` | `0x15 [profile, slot]`, value at 8..12 | HW round trip |

Slot = `column * 6 + row`. The MAXFIT81's map (81 keys, knob push = slot 85, knob turns = slots 90/91) is in
`registry::maxfit81_layout`; its factory tables are in `ry.rs`. Slot values: `[0, 0, usage, 0]` key,
`[0, 0, a, b]` / `[0, a, b, c]` key combination, `[1, 0, F0..F4, 0]` mouse button, `[3, 0, lo, hi]` consumer usage,
`[0x0A, 1, 0, 0]` Fn; `0x0D` / `0x0E` / `0x13` are lighting, connection and mode functions that exist only in firmware.
All zero is a dead key on the base layer and "use the base layer" on Fn.

**Hazard (HW, twice).** The board's control endpoint locked up until it was re-plugged: once when reads followed an
options write (`0x06`) after 30 ms, once when a read followed an LED write (`0x07`) after 500 ms. Both writes were
stored (the options bit and the LED flags read back changed after the re-plug), so the board accepts them and then
stops answering; on an earlier try the same options write only made GETs answer zeros for a moment. Debounce, auto-OS
and key writes never did this, with 30 ms or 500 ms pauses. The likely reading is a long flash commit for the
options/LED blocks during which any report kills the endpoint, but the safe wait is not known. Until it is, the driver
sends only the writes marked "HW round trip", waits 500 ms after each, and retries an empty reply slowly.
The LED test left this unit's LED flags at 8 (rainbow) instead of preset 0 (red); it was not written back.
Never sent at all: `0x02` factory reset, `0xAC` / `0x2C` flash erase, `0x7F` / `0x30` boot entry.

### 6.4 Mouse, pan1080 family (Tanto PRO WG13P, id 3109) — all HW, through the receiver and on the cable

On the cable the mouse is 3151:4026 (a PID many RongYuan mice share) and takes the same packets directly.
Its identify reply differs only in byte 5: 6 on the cable, 5 through the receiver. No battery command is known
for the cable; the level is only available from the receiver's status.

| | GET | SET |
|---|---|---|
| profile | `0x85` → `[1]` | — |
| options | `0xD3` → `[9]` report rate (1 = 1000, 2 = 500, 4 = 250, 8 = 125; `0x81/82/84` = 8K/4K/2K), `[10]` debounce, `[12..14]` flags, `[24..32]` light, `[40..48]` sleep, `[50]`/`[51]` X/Y sensitivity, `[52]` lift-off, `[53]` angle snap | `0x53`: the same 64 bytes with bytes 0..8 replaced by the header. Written whole, so the driver reads, changes one field and writes back. |
| DPI | `0xD4 [profile]` → `[2]` current, `[3]` count, X u16 LE at `8 + 2i`, Y at `24 + 2i`, RGB at `40 + 3i` | `0x54 [profile, current, count]` + the same tables |
| buttons | `0xD0 [profile]` → 16 raw slots; 0..5 = left, right, wheel, forward, back, DPI | `0x50 [profile, slot]`, value at 8..12 |

Round trips (write, read back, restore): report rate, lift-off, angle snap, debounce, the last DPI stage, the DPI button.
Button values: mouse buttons `[1, 0, F0..F4, 0]`, DPI cycle `[0x14, 0, 0, 0]`, keys as on the keyboard.
Not modelled: lighting, sleep timers (units unknown), sensitivity, macros, the wheel slots (12..15).
Battery comes from the receiver's status byte.

## 7. EarFun (Bluetooth RFCOMM) — Air Pro 4+

Driver: `crates/devhub-hid/src/earfun.rs` over `bt.rs` (Winsock `AF_BTH`, SPP UUID `1101`). Probe: `btprobe`.

**Nothing in this section has been run against an Air Pro 4+.** It is written from:

- [Omega-Destroyer/PairFree `docs/earfun/PROTOCOL.md`](https://github.com/Omega-Destroyer/PairFree/blob/main/docs/earfun/PROTOCOL.md) and its `data/earfun/*.json` (from EarFun's own app).
- [onlymash/WaveTune `docs/api/EARFUN_BLUETOOTH_API.md`](https://github.com/onlymash/WaveTune/blob/main/docs/api/EARFUN_BLUETOOTH_API.md) — includes a live capture on an Air Pro 4, used for the frame test.

Both are GPL-3.0; only protocol facts were taken.

Frame: `FF | 04 | flags | length | vendor (2) | command (2) | payload`. Settings are vendor `000A`, command `03xx`;
the reply is `83xx` and starts with a status byte (0 = success). The earbuds also send `03xx` frames unasked.

| setting | read | write | values |
|---|---|---|---|
| battery | `06` left, `07` right, `17` case | — | percent |
| firmware | `09` | — | text |
| noise control | `15` | `14` | 0 off, 1 cancelling, 2 ambient |
| cancelling mode | `3B` | `3A` | 0 ear adaptive, 4 adaptive, 6/5/3/2 levels 1..4, 1 wind |
| ambient mode | `3D` | `3C` | 0 default, 1 full |
| game mode | `13` | `12` | |
| two devices | `26` | `27` | followed by a restart; excluded by LDAC |
| sound quality | `2F` | `2E` | bits 1 aptX, 2 Adaptive, 8 LDAC, 16 Lossless; followed by a restart |
| microphone | `31` | `30` | 0 automatic, 1 left, 2 right |
| voice prompt volume | `39` | `38` | 0 = 100 % .. 4 = off |
| volume limit | `54` | `53` | 0 off, 1 = 90 dB .. 5 = 50 dB |
| balance | `56` | `55` | 0..100, 50 the middle |
| auto power-off | `48` | `47` | 1 = 5 min, 2 = 10, 3 = 30, 4 = 60, 5 = 120, 255 never |
| in-ear | `4A` | `49` | bit 0 set = pausing off, bit 1 set = answering off; other bits written back |
| touch off | `35` | `34` | 0 none, 1 both, 2 left, 3 right |
| touch gestures | `0B` (u16 entries, action in the low byte: L/R × tap 1, 2, 3, hold) | `0A [gesture, action, 0, 0]` left, `[0, 0, gesture, action]` right | gesture 1, 2, 3, 17 |
| find | `33` | `32` | bit 0 left, bit 1 right |
| restart / power off | — | `08` / `51 [1]` | |

Left out on purpose: the EQ (vendor `001D`, `0E01`; it cannot be read back, so a wrong frame would fail silently),
factory reset (`36`), renaming and the paired-device list.

Other Bluetooth devices seen on the desk, listed by the app as detected but without a settings driver:

- **EarFun Wave Pro** (HP200, listed as BES chip): what was checked on the unit on 2026-11-18, all without writing anything
  except the two firmware queries below:
  - Windows' cached service records (`HKLM\SYSTEM\CurrentControlSet\Services\BTHPORT\Parameters\Devices\<address>`)
    hold only SPP (`1101`, instance named "SPP"), A2DP sink (`110B`), AVRCP (`110C`, `110E`) and Hands-Free (`111E`).
    No GAIA (`00001107-D102-11E1-9B23-00025B00A5A5`) and no vendor UUID.
  - The SPP channel opens. It stayed silent for 10 seconds with nothing sent (`btprobe "wave pro" --listen 10`), and
    answers neither the GAIA (`FF 04 00 00 00 0A 03 09`) nor the EF (`EF 10 10 00 00 FE`) firmware query.
  - Bluetooth manufacturer id 0x0642, LMP 13 / subversion 3. I could not confirm which chip vendor 0x0642 is.
  - PairFree's table, taken from EarFun's own app, marks the model as `BesProtocolParser` / "not yet", and no other public
    source describes that protocol. So there is nothing to implement against, and guessing bytes at a device that may
    store settings is not safe. Next step: a capture of the official EarFun app talking to it (Android HCI snoop log, or
    the app's `BesProtocolParser` class from the APK).
- **HUAWEI FreeBuds SE 3**: not investigated.
- **Razer Pro Click Mini** (Bluetooth LE): not listed; it would need a GATT transport, which `bt.rs` does not have.
