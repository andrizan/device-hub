//! Real-device verification of the ATK driver path — atk::open → caps → read → apply (read back) → restore.
//! `cargo run -p devhub-hid --example atkread`

use devhub_core::backend::DeviceDriver;
use devhub_core::model::*;
use devhub_hid::atk;
use devhub_hid::transport;

fn main() {
    let api = transport::open().expect("hidapi");
    // Prefer wired 12a7, otherwise the dongle 1278 (same 16B relay protocol).
    let info_raw = api
        .device_list()
        .find(|d| {
            d.vendor_id() == 0x373b
                && (d.product_id() == 0x12a7 || d.product_id() == 0x1278)
                && d.usage_page() == 0xff02
                && d.usage() == 2
        })
        .expect("ATK ff02/u2 collection");
    let dev = info_raw.open_device(&api).expect("open");
    let info = DeviceInfo {
        id: DeviceId("atk-f1v3u".into()),
        kind: DeviceKind::Mouse,
        name: "ATK?".into(),
        vendor: "ATK".into(),
        model: "atk-f1v3u".into(),
        vid: 0x373b,
        pid: 0x12a7,
        transport: Transport::Wired,
        firmware: None,
        dongle_firmware: None,
        serial: None,
        driver: DriverKind::Atk,
        supported: false,
        asleep: false,
        battery: None,
        board_id: None,
    };
    let d = atk::open(dev, info, None).expect("driver");
    // The mouse may be asleep behind the dongle — wait for it to wake via the info request.
    let mut i = d.info();
    for _ in 0..15 {
        if i.firmware.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        i = d.info();
    }
    println!(
        "== info: name={} fw={:?} bat={:?}",
        i.name, i.firmware, i.battery
    );

    let c = d.caps();
    println!(
        "== caps: settings={} buttons={}",
        c.settings.len(),
        c.mouse.as_ref().map(|m| m.buttons.len()).unwrap_or(0)
    );

    let s = d.read().expect("read");
    let ms = s.mouse.as_ref().unwrap();
    println!("== stages: active={}", ms.active_stage);
    for (k, st) in ms.stages.iter().enumerate() {
        println!("   {k}: {}x{:?} rgb={:?}", st.x, st.y, st.color);
    }
    println!("== buttons:");
    for (k, b) in ms.buttons.iter().enumerate() {
        println!("   {k}: {b:?}");
    }
    println!("== settings:");
    for (k, v) in &s.settings {
        println!("   {k} = {v:?}");
    }

    // Safe write round trip: flip the toggles → read back → restore.
    for id in ["motion_sync", "move_close_led"] {
        let cur = matches!(s.settings.get(id), Some(SettingValue::Bool(true)));
        println!("== apply {id} -> {}", !cur);
        d.apply(&Change::Setting {
            id: id.into(),
            value: SettingValue::Bool(!cur),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = matches!(s2.settings.get(id), Some(SettingValue::Bool(true)));
        println!("   readback: {now} (expect {})", !cur);
        assert_eq!(now, !cur, "{id} roundtrip failed");
        d.apply(&Change::Setting {
            id: id.into(),
            value: SettingValue::Bool(cur),
        })
        .expect("restore");
    }

    // Numeric: debounce +1 → read back → restore.
    for id in ["debounce", "wheel_debounce", "bhop"] {
        if let Some(SettingValue::Num(cur)) = s.settings.get(id) {
            let t = if *cur < 20.0 { *cur + 1.0 } else { *cur - 1.0 };
            println!("== apply {id} {cur} -> {t}");
            d.apply(&Change::Setting {
                id: id.into(),
                value: SettingValue::Num(t),
            })
            .expect("apply");
            let s2 = d.read().expect("read2");
            let now = match s2.settings.get(id) {
                Some(SettingValue::Num(v)) => *v,
                _ => -1.0,
            };
            println!("   readback: {now}");
            assert_eq!(now, t);
            d.apply(&Change::Setting {
                id: id.into(),
                value: SettingValue::Num(*cur),
            })
            .expect("restore");
        }
    }

    // Sleep: set to 2 min → read back → restore.
    if let Some(SettingValue::Choice(cur)) = s.settings.get("sleep_min") {
        let cur = *cur;
        println!("== apply sleep_min {cur} -> 2");
        d.apply(&Change::Setting {
            id: "sleep_min".into(),
            value: SettingValue::Choice(2),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = match s2.settings.get("sleep_min") {
            Some(SettingValue::Choice(v)) => *v,
            _ => -1,
        };
        println!("   readback: {now}");
        assert_eq!(now, 2);
        d.apply(&Change::Setting {
            id: "sleep_min".into(),
            value: SettingValue::Choice(cur),
        })
        .expect("restore");
    }

    // LOD: set to 1.2mm (Choice 12) → read back → restore.
    if let Some(SettingValue::Choice(cur)) = s.settings.get("lod") {
        let cur = *cur;
        println!("== apply lod {cur} -> 12 (1.2mm)");
        d.apply(&Change::Setting {
            id: "lod".into(),
            value: SettingValue::Choice(12),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = match s2.settings.get("lod") {
            Some(SettingValue::Choice(v)) => *v,
            _ => -1,
        };
        println!("   readback: {now}");
        assert_eq!(now, 12);
        d.apply(&Change::Setting {
            id: "lod".into(),
            value: SettingValue::Choice(cur),
        })
        .expect("restore");
    }

    // Side button ShortcutKey: write Key('a' usage=4) → read back → restore the original action.
    if let Some(orig) = ms.buttons.get(3).cloned() {
        let a = ButtonAction::Key(KeyStroke { mods: 0, usage: 4 });
        println!("== apply button3 -> {a:?}");
        d.apply(&Change::Button { slot: 3, action: a })
            .expect("apply");
        let s2 = d.read().expect("read2");
        println!("   readback: {:?}", s2.mouse.as_ref().unwrap().buttons[3]);
        d.apply(&Change::Button {
            slot: 3,
            action: orig,
        })
        .expect("restore");
    }
    println!("OK");
}
