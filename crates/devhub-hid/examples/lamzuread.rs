//! Real-device verification of the LAMZU driver path — lamzu::open → caps → read → apply (read back) → restore.
//! `cargo run -p devhub-hid --example lamzuread`
//!
//! Maya X V2 / LM20 wired (37b0:0044) only. Round trips are only the safe kind that restore every original value.

use devhub_core::backend::DeviceDriver;
use devhub_core::model::*;
use devhub_hid::lamzu;
use devhub_hid::transport;

fn main() {
    let api = transport::open().expect("hidapi");
    // Prefer wired 0044, otherwise the Orbital 8K dongle 0048 (same feature path).
    let info_raw = api
        .device_list()
        .find(|d| {
            d.vendor_id() == 0x37b0
                && (d.product_id() == 0x0044 || d.product_id() == 0x0048)
                && d.usage_page() == 0xffff
                && d.usage() == 0
        })
        .expect("LAMZU FFFF:00 collection");
    let dev = info_raw.open_device(&api).expect("open");
    let info = DeviceInfo {
        id: DeviceId("lamzu-lm20".into()),
        kind: DeviceKind::Mouse,
        name: "LAMZU?".into(),
        vendor: "LAMZU".into(),
        model: "maya-x-v2".into(),
        vid: 0x37b0,
        pid: info_raw.product_id(),
        transport: if info_raw.product_id() == 0x0048 {
            Transport::Wireless24G
        } else {
            Transport::Wired
        },
        firmware: None,
        dongle_firmware: None,
        serial: None,
        driver: DriverKind::Lamzu,
        supported: false,
        asleep: false,
        battery: None,
        board_id: None,
    };
    let d = lamzu::open(dev, info, None).expect("driver");
    let i = d.info();
    println!(
        "== info: fw={:?} serial={:?} bat={:?}",
        i.firmware, i.serial, i.battery
    );

    let c = d.caps();
    println!(
        "== caps: settings={} buttons={}",
        c.settings.len(),
        c.mouse.as_ref().map(|m| m.buttons.len()).unwrap_or(0)
    );

    let s = d.read().expect("read");
    let ms = s.mouse.as_ref().unwrap();
    println!("== profile: active={}", s.profile.active);
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

    // Toggle round trip: flip motion_sync → read back → restore.
    {
        let id = "motion_sync";
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

    // Polling: 1000Hz → read back → restore.
    if let Some(SettingValue::Choice(cur)) = s.settings.get("polling_wireless") {
        let cur = *cur;
        println!("== apply polling {cur} -> 1000");
        d.apply(&Change::Setting {
            id: "polling_wireless".into(),
            value: SettingValue::Choice(1000),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = match s2.settings.get("polling_wireless") {
            Some(SettingValue::Choice(v)) => *v,
            _ => -1,
        };
        println!("   readback: {now}");
        assert_eq!(now, 1000);
        d.apply(&Change::Setting {
            id: "polling_wireless".into(),
            value: SettingValue::Choice(cur),
        })
        .expect("restore");
    }

    // LOD: 1.0mm (Choice 10) → read back → restore.
    if let Some(SettingValue::Choice(cur)) = s.settings.get("lod") {
        let cur = *cur;
        println!("== apply lod {cur} -> 10 (1.0mm)");
        d.apply(&Change::Setting {
            id: "lod".into(),
            value: SettingValue::Choice(10),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = match s2.settings.get("lod") {
            Some(SettingValue::Choice(v)) => *v,
            _ => -1,
        };
        println!("   readback: {now}");
        assert_eq!(now, 10);
        d.apply(&Change::Setting {
            id: "lod".into(),
            value: SettingValue::Choice(cur),
        })
        .expect("restore");
    }

    // Debounce: ±1 → read back → restore.
    if let Some(SettingValue::Num(cur)) = s.settings.get("debounce") {
        let t = if *cur < 20.0 { *cur + 1.0 } else { *cur - 1.0 };
        println!("== apply debounce {cur} -> {t}");
        d.apply(&Change::Setting {
            id: "debounce".into(),
            value: SettingValue::Num(t),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = match s2.settings.get("debounce") {
            Some(SettingValue::Num(v)) => *v,
            _ => -1.0,
        };
        println!("   readback: {now}");
        assert_eq!(now, t);
        d.apply(&Change::Setting {
            id: "debounce".into(),
            value: SettingValue::Num(*cur),
        })
        .expect("restore");
    }

    // Active stage: 0 → read back → restore.
    let orig_stage = ms.active_stage;
    let target = if orig_stage == 0 { 1 } else { 0 };
    println!("== apply active_stage {orig_stage} -> {target}");
    d.apply(&Change::ActiveStage(target)).expect("apply");
    let s2 = d.read().expect("read2");
    assert_eq!(s2.mouse.as_ref().unwrap().active_stage, target);
    println!("   readback: {target}");
    d.apply(&Change::ActiveStage(orig_stage)).expect("restore");

    // Side button (slot3): Key('a' usage=4) → read back → restore.
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

    // ---- Extended round trips ----------------------------------------------------------
    // Flip all toggles together → read back → restore.
    for id in [
        "hyper_mode",
        "tracking_mode",
        "dpi_xy",
        "angle_snap",
        "ripple",
        "rapid_right",
        "rapid_left",
        // Exists only via the dongle (automatically skipped when wired).
        "dongle_led",
        "glass_mode",
    ] {
        let Some(SettingValue::Bool(cur)) = s.settings.get(id) else {
            continue;
        };
        println!("== apply {id} {} -> {}", cur, !cur);
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
            value: SettingValue::Bool(*cur),
        })
        .expect("restore");
    }

    // Numeric: sensor_offset ±1, angle_tune +5, B-hop ms +200.
    for id in ["sensor_offset", "angle_tune", "bhop_ms"] {
        if let Some(SettingValue::Num(cur)) = s.settings.get(id) {
            let t = match id {
                "sensor_offset" => {
                    if *cur < 5.0 {
                        *cur + 1.0
                    } else {
                        *cur - 1.0
                    }
                }
                "angle_tune" => 5.0,
                _ => {
                    if *cur < 1000.0 {
                        *cur + 200.0
                    } else {
                        *cur - 200.0
                    }
                }
            };
            println!("== apply {id} {cur} -> {t}");
            d.apply(&Change::Setting {
                id: id.into(),
                value: SettingValue::Num(t),
            })
            .expect("apply");
            let s2 = d.read().expect("read2");
            let now = match s2.settings.get(id) {
                Some(SettingValue::Num(v)) => *v,
                _ => f32::NAN,
            };
            println!("   readback: {now}");
            assert_eq!(now, t, "{id} roundtrip failed");
            d.apply(&Change::Setting {
                id: id.into(),
                value: SettingValue::Num(*cur),
            })
            .expect("restore");
        }
    }

    // Sleep: 5 min → read back → restore.
    if let Some(SettingValue::Choice(cur)) = s.settings.get("sleep_min") {
        let cur = *cur;
        println!("== apply sleep_min {cur} -> 5");
        d.apply(&Change::Setting {
            id: "sleep_min".into(),
            value: SettingValue::Choice(5),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = match s2.settings.get("sleep_min") {
            Some(SettingValue::Choice(v)) => *v,
            _ => -1,
        };
        println!("   readback: {now}");
        assert_eq!(now, 5);
        d.apply(&Change::Setting {
            id: "sleep_min".into(),
            value: SettingValue::Choice(cur),
        })
        .expect("restore");
    }

    // Dongle LED mode (wireless only): led1 → another mode → restore.
    for id in ["dongle_led1", "dongle_led2", "dongle_led3"] {
        if let Some(SettingValue::Choice(cur)) = s.settings.get(id) {
            let cur = *cur;
            let t = if cur == 6 { 11 } else { 6 };
            println!("== apply {id} {cur} -> {t}");
            d.apply(&Change::Setting {
                id: id.into(),
                value: SettingValue::Choice(t),
            })
            .expect("apply");
            let s2 = d.read().expect("read2");
            let now = match s2.settings.get(id) {
                Some(SettingValue::Choice(v)) => *v,
                _ => -1,
            };
            println!("   readback: {now}");
            assert_eq!(now, t);
            d.apply(&Change::Setting {
                id: id.into(),
                value: SettingValue::Choice(cur),
            })
            .expect("restore");
        }
    }

    // AdaDPI curve: change the last point slightly and write → read back → restore.
    if let Some(cur) = s.mouse.as_ref().and_then(|m| m.adaptive.clone()) {
        let mut tmp = cur.clone();
        if let Some(p) = tmp.points.last_mut() {
            p.speed = if p.speed == 40 { 41 } else { 40 };
        }
        println!("== apply adaptive curve ({} points)", tmp.points.len());
        d.apply(&Change::Adaptive(tmp)).expect("adaptive apply");
        let s2 = d.read().expect("read2");
        let back = s2.mouse.as_ref().and_then(|m| m.adaptive.clone());
        println!("   readback: {:?}", back.as_ref().map(|c| c.points.len()));
        assert!(back.is_some());
        d.apply(&Change::Adaptive(cur)).expect("adaptive restore");
    }

    // B-hop direction: 0 (OFF) → 2 (up) → restore.
    if let Some(SettingValue::Choice(cur)) = s.settings.get("bhop_dir") {
        let cur = *cur;
        let t = if cur == 0 { 2 } else { 0 };
        println!("== apply bhop_dir {cur} -> {t}");
        d.apply(&Change::Setting {
            id: "bhop_dir".into(),
            value: SettingValue::Choice(t),
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let now = match s2.settings.get("bhop_dir") {
            Some(SettingValue::Choice(v)) => *v,
            _ => -1,
        };
        println!("   readback: {now}");
        assert_eq!(now, t);
        d.apply(&Change::Setting {
            id: "bhop_dir".into(),
            value: SettingValue::Choice(cur),
        })
        .expect("restore");
    }

    // DPI stages in bulk: write the same list back (E2E of the sub26 path).
    if !ms.stages.is_empty() {
        println!("== apply dpi stages (same {} stages)", ms.stages.len());
        d.apply(&Change::DpiStages {
            stages: ms.stages.clone(),
            active: ms.active_stage,
        })
        .expect("apply");
        let s2 = d.read().expect("read2");
        let st2 = &s2.mouse.as_ref().unwrap().stages;
        println!("   readback: {} stages, first={}", st2.len(), st2[0].x);
        assert_eq!(st2.len(), ms.stages.len());
        assert_eq!(st2[0].x, ms.stages[0].x);
    }

    // Profile switch: p1→p2→p1 (run last — it changes the settings being read).
    println!("== apply profile 0 -> 1");
    d.apply(&Change::Profile(1)).expect("apply");
    let s2 = d.read().expect("read2");
    println!("   readback: active={}", s2.profile.active);
    assert_eq!(s2.profile.active, 1);
    d.apply(&Change::Profile(0)).expect("restore");
    let s3 = d.read().expect("read3");
    assert_eq!(s3.profile.active, 0);
    println!("   restored to profile 0");
    println!("OK");
}
