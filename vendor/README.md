# vendor/ — Patched copy of GPUI's core (`gpui-pre`)

The `[patch.crates-io]` in `Cargo.toml` makes the build use this directory instead of crates.io's `gpui-pre` **0.3.12**.
It is Apache-2.0 (see `LICENSE-APACHE`). The upstream is Zed's GPUI.
Modified places carry a `DEVHUB` comment. `gpui-pre-windows` is used unmodified from crates.io.

## Differences from the original (vs. crates.io 0.3.12)

| File | Contents |
| --- | --- |
| `gpui-pre/src/elements/div.rs`, `style.rs` | Spring-driven mouse-wheel scrolling (`Style::smooth_wheel`, `set_smooth_wheel`) |

`Cargo.toml.orig` is from the crates.io distribution. Its line endings differ from the original, so a plain diff balloons in line count.
To take a diff, use `diff --strip-trailing-cr`.

## Update procedure

1. Before bumping the gpui-pre version in `Cargo.toml`, extract the new version's original from `~/.cargo/registry/src/`.
2. Port the changes marked `DEVHUB` into the files in the table above, and make `cargo test --locked` pass.
3. Check on a real device that mouse-wheel scrolling still glides.

## History

- 2026-11-19: Removed Liquid Glass for performance. The GPU backdrop filter (`Window::paint_backdrop`,
  `Scene::backdrops`, the Windows `backdrop.rs` / `liquid_glass.hlsl` passes), the nearest-neighbor image
  sampler used by the pixel-art wallpaper, the outer-shadow clip fix for translucent glass and the
  `DEVHUB_GPUI_GPUTIME` GPU timer are gone. `scene.rs` and `window.rs` are back to upstream, and the
  vendored `gpui-pre-windows` copy was deleted in favour of the crates.io release.
- 2026-11-11 / 2026-11-10: Liquid Glass material, rim and shadow fixes (all removed on 2026-11-19).
- 2026-10-13: 0.3.7 → 0.3.9 to match gpui-kit 0.7.3.
- 2026-11-05: 0.3.9 → 0.3.11 to match gpui-kit 0.9.0.
