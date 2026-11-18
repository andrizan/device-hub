//! Render every theme × every mode to PNG without a device (for design review).
//!   cargo run -p devhub-obs --example gallery  → obs-gallery-*.png
use devhub_obs::config::{MouseStyle, OverlayCfg, Theme};
use devhub_obs::render::{self, Mode, Vis};

fn main() {
    let font = render::font();
    let layout = devhub_core::registry::su66max_layout();
    let selected = layout
        .iter()
        .filter(|k| {
            matches!(
                k.label.as_str(),
                "Q" | "W" | "E" | "A" | "S" | "D" | "LCtrl"
            )
        })
        .map(|k| k.id)
        .collect();
    let live = render::demo_live(
        Some(("SU66 Max".into(), layout, 4.0)),
        Some(("HTS Plus Pro".into(), vec!["Left".into(), "Right".into()])),
        0.55,
    );
    let compact = Vis {
        show_key_ids: selected,
        ..Vis::default()
    };
    let pix = render::draw(&live, font.as_ref(), Mode::Kb, &compact);
    image::save_buffer(
        "obs-compact-keyboard.png",
        &pix.buf,
        pix.w,
        pix.h,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    for theme in Theme::ALL {
        for (mode, tag) in [
            (Mode::Kb, "kb".to_string()),
            (Mode::Mouse, "mouse".to_string()),
            (Mode::Both, "both".to_string()),
        ] {
            for ms in MouseStyle::ALL {
                if mode == Mode::Kb && ms != MouseStyle::Both {
                    continue;
                }
                let cfg = OverlayCfg {
                    theme,
                    numbers: true,
                    mouse_style: ms,
                    ..Default::default()
                };
                let vis = Vis::default().with_cfg(&cfg);
                let pix = render::draw(&live, font.as_ref(), mode, &vis);
                let sty = if mode == Mode::Kb {
                    String::new()
                } else {
                    format!("-{}", ms.label())
                };
                let f = format!("obs-gallery-{theme:?}-{tag}{sty}.png").to_lowercase();
                image::save_buffer(&f, &pix.buf, pix.w, pix.h, image::ExtendedColorType::Rgba8)
                    .unwrap();
                println!("{f} {}x{}", pix.w, pix.h);
            }
        }
    }
}
