//! HTS Plus Pro wired (33e4:5219) E2E: EEPROM dump + write round trip.
//! Usage:
//!   cargo run -p devhub-hid --example htse2e -- dump     # dump the relevant regions
//!   cargo run -p devhub-hid --example htse2e -- e2e      # write round-trip test (mouse must be awake)
use devhub_hid::transport;
use std::time::{Duration, Instant};

fn cmd(dev: &hidapi::HidDevice, c: u8, addr: u16, data: &[u8]) -> Vec<u8> {
    let mut f = [0u8; 17];
    f[0] = 8;
    f[1] = c;
    f[3] = (addr >> 8) as u8;
    f[4] = addr as u8;
    f[5] = data.len().min(10) as u8;
    f[6..6 + data.len().min(10)].copy_from_slice(&data[..data.len().min(10)]);
    let s: u32 = f[..16].iter().map(|&b| b as u32).sum();
    f[16] = 85u32.wrapping_sub(s) as u8;
    dev.write(&f).unwrap();
    let dl = Instant::now() + Duration::from_millis(600);
    loop {
        let mut b = [0u8; 64];
        match dev.read_timeout(&mut b, 80) {
            Ok(n) if n >= 17 && b[1] == c => return b[..17].to_vec(),
            _ => {
                if Instant::now() > dl {
                    return vec![];
                }
            }
        }
    }
}

/// EEPROM read (concatenates 10B chunks). Response data is b[6..].
fn ee_read(dev: &hidapi::HidDevice, addr: u16, len: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut a = addr;
    while (out.len() as u16) < len {
        let want = (len - out.len() as u16).min(10);
        let r = cmd(dev, 8, a, &vec![0u8; want as usize]);
        if r.len() < 17 {
            println!("  ee_read @{a}: TIMEOUT");
            break;
        }
        out.extend_from_slice(&r[6..6 + want as usize]);
        a += want;
    }
    out
}

/// EEPROM write (10B chunks).
fn ee_write(dev: &hidapi::HidDevice, addr: u16, data: &[u8]) {
    for (i, chunk) in data.chunks(10).enumerate() {
        let a = addr + (i * 10) as u16;
        let r = cmd(dev, 7, a, chunk);
        if r.len() < 17 {
            println!("  ee_write @{a}: TIMEOUT");
        }
        std::thread::sleep(Duration::from_millis(12));
    }
}

