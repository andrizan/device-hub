//! SU66 lighting verification probe, round 2:
//!  A) Determine whether mode0 is a "static display of the selected palette slot" (slot0=rainbow/others=solid color)
//!  B) Whether effects 20-23 are accepted (write → read back)
//!  C) Whether modes 16-19 are key-reactive (have the user type and film it)
//! Restores the original LightingState (including the palette) on exit.
use devhub_core::{Change, DeviceKind, LightingState, Rgb};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

fn apply(d: &std::sync::Arc<dyn devhub_core::DeviceDriver>, s: &LightingState) {
    match d.apply(&Change::Lighting {
        area: 0,
        state: s.clone(),
    }) {
        Ok(()) => {}
        Err(e) => println!("  apply ERR {e}"),
    }
}

fn step(
    d: &std::sync::Arc<dyn devhub_core::DeviceDriver>,
    label: &str,
    s: &LightingState,
    dwell_ms: u64,
) {
    apply(d, s);
    println!("STEP {label} {}", ms());
    std::thread::sleep(Duration::from_millis(dwell_ms));
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
        "SNAPSHOT {} effect={} sel={} palette={:?}",
        ms(),
        base.effect,
        base.palette_sel,
        base.palette
    );

    let mut s = base.clone();
    s.on = true;
    s.zone_on = vec![true, true];
    s.brightness = 80;
    s.speed = 80;
    s.direction = 0;
    s.color_correction = None;

    // ---- A. Is mode0 a static palette display? ----
    // Rewrite the 8 slots to distinguishable colors (restored on exit)
    let distinct = [
        rgb(255, 0, 0),     // 0: red    (the "more/rainbow" slot in the official UI)
        rgb(255, 128, 0),   // 1: orange
        rgb(255, 255, 0),   // 2: yellow
        rgb(0, 255, 0),     // 3: green
        rgb(0, 255, 255),   // 4: cyan
        rgb(0, 0, 255),     // 5: blue
        rgb(160, 0, 255),   // 6: purple
        rgb(255, 255, 255), // 7: white
    ];
    s.effect = 0;
    s.palette = distinct.to_vec();
    for (sel, dwell) in [
        (0usize, 8000u64),
        (1, 4000),
        (3, 4000),
        (5, 4000),
        (7, 4000),
    ] {
        let mut p = s.clone();
        p.palette_sel = sel as u8;
        p.color = p.palette[sel];
        step(&d, &format!("m0_sel{sel}"), &p, dwell);
    }
    // Brightness response of the static mode (sel=1 orange)
    for bri in [20u8, 100] {
        let mut p = s.clone();
        p.palette_sel = 1;
        p.color = p.palette[1];
        p.brightness = bri;
        step(&d, &format!("m0_bri{bri}"), &p, 4000);
    }
    s.brightness = 80;

    // ---- B. Are effects 20-23 accepted? ----
    s.palette_sel = 1;
    s.color = s.palette[1];
    for m in 20u8..=23 {
        let mut p = s.clone();
        p.effect = m;
        apply(&d, &p);
        let rb = d
            .read()
            .ok()
            .and_then(|st| st.lighting.get(&0u8).map(|l| l.effect));
        println!("STEP mode{m} written rb={rb:?} {}", ms());
        std::thread::sleep(Duration::from_millis(4000));
    }

    // ---- C. Key-reaction test for modes 16-19 ----
    println!("=== TYPE_PHASE_START {} ===", ms());
    println!("*** Keep typing on the keyboard continuously for the next 80 seconds ***");
    for m in [16u8, 17, 18, 19] {
        let mut p = s.clone();
        p.effect = m;
        apply(&d, &p);
        println!("REACT mode{m} {}", ms());
        std::thread::sleep(Duration::from_millis(20000));
    }
    println!("=== TYPE_PHASE_END {} ===", ms());

    println!("RESTORE {}", ms());
    apply(&d, &base);
    println!("DONE {}", ms());
}
