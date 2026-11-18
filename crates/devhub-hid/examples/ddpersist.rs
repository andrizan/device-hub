//! Real-device check of the hybrid release:
//! 1. Stream 7 keys in red at 30fps for 5 seconds → red should appear
//! 2. Send an empty map → tracked pins are released via [5,4] → should return to the firmware animation
use devhub_core::{DeviceKind, Rgb};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

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
    let area = d
        .caps()
        .lighting
        .iter()
        .find(|a| a.direct_drive)
        .map(|a| a.id)
        .expect("direct_drive area");
    let mut cells = BTreeMap::new();
    for i in 0..7usize {
        cells.insert(i, Rgb::new(255, 0, 0));
    }
    println!("Streaming 7 keys in red for 5 seconds");
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        d.direct_drive(area, &cells).unwrap();
        std::thread::sleep(Duration::from_millis(33));
    }
    println!("Sending empty map — the 7 keys should return to firmware control (released)");
    d.direct_drive(area, &BTreeMap::new()).unwrap();
    std::thread::sleep(Duration::from_secs(8));
    println!("Done");
}