fn ee_u8(dev: &hidapi::HidDevice, addr: u16) -> Option<u8> {
    let b = ee_read(dev, addr, 2);
    if b.len() >= 2 && (b[0] as u16 + b[1] as u16) & 0xff == 85 {
        Some(b[0])
    } else {
        None
    }
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn dump(dev: &hidapi::HidDevice) {
    println!("=== FW ===");
    let r = cmd(dev, 18, 0, &[]);
    println!("  cmd18: {:?}", r);
    if r.len() >= 17 {
        println!("    -> display: {:x}.{:02x}", r[5], r[6]);
    }
    let r = cmd(dev, 29, 0, &[]);
    println!("  cmd29: {:?}", r);
    if r.len() >= 17 {
        println!("    -> display: {:x}.{:02x}", r[6], r[7]);
    }
    println!("=== cmd23 (long_range get) ===");
    println!("  {:?}", cmd(dev, 23, 0, &[]));
    println!("=== cmd4 (battery) ===");
    println!("  {:?}", cmd(dev, 4, 0, &[]));
    println!("=== cmd14 (profile) ===");
    println!("  {:?}", cmd(dev, 14, 0, &[]));
    println!("=== KeyFunction @96 (5 slots x 4B) ===");
    let kf = ee_read(dev, 96, 20);
    for i in 0..5 {
        println!("  slot{i}: [{}]", hex(&kf[i * 4..(i + 1) * 4]));
    }
    println!("=== Magnetic @239..250 ===");
    for addr in [239u16, 241, 243, 245, 247, 249] {
        println!("  @{addr}: {:?}", ee_read(dev, addr, 2));
    }
    println!("=== ShortcutKey @256+32k (raw for type5 slots only) ===");
    for i in 0..5u16 {
        if kf[i as usize * 4] == 5 {
            let sc = ee_read(dev, 256 + i * 32, 32);
            println!("  slot{i} @{}: [{}]", 256 + i * 32, hex(&sc));
        }
    }
    println!("=== KeyOperation @8 ===");
    println!("  {:?}", ee_read(dev, 8, 2));
}

fn main() {
    let api = transport::open().unwrap();
    let d = api
        .device_list()
        .find(|x| {
            x.vendor_id() == 0x33e4
                && x.product_id() == 0x5219
                && x.usage_page() == 0xff02
                && x.usage() == 2
        })
        .unwrap()
        .open_device(&api)
        .unwrap();
    d.set_blocking_mode(false).ok();

    match std::env::args().nth(1).as_deref() {
        Some("e2e") => e2e(&d),
        Some("drv") => drv(),
        _ => dump(&d),
    }
}

/// E2E through the driver path (RealHub→driver→read/apply). Verifies including encode/RMW.
fn drv() {
    use devhub_core::model::*;
    let hub = devhub_hid::connect().unwrap();
    let devs = hub.scan().unwrap();
    let info = devs
        .iter()
        .find(|d| d.model == "hts-plus-pro")
        .expect("HTS Plus Pro wired not found");
    println!("found: {} {}", info.name, info.id.0);
    let drv = hub.driver(&info.id).unwrap();

    let st0 = drv.read().unwrap();
    let get = |st: &DeviceState, id: &str| st.settings.get(id).cloned();
    let orig: Vec<(String, SettingValue)> = [
        "ms_trigger_left",
        "ms_trigger_right",
        "ms_fast_left_en",
        "ms_fast_left",
        "ms_fast_right_en",
        "ms_fast_right",
    ]
    .iter()
    .filter_map(|id| get(&st0, id).map(|v| (id.to_string(), v)))
    .collect();
    let orig_btn3 = st0.mouse.as_ref().unwrap().buttons[3].clone();
    println!("baseline settings: {orig:?}\nbaseline slot3: {orig_btn3:?}");

    let mut fails = 0;
    macro_rules! check {
        ($name:expr, $cond:expr) => {
            if $cond {
                println!("  [OK] {}", $name);
            } else {
                fails += 1;
                println!("  [FAIL] {}", $name);
            }
        };
    }

    // ---- RT: enable toggle + sensitivity RMW ----
    println!("== RT (ms_fast) ==");
    drv.apply(&Change::Setting {
        id: "ms_fast_left_en".into(),
        value: SettingValue::Bool(true),
    })
    .unwrap();
    let st = drv.read().unwrap();
    check!(
        "en=true readback",
        get(&st, "ms_fast_left_en") == Some(SettingValue::Bool(true))
    );
    check!(
        "value preserved on enable",
        get(&st, "ms_fast_left") == get(&st0, "ms_fast_left")
    );
    drv.apply(&Change::Setting {
        id: "ms_fast_left".into(),
        value: SettingValue::Num(7.),
    })
    .unwrap();
    let st = drv.read().unwrap();
    check!(
        "value=7 readback",
        get(&st, "ms_fast_left") == Some(SettingValue::Num(7.))
    );
    check!(
        "en still true after value write",
        get(&st, "ms_fast_left_en") == Some(SettingValue::Bool(true))
    );
    drv.apply(&Change::Setting {
        id: "ms_fast_left_en".into(),
        value: SettingValue::Bool(false),
    })
    .unwrap();
    let st = drv.read().unwrap();
    check!(
        "disable keeps value 7 (equivalent to official 0x07)",
        get(&st, "ms_fast_left") == Some(SettingValue::Num(7.))
            && get(&st, "ms_fast_left_en") == Some(SettingValue::Bool(false))
    );

    // ---- Trigger position: official shows 1-20 ----
    println!("== ms_trigger (1-20 display) ==");
    let t0 = get(&st0, "ms_trigger_left");
    drv.apply(&Change::Setting {
        id: "ms_trigger_left".into(),
        value: SettingValue::Num(10.),
    })
    .unwrap();
    let st = drv.read().unwrap();
    check!(
        "set 10 → readback 10",
        get(&st, "ms_trigger_left") == Some(SettingValue::Num(10.))
    );
    if let Some(v) = t0 {
        drv.apply(&Change::Setting {
            id: "ms_trigger_left".into(),
            value: v,
        })
        .unwrap();
    }

    // ---- Side button: Media (ShortcutKey + CRC) ----
    println!("== side button slot3 ==");
    drv.apply(&Change::Button {
        slot: 3,
        action: ButtonAction::Media(MediaFn::PlayPause),
    })
    .unwrap();
    let st = drv.read().unwrap();
    let b3 = st.mouse.as_ref().unwrap().buttons[3].clone();
    check!(
        "slot3 = Media(PlayPause)",
        b3 == ButtonAction::Media(MediaFn::PlayPause)
    );
    // Key + modifiers (Ctrl+Shift+A)
    drv.apply(&Change::Button {
        slot: 3,
        action: ButtonAction::Key(KeyStroke {
            mods: mods::LCTRL | mods::LSHIFT,
            usage: 4,
        }),
    })
    .unwrap();
    let st = drv.read().unwrap();
    let b3 = st.mouse.as_ref().unwrap().buttons[3].clone();
    check!(
        "slot3 = Ctrl+Shift+A",
        matches!(b3, ButtonAction::Key(k) if k.mods == 0b11 && k.usage == 4)
    );
    // Mouse "Back" (type1 — the shortcut region remains but is disabled via type)
    drv.apply(&Change::Button {
        slot: 3,
        action: ButtonAction::Mouse(MouseFn::Back),
    })
    .unwrap();
    let st = drv.read().unwrap();
    check!(
        "slot3 = Mouse Back",
        st.mouse.as_ref().unwrap().buttons[3] == ButtonAction::Mouse(MouseFn::Back)
    );

    // ---- Restore ----
    println!("== restore ==");
    drv.apply(&Change::Button {
        slot: 3,
        action: orig_btn3,
    })
    .unwrap();
    for (id, v) in orig {
        drv.apply(&Change::Setting { id, value: v }).unwrap();
    }
    let st = drv.read().unwrap();
    println!(
        "restored settings: {:?}",
        st.settings
            .iter()
            .filter(|(k, _)| k.starts_with("ms_"))
            .collect::<Vec<_>>()
    );
    println!(
        "restored slot3: {:?}",
        st.mouse.as_ref().unwrap().buttons[3]
    );
    println!("== {} ==", if fails == 0 { "ALL PASS" } else { "FAILURES" });
}

/// Write round-trip test. Save current values → write → read back → restore.
fn e2e(dev: &hidapi::HidDevice) {
    println!("=== baseline ===");
    dump(dev);

    // Regions to save: KeyFunction + shortcut of the side buttons (slot3/4), the 4 magnetic buttons
    let kf = ee_read(dev, 96, 20);
    let mut saved_short: Vec<(u16, Vec<u8>)> = vec![];
    for i in 0..5u16 {
        if kf[i as usize * 4] == 5 {
            saved_short.push((256 + i * 32, ee_read(dev, 256 + i * 32, 32)));
        }
    }
    let mag: Vec<(u16, Vec<u8>)> = [239u16, 241, 245, 247]
        .iter()
        .map(|&a| (a, ee_read(dev, a, 2)))
        .collect();

    // ---- RT (ms_fast) write round trip ----
    println!("\n=== RT e2e (@241/247) ===");
    for (addr, name) in [(241u16, "L"), (247u16, "R")] {
        for wire in [0x83u8, 0x00, 0x87] {
            ee_write(dev, addr, &[wire, 85u8.wrapping_sub(wire)]);
            let rb = ee_u8(dev, addr);
            let st = if rb == Some(wire) { "OK" } else { "FAIL" };
            println!("  {name} @{addr} write {wire:#04x} -> read {rb:?} [{st}]");
        }
    }

    // ---- Side button: write Media PlayPause to slot3 (Back) and round-trip ----
    println!("\n=== side-button e2e (slot3) ===");
    let media: Vec<u8> = {
        let mut b = vec![2, 0x82, 0xCD, 0x00, 0x42, 0xCD, 0x00, 0];
        let crc = 85u32.wrapping_sub(b[..7].iter().map(|&x| x as u32).sum::<u32>()) as u8;
        b[7] = crc;
        b
    };
    ee_write(dev, 256 + 3 * 32, &media);
    std::thread::sleep(Duration::from_millis(50));
    ee_write(
        dev,
        96 + 3 * 4,
        &[5, 0, 0, {
            let r = [5u8, 0, 0];
            85u8.wrapping_sub(r.iter().map(|&x| x as u32).sum::<u32>() as u8)
        }],
    );
    let rb = ee_read(dev, 256 + 3 * 32, 8);
    println!("  shortcut write -> read [{}]", hex(&rb));
    let kf3 = ee_read(dev, 96 + 3 * 4, 4);
    println!("  keyfn slot3 -> [{}]", hex(&kf3));

    // ---- Restore ----
    println!("\n=== restore ===");
    ee_write(dev, 96, &kf);
    for (a, b) in &saved_short {
        ee_write(dev, *a, b);
    }
    for (a, b) in &mag {
        ee_write(dev, *a, b);
    }
    println!("restored. verify:");
    dump(dev);
}
