//! Reads back the cell isCustom flag right after a DD send.
use devhub_core::{DeviceKind, Rgb};
use std::collections::BTreeMap;
fn main() {
    let hub = devhub_hid::connect().expect("connect");
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
        .expect("dd area");
    let mut cells = BTreeMap::new();
    for i in 0..7usize {
        cells.insert(i, Rgb::new(255, 0, 0));
    }
    d.direct_drive(area, &cells).unwrap();
    println!("DD sent. reading back lighting state...");
    let st = d.read().unwrap();
    if let Some(l) = st.lighting.get(&area) {
        println!(
            "on={} effect={} per_key entries={}",
            l.on,
            l.effect,
            l.per_key.len()
        );
        for (i, rgb) in l.per_key.iter().take(12) {
            println!("  key{i} = {rgb:?}");
        }
    }
}
