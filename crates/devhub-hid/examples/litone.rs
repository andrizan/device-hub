//! Minimal probe that holds a single mode for the given number of seconds: `litone <mode> <sec>`
//! Restores the original LightingState on exit.
use devhub_core::{Change, DeviceKind};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn main() {
    let mode: u8 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(16);
    let secs: u64 = std::env::args()
        .nth(2)
        .and_then(|a| a.parse().ok())
        .unwrap_or(40);
    let hub = devhub_hid::connect().expect("connect");
    let info = hub
        .scan()
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.kind == DeviceKind::Keyboard && d.supported)
        .expect("SU66");
    let d = hub.driver(&info.id).expect("driver");
    let base = d
        .read()
        .expect("read")
        .lighting
        .get(&0u8)
        .cloned()
        .expect("lighting");
    let mut s = base.clone();
    s.on = true;
    s.zone_on = vec![true, true];
    s.brightness = 80;
    s.speed = 80;
    s.effect = mode;
    println!("SET mode={mode} {}", ms());
    let _ = d.apply(&Change::Lighting { area: 0, state: s });
    std::thread::sleep(Duration::from_secs(secs));
    println!("RESTORE {}", ms());
    let _ = d.apply(&Change::Lighting {
        area: 0,
        state: base,
    });
    println!("DONE {}", ms());
}
