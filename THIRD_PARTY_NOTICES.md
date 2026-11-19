# Third-Party Notices

Device HUB bundles or derives from the following third-party works.

## Vendored code

| Package | Copyright | License | Notes |
| --- | --- | --- | --- |
| `vendor/gpui-pre` | Zed Industries | Apache-2.0 (`LICENSE-APACHE` in the source dir; `licenses/gpui-pre-LICENSE-APACHE` in Windows packages) | GPUI fork with a spring wheel-scrolling patch; see `vendor/README.md` in the source tree |

## Bundled fonts (`crates/device-hub/fonts/`)

All embedded into `device-hub.exe` at build time.

| Font | Copyright | License |
| --- | --- | --- |
| Geist | The Geist Project Authors | SIL OFL 1.1 (`licenses/Geist-OFL.txt`) |
| IBM Plex Sans JP | IBM Corp. | SIL OFL 1.1 (`licenses/IBMPlexSansJP-OFL.txt`) |
| Zen Maru Gothic | The Zen Maru Gothic Authors | SIL OFL 1.1 (`licenses/ZenMaruGothic-OFL.txt`) |

## Icons

The UI includes SVG paths derived from [Lucide](https://lucide.dev) (ISC,
with MIT terms for icons derived from Feather). See `licenses/lucide-LICENSE`.

## Analysis material

The repository documents observed protocol behavior and includes device
report captures used as test fixtures. Copies of vendor web bundles and
extracted source files are kept outside the tracked source tree.
