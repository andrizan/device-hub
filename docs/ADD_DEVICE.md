# Adding a device (supporting a new mouse / keyboard)

This procedure is written so that it can be used as is in **another session, another agent, or another environment**,
and can be read without any prior knowledge.

- The UI (`crates/device-hub`) needs **no changes**. Once the driver returns `Capabilities`, the screens appear automatically.
- The work happens in `crates/devhub-core/src/registry.rs` (1 line) and `crates/devhub-hid/src/<driver>/` (protocol).

## Current support table

| Device | VID:PID | usage page | Driver | Web version | Status |
|---|---|---|---|---|---|
| SU66 Max | 1CA6:3009 | FFB0 / 01 | Xsyd (`xsyd.rs`) | hub.xsyd.top/keyboard | **v2 verified on hardware** (keymap, lighting, per-key performance, polling, settings, configuration, advanced keys, macros, live_travel, calibration) |
| HTS Plus Pro (wireless) | dongle 33E4:3854 | FF02 / 02 | Mousefit → `mousefit8.rs` (report-8/EEPROM) | www.mouse.fit | **v2 verified on hardware** (all settings, DPI, buttons, macros, polling 125–8K, magnetic-button calibration/press depth, sleep detection / automatic re-read on wake) |
| HTS Plus Pro (wired) | 33E4:5219 | FF02 / 02 | Mousefit → `mousefit8.rs` (report-8/EEPROM) | www.mouse.fit | **v2 verified on hardware** (feature reports do not work even on the cable; report-8 is used) |
| HTX Ultra | 33E4:5608, 5617, 5618, 5619 | FF02 / 02 (presumed) | Mousefit | www.mouse.fit | registered from the model table / unverified |
| ATK F1 Ultimate | TBD | TBD | TBD | (not in mouse.fit's model table) | See §7 |
| Fantech MAXFIT81 (cable) | 3151:4010 | FFFF / 02 | Ry (`ry.rs`) | — | **Verified on hardware**: reads; writes for keys (both layers), debounce, auto-OS. Lighting / options / sleep / profiles are read-only on purpose (PROTOCOL_NOTES §6.3) |
| RongYuan 2.4G keyboard receiver (the MAXFIT81's) | 3151:4011 | FFFF / 02 | Ry (`ry.rs`) | — | Generic entry: receiver status read; relay to the keyboard unverified |
| Fantech Tanto PRO WG13P (2.4G receiver) | 3151:402D | FFFF / 02 | Ry (`ry.rs`) | — | **Verified on hardware**: DPI stages, buttons, report rate, lift-off, angle snap, debounce, battery, sleep detection |
| Fantech Tanto PRO WG13P (cable) | 3151:4026 | FFFF / 02 | Ry (`ry.rs`) | — | **Verified on hardware**: the same settings, written and restored directly. No battery reading on the cable |
| EarFun Air Pro 4+ | Bluetooth name | SPP `1101` | Earfun (`earfun.rs`) | — | Written from protocol notes; not tested on a unit (PROTOCOL_NOTES §7) |
| EarFun Wave Pro, HUAWEI FreeBuds SE 3 | Bluetooth name | — | none | — | Detected only; no settings protocol |

Bluetooth devices are not in `KNOWN`: they are matched by their paired name in `registry::KNOWN_BT` and opened
over RFCOMM (`crates/devhub-hid/src/bt.rs`).

RongYuan products share USB ids with other vendors' boards, so a registry entry must not claim the whole id:

- `registry::SHARED_IDS` lists such ids with a piece of the HID product string; `match_device` only matches
  when the product string contains it. Another mouse on 3151:4026 is simply not listed.
- A keyboard receiver (3151:4011) names no product, so it is listed as "2.4G keyboard receiver" and named by
  the driver once the keyboard behind it answers.
- `ry.rs` asks the device for its id before every first read or write (`RyDriver::board`) and refuses ids
  outside `BOARDS`. A receiver whose device is asleep opens with empty capabilities; nothing is assumed.

When adding a board on a shared id, add both the `SHARED_IDS` line and the `BOARDS` entry.

**Note that the protocol changes with the connection type**: mouse.fit's new protocol (64B feature report) is
for wired models only — the 8K dongle has no Windows feature report at all, and through the dongle you can
configure the mouse only via the 16B report-8/EEPROM path (`mousefit8.rs`). The model behind a dongle cannot be
queried with report-8, so model differences are looked up from the registry's model (3854 assumes
the paired model).

---

## 0. Overall flow

1. Identify the device (VID / PID / usage page)
2. Analyze the web configurator (read the JS + record the traffic)
3. Register it in the registry → at this point the UI shows it as "detected, unsupported"
4. Write the driver (caps → read → apply → actions)
5. Save frames to fixtures and write tests
6. Verify on hardware and update the docs

## 1. Identification

```bash
cargo run -p devhub-hid --example enum      # enumerate all HID interfaces
```

What to record: **VID, PID, product string, usage_page, usage, interface number, connection type**
(the PID often changes between wired / 2.4G dongle / 8K dongle — record them all).

The configuration interface is almost always vendor-defined (`usage_page >= 0xFF00`).
In PowerShell, `Get-PnpDevice -Class HIDClass | ft FriendlyName, InstanceId` also shows the PID.

## 2. Analyzing the web configurator

### 2.1 Getting the JS bundles

Open the configurator in Chrome / Edge and, in the DevTools Console:

```js
// list of loaded JS and JSON
performance.getEntriesByType('resource').map(r => r.name).filter(u => /\.(js|json|wasm)/.test(u))
```

Vite-based sites (mouse.fit, hub.xsyd.top) split the code into chunks per screen. `fetch` the main JS, collect the
chunk names it references with `/[\w\-]+\-[\w]{8}\.js/g`, and `fetch` them all to get the whole code.
To save it, right-click in the Sources tab → Save as, or:

```js
const t = await fetch(URL).then(r => r.text());
const a = Object.assign(document.createElement('a'), {href: URL.createObjectURL(new Blob([t])), download: 'bundle.js'}); a.click();
```

### 2.2 Where to read

| String to look for | What it tells you |
|---|---|
| `requestDevice` / `filters` / `vendorId` / `usagePage` | supported VID/PID, configuration interface |
| `sendFeatureReport` / `receiveFeatureReport` | feature report type (mouse.fit family) |
| `sendReport` / `inputreport` | output/input report type (xsyd family) |
| `reportId` / `new Uint8Array(65)` / `new Uint8Array(64)` | frame length |
| `Crc` / `checksum` / `^=` | checksum / obfuscation |
| Japanese/English UI text (i18n dictionary) | **the full list of features** (what to declare in caps) |
| a settings JSON such as `env-models.json` | **per-model value ranges** (the caps values themselves) |
| `J(this,"getXxx"` / `async getXxx(` | methods of the protocol class = the full list of operations |
| constant objects such as `{ReportRate:0, ...}` | address tables, command numbers |

Organize the results in your own words and append them to `docs/PROTOCOL_NOTES.md` with the source and the range
confirmed on hardware. Keep the distribution files and code excerpts you obtained outside Git. If you use traffic
captures in tests, extract only the HID reports you need and check that they contain no serial numbers or personal settings.

### 2.3 Recording the traffic (to pin down field positions)

Before reading the configurator's JS, paste the following snippet into the Console and then "connect"; all traffic is logged:

```js
(() => {
  const hex = b => [...new Uint8Array(b.buffer ?? b)].map(x => x.toString(16).padStart(2, '0')).join(' ');
  const P = HIDDevice.prototype;
  for (const m of ['sendReport', 'sendFeatureReport']) {
    const o = P[m];
    P[m] = function (id, data) { console.log(`>> ${m} id=${id}`, hex(data)); return o.call(this, id, data); };
  }
  const r = P.receiveFeatureReport;
  P.receiveFeatureReport = async function (id) { const v = await r.call(this, id); console.log(`<< feature id=${id}`, hex(v)); return v; };
  const a = P.addEventListener;
  P.addEventListener = function (t, f, o) {
    if (t === 'inputreport') { const g = f; f = e => { console.log(`<< input id=${e.reportId}`, hex(e.data)); g(e); }; }
    return a.call(this, t, f, o);
  };
  console.log('HID hook installed');
})();
```

**Changing just one item and looking at the difference in what is sent** is the fastest way (e.g. change DPI from 800 to 1600 and see which bytes change).
USBPcap + Wireshark can capture it too, but hooking WebHID is faster.

### 2.4 Saving as fixtures

```
crates/devhub-hid/fixtures/<model>/
  read_dpi.req.bin   read_dpi.resp.bin   read_dpi.md   ← what the frame is and what can be read from it
  set_dpi_1600.req.bin ...
```

This lets another agent, or an environment without the device, write tests.

## 3. Register in the registry

One line in `KNOWN` in `crates/devhub-core/src/registry.rs` (one line per connection type):

```rust
KnownDevice {
    vid: 0x1234, pid: 0x5678,
    usage_page: Some(0xFF00), usage: Some(0x01),
    kind: DeviceKind::Mouse,
    vendor: "Vendor", model: "model-id", name: "Display name",
    transport: Transport::Wired,
    driver: DriverKind::Mousefit,   // use this if an existing protocol can talk to it, otherwise §4.3
    verified: false,
},
```

With just this, the device appears in the sidebar of `cargo run -p device-hub` as "unsupported" (`supported=false`).

## 4. Driver

### 4.1 Adding a model to an existing protocol family

Example: another G-Wolves mouse on mouse.fit → just add the env-models.json row to the model table in `src/mousefit/models.rs`.
Build caps automatically from the model table (BACKEND_BRIEF §4).

### 4.2 How to write caps

`crates/devhub-core/src/demo.rs` is a complete example.

- Scalar settings → `SettingSpec::toggle / slider / choice` (pick the section). Match the id to the table in BACKEND_BRIEF §2.
- Operations → `ActionSpec` (`.danger("confirmation text")` if dangerous, `.file(&["bin"])` if it should ask for a file).
- Mouse → `MouseCaps` (DPI range, number of stages, button names, assignable kinds, macros).
- Keyboard → `KeyboardCaps` (for the layout, add a function to the registry; the catalog is the device's key-code space; PerfCaps; advanced keys).
- Lighting → `LightArea` (effect list, and whether color/speed/direction exist).
- **Do not declare features the device does not have.** The UI hides them automatically.

### 4.3 A new protocol family

1. Add a variant to `DriverKind` in `crates/devhub-core/src/model.rs`.
2. Implement `DeviceDriver` in `crates/devhub-hid/src/<name>/mod.rs`.
3. Add branches to `driver()` and `driver_ready()` in `crates/devhub-hid/src/lib.rs`.

### 4.4 Implementation checklist

- [ ] `info()` has firmware / dongle firmware / battery
- [ ] `caps()` matches the web version's feature list (i18n dictionary)
- [ ] `read()` covers every item. Items that cannot be read do not go into the map
- [ ] `apply()` handles every `Change` variant (for unsupported ones, do not return `Err(Unsupported)`; do not declare them in caps)
- [ ] a save command if needed (so values do not revert after a power loss)
- [ ] `Err(Disconnected)` on unplug, `asleep` on wireless sleep
- [ ] unit tests using fixtures

## 5. Verification

1. Start without `DEVHUB_DEMO` → open the device → try every tab.
2. Check each value in the web version (open the browser only for verification; do not use the app while it is open).
3. Confirm that the values persist across unplug/replug and power cycles.
4. Update `verified: true` in the registry, the table in this document, and PROTOCOL_NOTES.

## 6. Common pitfalls

- If you do not match the usage page, you will open another interface with the same VID (such as the keyboard input itself).
- hidapi's write needs the report ID in the first byte (even if it is 0) → send a 64B frame as 65B.
- hidapi's read **does not prepend the report ID for unnumbered reports** (xsyd: buf[0] is the start of the data;
  for numbered ones like report-8, buf[0] is the ID). `transport::Channel::recv` returns the raw buffer, so the
  protocol side decides.
- **WebHID's `sendFeatureReport` succeeding is not the same as Windows' `HidD_SetFeature` succeeding.**
  On an IF that has no feature report, like the dongle, the web version cannot use that path either
  (the JS swallows it with try/catch, so on the site "nothing happens"). Try
  `send_feature_report`/`get_feature_report` on the real device before choosing the transport.
- The start of the response data may be shifted by 1 byte (mouse.fit reads from `r[6]`).
- Responses via a wireless dongle are slow (add a commonDelay of about 20ms). While the mouse is asleep,
  only the dongle ACKs with an "all-zero data part" → `DeviceInfo.asleep`.
- A value read back after a write may have been rounded (DPI step, mm×1000 quantization).
- The same setting may have a **different storage format depending on the connection path** (mouse.fit's ReportRate@0 is
  not an index but the same code table as the new protocol). Follow the web version's encode all the way before deciding.

## 7. Undetermined models (ATK F1 Ultimate etc.)

1. Plug in the real device and get VID/PID/usage with §1.
2. If the VID is `33E4` and the PID is in mouse.fit's model table, it is Mousefit. Otherwise:
   - Open the ATK / VXE-family configurator (follow the link from the manufacturer's site) and analyze it with §2.
   - Also check where mouse.fit's "Can't connect to the device?" (LinkJump) leads.
3. If it is the same as an existing protocol, just add it to the registry. If it differs, see §4.3.
