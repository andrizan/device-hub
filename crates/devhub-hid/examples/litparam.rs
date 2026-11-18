//! SU66 lighting-parameter sweep — writes individual parameter changes to the real device in order and
//! prints each step's start time in milliseconds to stdout. By matching it against continuously filmed camera footage
//! via timestamps, the real-device behavior of zone/palette/brightness/direction/speed/color correction
//! is classified. Restores the original LightingState on exit.
use devhub_core::{Change, DeviceKind, LightingState};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

const STEP_MS: u64 = 4000;

fn step(d: &std::sync::Arc<dyn devhub_core::DeviceDriver>, label: &str, s: &LightingState) {
    let t = ms();
    match d.apply(&Change::Lighting {
        area: 0,
        state: s.clone(),
    }) {
        Ok(()) => println!("STEP {label} OK {t}"),
        Err(e) => println!("STEP {label} ERR {t} {e}"),
    }
    std::thread::sleep(Duration::from_millis(STEP_MS));
}

fn main() {
    let hub = match devhub_hid::connect() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("connect: {e}");
            return;
        }
    };
    let info = hub
        .scan()
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.kind == DeviceKind::Keyboard && d.supported)
        .expect("SU66");
    let d = hub.driver(&info.id).expect("driver");
    let st0 = d.read().expect("initial read");
    let base = st0.lighting.get(&0u8).cloned().expect("lighting area0");
    println!(
        "SNAPSHOT {} effect={} bri={} spd={} dir={} sel={} zone={:?} cc={:?} palette={:?}",
        ms(),
        base.effect,
        base.brightness,
        base.speed,
        base.direction,
        base.palette_sel,
        base.zone_on,
        base.color_correction,
        base.palette
    );

    let mut s = base.clone();
    s.on = true;
    s.zone_on = vec![true, true];
    // Match each slot's own color to palette[sel] (F4: avoids the slot being overwritten when sel changes)
    let keep_slot = |s: &mut LightingState| {
        let sel = s.palette_sel as usize;
        if let Some(c) = s.palette.get(sel).cloned() {
            s.color = c;
        }
    };

    // ---- A. Zone check (effect=8 uniform, sel kept) ----
    s.effect = 8;
    s.brightness = 80;
    s.speed = 80;
    s.direction = 0;
    keep_slot(&mut s);
    for (label, zones) in [
        ("zone_none", vec![false, false]),
        ("zone_lower", vec![false, true]),
        ("zone_upper", vec![true, false]),
        ("zone_both", vec![true, true]),
    ] {
        let mut z = s.clone();
        z.zone_on = zones;
        step(&d, label, &z);
    }
    s.zone_on = vec![true, true];

    // ---- B. Palette sel 0..7 (effect=8) ----
    for sel in 0..8u8 {
        let mut p = s.clone();
        p.palette_sel = sel;
        keep_slot(&mut p);
        step(&d, &format!("pal{sel}"), &p);
    }
    s.palette_sel = 0;
    keep_slot(&mut s);

    // ---- C. Color response of effect 0 (sel 0 vs sel 5) ----
    for sel in [0u8, 5] {
        let mut p = s.clone();
        p.effect = 0;
        p.palette_sel = sel;
        keep_slot(&mut p);
        step(&d, &format!("m0pal{sel}"), &p);
    }

    // ---- D. Brightness sweep (effect=8) ----
    for bri in [0u8, 20, 40, 60, 100] {
        let mut p = s.clone();
        p.brightness = bri;
        step(&d, &format!("bri{bri}"), &p);
    }
    s.brightness = 80;

    // ---- E. Direction (effect=11 sine) ----
    for dir in [0u8, 1] {
        let mut p = s.clone();
        p.effect = 11;
        p.direction = dir;
        step(&d, &format!("dir{dir}"), &p);
    }
    s.direction = 0;

    // ---- F. Speed (effect=10 shift) ----
    for spd in [5u8, 50, 100] {
        let mut p = s.clone();
        p.effect = 10;
        p.speed = spd;
        step(&d, &format!("spd{spd}"), &p);
    }
    s.speed = 80;

    // ---- G. Color correction (effect=8) ----
    for (label, cc) in [
        ("cc_red", [100u8, 0, 0]),
        ("cc_grn", [0, 100, 0]),
        ("cc_blu", [0, 0, 100]),
        ("cc_off", [0, 0, 0]),
    ] {
        let mut p = s.clone();
        p.effect = 8;
        p.color_correction = Some(devhub_core::Rgb {
            r: cc[0],
            g: cc[1],
            b: cc[2],
        });
        step(&d, label, &p);
    }

    println!("RESTORE {}", ms());
    if let Err(e) = d.apply(&Change::Lighting {
        area: 0,
        state: base,
    }) {
        println!("RESTORE ERR {e}");
    }
    println!("DONE {}", ms());
}
