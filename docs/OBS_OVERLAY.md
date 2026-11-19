# Device HUB Live: OBS overlay

- When the displayed keys are narrowed down, empty rows and gaps within rows are closed up. With all keys shown, the original physical layout is kept.
- For keys, `FT` is first travel, `AP` is the actuation point (the re-ON amount with RT), and `R` is reset. The unit is mm. Normal keys show `AP` / `R`, and Rapid Trigger shows `FT` / `AP` / `R`. The values are read when the target device is detected.
- To overlay the measured Hz of a USB mouse, see "Live USB rate display" below.
- Magnetic buttons use a bar display that distinguishes the current depth, the trigger position (% of the stroke), and the RT sensitivity (`OFF` when disabled).

`cargo build --locked --release -p devhub-obs` produces `target/release/devhub_obs.dll`. To apply it to an existing OBS, close OBS, replace the installed DLL, and restart OBS. The source's render size changes, so adjust the placement or crop in the scene if needed.

## Live USB rate display

In OBS's "Add Source", choose **Device HUB USB Polling Rate**. The plugin opens the Windows USBPcap driver directly. USBPcap only needs to be installed once. Python, Wireshark, and the text source are not needed while streaming. The measurement code is `crates/devhub-polling` — it gives the same value as the "USB transfer" method on the app's "Test" tab (only 1 process can hold the same hub at a time).

