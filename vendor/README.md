# vendor/ — Adding a GPU backdrop-filter to GPUI (the base of Liquid Glass)

The `[patch.crates-io]` in `Cargo.toml` makes the build use this directory instead of crates.io's `gpui-pre` / `gpui-pre-windows` **0.3.11**.
Both are Apache-2.0 (see each `LICENSE-APACHE`). The upstream is Zed's GPUI.
Modified places carry a `DEVHUB` comment.

## Differences from the original (vs. crates.io 0.3.11)

| File | Contents |
| --- | --- |
| `gpui-pre/src/elements/div.rs`, `style.rs`, `window.rs`, `scene.rs` | Added the backdrop element API and `Scene::backdrops` |
| `gpui-pre-windows/src/backdrop.rs` (new) | Downsample → separable blur → composition of refraction, saturation, brightness, and rim light |
| `gpui-pre-windows/src/gpu_timer.rs` (new) | GPU timestamp measurement that runs only when `DEVHUB_GPUI_GPUTIME=1` |
| `gpui-pre-windows/src/directx_renderer.rs`, `shaders.hlsl`, `build.rs` | Integrated the backdrop pass and shader |

`Cargo.toml.orig` is from the crates.io distribution. Its line endings differ from the original, so a plain diff balloons in line count.
To take a diff, use `diff --strip-trailing-cr`.

## Update procedure

1. Before bumping the gpui-pre version in `Cargo.toml`, extract the new version's original from `~/.cargo/registry/src/`.
2. Port the changes marked `DEVHUB` into the files in the table above, and make `cargo test --locked` and `cargo check --locked --features perf` pass.
3. Check on a real device that the background (`/tech` and GPUI's glass) is not broken.

## History

- 2026-11-11: Added to `liquid_glass.hlsl` a way to turn off the per-surface "right and bottom edge rim".
  The GPU Liquid Glass path does not draw the surface's own edge (`Material::edge`), so the
  1px bright line that appears on the right edge of the nav could not be removed by changing the material's values (`Backdrop::rim[1]` is reused to pass the strength:
  0 = default / positive = that multiplier / negative = off). The default value stays 1.0, so the match with the reference implementation
  does not change (the comparison target of tools/glass-fidelity.ps1 is the same).

- 2026-11-10: Fixed the outer shadow in `shaders.hlsl` to be clipped by the element's outline.
  Fixes the bug where translucent glass took in its own shadow as the background, turning white cards and the nav gray.
  The inner shadow is still drawn inside the element as before.

- 2026-11-10: From the pinned revision `c002c4d3758f934fdfaf173e8214a24eb65a05cd` of `1234igor/gpui-liquid-glass`,
  ported the Regular / Clear material formulas to `gpui-pre-windows/src/liquid_glass.hlsl`. Combined with Windows per-region
  snapshots and a downscaled Gaussian, and tuned the contrast of the dark tint and light Clear.
  `Backdrop::glass` passes the material and interaction energy. Adjacent identical materials in the same layer are merged.
  The MIT license of the port source is [gpui-liquid-glass-LICENSE-MIT](../licenses/gpui-liquid-glass-LICENSE-MIT),
  and for GPUI's Apache-2.0 see `LICENSE-APACHE` in each vendor directory. Both are bundled with the Windows distribution.

- 2026-10-13: 0.3.7 → 0.3.9 to match gpui-kit 0.7.3. The differences in the table above applied cleanly (window.rs only shifted by 182 lines).
  Rendering (directx_renderer.rs / shaders.hlsl / style.rs) is unchanged upstream. Captures of 5 screens match the old version (0.1.15) pixel for pixel.
- 2026-11-05: 0.3.9 → 0.3.11 to match gpui-kit 0.9.0.
  div.rs / window.rs / scene.rs / directx_renderer.rs / shaders.hlsl had changed upstream, so a three-way merge was done
  (the 2-line smooth_wheel declaration in div.rs and 1 hunk in scene.rs were fixed by hand). shaders.hlsl and directx_renderer.rs
  applied cleanly. build.rs is unchanged upstream.
