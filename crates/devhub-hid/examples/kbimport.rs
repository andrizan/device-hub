//! Applies an export from the official configurator (hub.xsyd.top) to the real device.
//! `cargo run -p devhub-hid --example kbimport -- <keyboard_config.json>`
//! Accepts both the encrypted `{"data": ...}` and decrypted JSON
//! (DeviceHub's own `{state: DeviceState}` JSON also works).

use std::path::Path;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: kbimport <keyboard_config.json>");
        return;
    };
    let hub = match devhub_hid::connect() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("connect: {e}");
            return;
        }
    };
    let mut applied = false;
    for info in hub.scan().unwrap_or_default() {
        if !info.supported {
            continue;
        }
        let drv = match hub.driver(&info.id) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if drv.caps().keyboard.is_none() {
            continue;
        }
        println!("== {}", info.name);
        if let Ok(st) = drv.read() {
            if let Some(kb) = &st.keyboard {
                println!(
                    "   current: layers={} adv={} macros={}",
                    kb.keymap.len(),
                    kb.advanced.len(),
                    kb.macros.iter().filter(|m| m.is_some()).count()
                );
            }
        }
        match drv.run_action("import_config", Some(Path::new(&path))) {
            Ok(o) => {
                println!("{}", o.message.unwrap_or_else(|| "Done".into()));
                applied = true;
                // Read back and show the key points
                if let Ok(st) = drv.read() {
                    if let Some(kb) = &st.keyboard {
                        println!(
                            "   layers={} adv={} macros={}",
                            kb.keymap.len(),
                            kb.advanced.len(),
                            kb.macros.iter().filter(|m| m.is_some()).count()
                        );
                        for a in &kb.advanced {
                            println!("   adv key{}: {:?}", a.key, a.binding);
                        }
                    }
                    for (area, l) in &st.lighting {
                        println!(
                            "   light{area}: on={} effect={} lum={} spd={} zone={:?}",
                            l.on, l.effect, l.brightness, l.speed, l.zone_on
                        );
                    }
                }
            }
            Err(e) => eprintln!("   import: {e}"),
        }
    }
    if !applied {
        println!("No keyboard to apply to was found");
    }
}
