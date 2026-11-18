# Device HUB Live: OBS overlay

- When the displayed keys are narrowed down, empty rows and gaps within rows are closed up. With all keys shown, the original physical layout is kept.
- On a key, `FT` is first travel, `AP` is the actuation point (the re-ON amount for RT), and `R` is reset. The unit is mm. Normal keys show `AP` / `R`; with Rapid Trigger they show `FT` / `AP` / `R`. The values are read when the target device is detected.
- ~~The mouse Hz chip~~ was removed (Windows 11 restricts raw input delivery to background processes, so when OBS was in the background at delivery time the value could not be trusted). Check the measured polling rate on the app's Performance / Test screen while it is in the foreground.
- Magnetic buttons use a display with bars that distinguishes the current depth, the trigger position (% of the stroke), and the RT sensitivity (`OFF` when disabled).

`cargo build --locked --release -p devhub-obs` produces `target/release/devhub_obs.dll`. To apply it to an existing OBS, close OBS, replace the installed DLL, and restart OBS. The source's render size changes, so adjust the placement or crop in the scene if needed.

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
