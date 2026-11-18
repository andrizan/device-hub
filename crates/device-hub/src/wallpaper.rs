//! Wallpaper that serves as the base of the glass.
//!
//! The GPU backdrop is built from "what the app drew", so the desktop does not show through.
//! So a wallpaper is laid across the whole window. The default is a procedurally generated aurora (one still image = zero
//! drawing cost when idle). It can be replaced with any image (PNG/JPEG) in settings.

use gpui_kit::*;
use image::{Frame, RgbaImage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct Blob {
    x: f32,
    y: f32,
    r: f32,
    c: [f32; 3],
    k: f32,
}

fn rgbf(v: u32) -> [f32; 3] {
    [
        ((v >> 16) & 255) as f32,
        ((v >> 8) & 255) as f32,
        (v & 255) as f32,
    ]
}

fn blobs(light: bool, accent: u32) -> Vec<Blob> {
    let a = rgbf(accent);
    let b = |x, y, r, c: [f32; 3], k| Blob { x, y, r, c, k };
    if light {
        vec![
            b(0.12, 0.10, 0.62, rgbf(0xa9dcff), 1.0),
            b(0.88, 0.06, 0.55, rgbf(0xd4c2ff), 0.9),
            b(0.80, 0.94, 0.66, rgbf(0x9ff0de), 0.85),
            b(0.18, 0.90, 0.58, rgbf(0xffcce2), 0.8),
            b(
                0.55,
                0.45,
                0.5,
                [a[0] * 0.5 + 127., a[1] * 0.5 + 127., a[2] * 0.5 + 127.],
                0.55,
            ),
        ]
    } else {
        vec![
            b(0.08, 0.04, 0.62, rgbf(0x15406a), 0.95),
            b(0.94, 0.10, 0.52, rgbf(0x3c1f70), 0.85),
            b(0.84, 0.96, 0.64, rgbf(0x0b4c4a), 0.9),
            b(0.14, 0.94, 0.56, rgbf(0x52163f), 0.8),
            b(
                0.56,
                0.52,
                0.42,
                [a[0] * 0.32, a[1] * 0.32, a[2] * 0.32],
                0.7,
            ),
        ]
    }
}

fn hash(x: f32, y: f32) -> f32 {
    let h = (x * 127.1 + y * 311.7).sin() * 43758.547;
    h - h.floor()
}

fn generate(light: bool, accent: u32, w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    let (top, bot) = if light {
        ([234., 240., 248.], [247., 248., 252.])
    } else {
        ([5., 7., 12.], [9., 10., 17.])
    };
    let bl = blobs(light, accent);
    let (wf, hf) = (w as f32, h as f32);
    for y in 0..h {
        let v = y as f32 / hf;
        for x in 0..w {
            let u = x as f32 / wf;
            let mut c = [
                top[0] + (bot[0] - top[0]) * v,
                top[1] + (bot[1] - top[1]) * v,
                top[2] + (bot[2] - top[2]) * v,
            ];
            for b in &bl {
                let dx = (u - b.x) * (wf / hf);
                let dy = v - b.y;
                let k = (-(dx * dx + dy * dy) / (b.r * b.r * 0.33)).exp() * b.k;
                if k > 0.001 {
                    for (ci, bi) in c.iter_mut().zip(b.c.iter()) {
                        *ci += (bi - *ci) * k;
                    }
                }
            }
            let n = (hash(u * 91., v * 57.) - 0.5) * if light { 2.0 } else { 3.2 };
            img.put_pixel(
                x,
                y,
                image::Rgba([
                    (c[0] + n).clamp(0., 255.) as u8,
                    (c[1] + n).clamp(0., 255.) as u8,
                    (c[2] + n).clamp(0., 255.) as u8,
                    255,
                ]),
            );
        }
    }
    img
}

static CACHE: Mutex<Option<HashMap<String, Arc<RenderImage>>>> = Mutex::new(None);

fn to_render(img: RgbaImage) -> Arc<RenderImage> {
    // RenderImage expects BGRA.
    let mut img = img;
    for p in img.pixels_mut() {
        p.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(smallvec::smallvec![Frame::new(img)]))
}

/// A wallpaper matching the current theme, accent and user image.
pub fn current(custom: Option<&str>) -> Arc<RenderImage> {
    let light = crate::theme::is_light();
    let accent = crate::theme::ACCENTS[crate::theme::accent_index() as usize].1;
    let key = match custom {
        Some(p) => format!("file:{p}"),
        None => format!("aurora:{light}:{accent:06x}"),
    };
    let mut g = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    if let Some(i) = map.get(&key) {
        return i.clone();
    }
    let img = custom
        .and_then(|p| image::open(p).ok())
        .map(|i| {
            let i = i.thumbnail(1920, 1200);
            i.to_rgba8()
        })
        .unwrap_or_else(|| generate(light, accent, 720, 450));
    let out = to_render(img);
    if map.len() > 8 {
        map.clear();
    }
    map.insert(key, out.clone());
    out
}