1. Install [USBPcap](https://desowin.org/usbpcap/) and restart as the installer instructs. Start OBS with the updated DLL.
2. Pick the product from the "Target device (product)" list in the properties — the hub number and USB address are resolved automatically from its current values (after unplugging/replugging or changing ports, stop measuring, select the target again, and resume). For the IN endpoint, enter only the number (0x81 → 1, which is what most mice use). Report ID defaults to -1 (all). If your product is not listed or you want to set it manually, leave the list at "(manual hub/address below)" and enter the hub number and address by hand — you can find them with the [steps for identifying the target USB](POLLING_RATE.md).
3. Enable "Measure USB input" and move the mouse. It shows the number of successful target transfers in the last 1 second, updated every 250 ms. It shows a waiting display until the first input is received, and 0 Hz while idle after that. With a receiver, it measures the USB transfers to the PC.
4. If access is denied, start OBS as administrator. Only 1 source can measure the same hub at a time. In other scenes, share it with "Add Existing". Clearing the measurement checkbox or deleting the source releases the hub. After unplugging/replugging, check the address and turn measurement on again.

### Using a text source (legacy path for comparison — normally use the USB source above)

It counts the movement interrupt IN transfers of a wired USB mouse or a 2.4 GHz receiver and passes the count for the last 1 second to OBS's "Text (GDI+)" source. It can be placed freely on top of Device HUB Live. The capture side updates the displayed value every 250 ms, and it is reflected in the stream when OBS reloads the file. For wireless, the observed span is from the receiver to the PC. USB polling attempts while idle are not included in the count.

1. Prepare Python 3.8 or later and USBPcap on Windows, and identify the target with [the steps for checking the USB device and endpoint](POLLING_RATE.md).
2. Run the following in an administrator PowerShell at the repository root. In the example, 1, 5, and 0x81 are the USBPcap number, the USB address, and the mouse movement IN endpoint, respectively. If several functions share the same endpoint, specify the Report ID you confirmed for the mouse with `--report-id`.

   ```powershell
   python .\scripts\obs-usb-rate.py --hub 1 --device 5 --endpoint 0x81
   ```

3. In OBS, choose "Sources" → "+" → "Text (GDI+)", select "Read from file", and specify the absolute path of `obs_usb_rate.txt` shown in the console. Even after you switch to the game, it keeps updating while the script is running.
4. Until the first target input it shows `USB  -- Hz`; if no input arrives for 1 second or more after receiving, it shows `USB  0 Hz`. Quitting with Ctrl+C returns it to the waiting display. If the USB address changed after reconnecting, restart with the new value.

[USBPcap capture format](https://desowin.org/usbpcap/captureformat.html) / [OBS text sources](https://obsproject.com/kb/text-sources)

## Standalone use (no Device HUB app needed, OSS distribution)

The plugin opens the device itself via HID, so **it works even if the Device HUB app is not installed or running**.
So that every display setting can be changed from the OBS source properties alone, even where the app is absent,
the properties have a "**Configure everything in OBS (ignore app settings)**" toggle and an "Appearance" group:

- OFF (default): as before — the app's `obs_overlay.json` takes effect, and the OBS properties cover only title / labels / accent / display key selection.
- ON: all of the "Appearance" items below are enabled, and `obs_overlay.json` is not read. It works with the defaults even where there is no settings file.

Items in the Appearance group: theme / how the mouse is drawn / arrangement / fill direction / toggles for numbers, %, badges, actuation line, shadow, and pressed color /
sliders for render scale, key size, corner radius, padding, panel opacity, and glow / custom title text.
The accent color (presets + custom color picker) and the key/button display checkboxes stay as the existing properties regardless of the mode.

## Locale (OSS distribution)

Properties, the source name, and the module description are all looked up through OBS's plugin locale mechanism
(`obs_module_get_locale_string`). Placing ini files in `data/locale/` follows the OBS UI language, and
when they are not loaded, a key is missing, or OBS is old, the English fallback in the code is shown:

- `data/locale/en-US.ini` — default (all keys required)
- `data/locale/ja-JP.ini` — Japanese

If a distribution package bundles `bin/64bit/devhub_obs.dll` + `data/locale/*.ini`, it runs fully standalone.

## Customization (`%APPDATA%\DeviceHub\obs_overlay.json`)

Edit it from the app's "OBS overlay" page; the plugin reloads it about once per second.
All new fields are `serde(default)`, so old settings files can be read as they are.

### Design

| Key | Type | Default | Description |
|---|---|---|---|
| `theme` | `Glass`/`Light`/`Clear`/`Neon` | `Glass` | color scheme preset. `Clear` has no panel |
| `accent` | `[r,g,b]` | `[60,190,255]` | color of the border, title, and Hz. A color chosen in the OBS source settings takes priority |
| `pressed` | `[r,g,b]` or `null` | `null` | pressed color (fill, glow, pressed outline). `null` = follow the accent |
| `fill_mode` | `Bottom`/`Top`/`Fade` | `Bottom` | direction of the press fill |
| `panel_alpha` | 0–100 | `100` | background panel opacity % |
| `glow` | 0–200 | `100` | glow intensity % when pressed |
| `shadow` | bool | `true` | drop shadow of keys and panel |
| `scale` | 0.75–3.0 | `1.0` | render resolution |
| `mouse_style` | `Both`/`Shape`/`Bars` | `Both` | how the mouse is drawn |

### Layout

| Key | Type | Default | Description |
|---|---|---|---|
| `key_size` | 0.7–1.4 | `1.0` | size factor of one key (density) |
| `radius` | 0–2 | `1.0` | corner radius factor. 0 is square |
| `pad` | 0.5–2 | `1.0` | inner panel padding factor |
| `arrange` | `Side`/`Stack`/`Swap` | `Side` | arrangement of the combined "keys + mouse" source |

### What to show

| Key | Type | Default | Description |
|---|---|---|---|
| `title` | bool | `true` | title row |
| `title_text` | string | `""` | title text. Empty = device name |
| `labels` | bool | `true` | key names and button names |
| `badges` | bool | `true` | AP/FT/R text inside keys |
| `trigger_line` | bool | `true` | actuation line (the horizontal line inside a key) |
| `numbers` | bool | `false` | numeric press amount |
| `num_pct` | bool | `false` | show the number as stroke % instead of mm |
| ~~`rate`~~ | — | — | (removed — the measured Hz chip itself was deleted) |
| `show_keys`/`show_buttons` | array | `[]` | keys to show (`LayoutKey::id`) / buttons (display names). Empty = all |

### Priority

- A source with "Configure everything in OBS (ignore app settings)" ON does **not look at this file at all** — the AND rule below applies only to sources with it OFF.
- The display filter is an AND of **app ∩ OBS properties** (if both specify something, the intersection).
- `title`/`labels` are shown only when both the app side and the OBS side are ON.
- For `accent`, a color explicitly set on the OBS side takes priority. New items such as `pressed` are set on the app side only.
- The old `hide_keys`/`hide_buttons` are read-compatible only (not written on new saves).
