//! Lighting E2E: verifies read → write → readback → restore on the real device.
//! Official wire layout (r[4]=open, r[5]=mode, r[6]=brightness, r[7]=speed, r[8]=direction,
//! r[9]=palette selection) takes effect correctly. Restores the original state at the end.

use devhub_core::*;

fn main() {
    let hub = match devhub_hid::connect() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("connect: {e}");
            return;
        }
    };
    let list = hub.scan().unwrap_or_default();
    for info in &list {
        if !info.supported {
            continue;
        }
        let drv = match hub.driver(&info.id) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: driver: {e}", info.name);
                continue;
            }
        };
        let caps = drv.caps();
        // --- perf mode byte (continuous RT) round trip — verified on key 0, then restored ---
        if caps.keyboard.as_ref().is_some_and(|k| k.perf.is_some()) {
            if let Some(kst) = drv.read().ok().and_then(|s| s.keyboard) {
                if let Some(orig_p) = kst.perf.first().copied() {
                    print!("   perf : ");
                    let mut tp = orig_p;
                    tp.mode = TriggerMode::Rapid;
                    tp.continuous = !tp.continuous;
                    let ok1 = drv
                        .apply(&Change::KeyPerf {
                            keys: vec![0],
                            perf: tp,
                        })
                        .is_ok()
                        && drv
                            .read()
                            .ok()
                            .and_then(|s| s.keyboard?.perf.first().copied())
                            .is_some_and(|b| {
                                b.mode == TriggerMode::Rapid && b.continuous == tp.continuous
                            });
                    // Round-trip the non-continuous side (mode=2) as well
                    tp.continuous = !tp.continuous;
                    let ok2 = drv
                        .apply(&Change::KeyPerf {
                            keys: vec![0],
                            perf: tp,
                        })
                        .is_ok()
                        && drv
                            .read()
                            .ok()
                            .and_then(|s| s.keyboard?.perf.first().copied())
                            .is_some_and(|b| b.continuous == tp.continuous);
                    let ok3 = drv
                        .apply(&Change::KeyPerf {
                            keys: vec![0],
                            perf: orig_p,
                        })
                        .is_ok()
                        && drv
                            .read()
                            .ok()
                            .and_then(|s| s.keyboard?.perf.first().copied())
                            .is_some_and(|b| b == orig_p);
                    println!(
                        "RT+continuous={} RT+non-continuous={} restore={}",
                        if ok1 { "OK" } else { "FAIL" },
                        if ok2 { "OK" } else { "FAIL" },
                        if ok3 { "OK" } else { "FAIL" },
                    );
                }
            }
        }
        // --- Advanced-key TGL round trip (wire: [6,2,row,col,TGL,kc,time]) ---
        if caps
            .keyboard
            .as_ref()
            .is_some_and(|k| k.advanced.contains(&AdvancedKind::Tgl))
        {
            print!("   tgl  : ");
            let st = drv.read().ok().and_then(|s| s.keyboard);
            let orig_adv = st
                .as_ref()
                .and_then(|k| k.advanced.iter().find(|a| a.key == 0))
                .cloned();
            let write = Change::Advanced {
                key: 0,
                layer: 0,
                binding: Some(Advanced::Tgl {
                    code: 0x04,
                    delay_ms: 42,
                }),
            };
            let ok1 = drv.apply(&write).is_ok()
                && drv
                    .read()
                    .ok()
                    .and_then(|s| s.keyboard?.advanced.into_iter().find(|a| a.key == 0))
                    .is_some_and(|a| matches!(a.binding, Advanced::Tgl { delay_ms: 42, .. }));
            // Restore the original binding (clear it if None)
            let restore_binding = orig_adv.map(|a| a.binding);
            let ok2 = drv
                .apply(&Change::Advanced {
                    key: 0,
                    layer: 0,
                    binding: restore_binding.clone(),
                })
                .is_ok()
                && drv
                    .read()
                    .ok()
                    .and_then(|s| s.keyboard?.advanced.into_iter().find(|a| a.key == 0))
                    .map(|a| Some(a.binding.clone()))
                    .unwrap_or(None)
                    == restore_binding;
            println!(
                "write+delay42={} restore={}",
                if ok1 { "OK" } else { "FAIL" },
                if ok2 { "OK" } else { "FAIL" },
            );
        }
        // --- Keymap single-key round trip ([3,2,layer,row,30×u16] / [3,4,layer,row,col,kc]) ---
        if caps.keyboard.is_some() {
            print!("   keym : ");
            if let Some(km) = drv
                .read()
                .ok()
                .and_then(|s| s.keyboard?.keymap.into_iter().next())
            {
                if let Some(&orig_code) = km.first() {
                    // Write the same value (no-op check) → round-trip with a different value → restore
                    let other = if matches!(orig_code, KeyAction::Code(0x04)) {
                        KeyAction::Code(0x05)
                    } else {
                        KeyAction::Code(0x04)
                    };
                    let ok = drv
                        .apply(&Change::KeyMap {
                            layer: 0,
                            key: 0,
                            action: other,
                        })
                        .is_ok()
                        && drv
                            .read()
                            .ok()
                            .and_then(|s| s.keyboard?.keymap.into_iter().next()?.first().copied())
                            .is_some_and(|a| a == other)
                        && drv
                            .apply(&Change::KeyMap {
                                layer: 0,
                                key: 0,
                                action: orig_code,
                            })
                            .is_ok()
                        && drv
                            .read()
                            .ok()
                            .and_then(|s| s.keyboard?.keymap.into_iter().next()?.first().copied())
                            .is_some_and(|a| a == orig_code);
                    println!("{}", if ok { "OK" } else { "FAIL" });
                }
            }
        }
        if caps.keyboard.is_none() || caps.lighting.is_empty() {
            continue;
        }
        let area = caps.lighting[0].id;
        let Some(orig) = drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) else {
            continue;
        };
        println!("== {} (area {area})", info.name);
        println!(
            "   current: on={} effect={} #{:06x} lum={} spd={} dir={} keys={}",
            orig.on,
            orig.effect,
            orig.color.to_hex(),
            orig.brightness,
            orig.speed,
            orig.direction,
            orig.per_key.len()
        );
        let mut fails = 0u32;

        // --- 1) Base (effect/brightness/speed/direction/on) round trip ---
        let mut t = orig.clone();
        t.on = !t.on;
        // On models with zones, on is synthesized from zone_on — move all zones to match the toggle.
        if !t.zone_on.is_empty() {
            t.zone_on = vec![t.on; t.zone_on.len()];
        }
        t.effect = if t.effect == 1 { 5 } else { 1 };
        t.brightness = if t.brightness > 50 { 30 } else { 70 };
        t.speed = if t.speed > 50 { 20 } else { 80 };
        t.direction = 1 - t.direction.min(1);
        print!("   base:  ");
        match drv.apply(&Change::Lighting {
            area,
            state: t.clone(),
        }) {
            Err(e) => {
                println!("apply err: {e}");
                fails += 1;
            }
            Ok(_) => {
                // The device may return old values until the save (flash write) takes effect.
                std::thread::sleep(std::time::Duration::from_millis(80));
                match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                    Some(b) => {
                        let ok = b.on == t.on
                            && b.effect == t.effect
                            && b.brightness == t.brightness
                            && b.speed == t.speed
                            && b.direction == t.direction;
                        println!(
                            "on={} effect={} lum={} spd={} dir={} → {}",
                            b.on,
                            b.effect,
                            b.brightness,
                            b.speed,
                            b.direction,
                            if ok { "OK" } else { "MISMATCH" }
                        );
                        if !ok {
                            fails += 1;
                        }
                    }
                    None => {
                        println!("readback failed");
                        fails += 1;
                    }
                }
            }
        }

        // --- 2) Palette color round trip ---
        print!("   color: ");
        let mut t = orig.clone();
        t.color = Rgb::new(0x12, 0x34, 0xab);
        match drv.apply(&Change::Lighting {
            area,
            state: t.clone(),
        }) {
            Err(e) => {
                println!("apply err: {e}");
                fails += 1;
            }
            Ok(_) => {
                // The device may return old values until the save (flash write) takes effect.
                std::thread::sleep(std::time::Duration::from_millis(80));
                match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                    Some(b) => {
                        let ok = b.color == t.color;
                        println!(
                            "wrote #{:06x} → read #{:06x} {}",
                            t.color.to_hex(),
                            b.color.to_hex(),
                            if ok { "OK" } else { "MISMATCH" }
                        );
                        if !ok {
                            fails += 1;
                        }
                    }
                    None => {
                        println!("readback failed");
                        fails += 1;
                    }
                }
            }
        }

        // --- 3) Custom area (absolute write, clear and restore of isCustom cells) ---
        print!("   perkey: ");
        // Put colors on keys 0 and 1 (per_key is always written as absolute state in area0)
        let mut t = orig.clone();
        t.per_key.clear();
        t.per_key.insert(0, Rgb::new(0xff, 0x00, 0x00));
        t.per_key.insert(1, Rgb::new(0x00, 0xff, 0x00));
        match drv.apply(&Change::Lighting {
            area,
            state: t.clone(),
        }) {
            Err(e) => {
                println!("apply err: {e}");
                fails += 1;
            }
            Ok(_) => {
                // The device may return old values until the save (flash write) takes effect.
                std::thread::sleep(std::time::Duration::from_millis(80));
                match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                    Some(b) => {
                        let ok = b.per_key.get(&0) == Some(&Rgb::new(0xff, 0, 0))
                            && b.per_key.get(&1) == Some(&Rgb::new(0, 0xff, 0))
                            && b.per_key.len() <= 4; // keys 0 and 1 may have multiple LED cells
                        println!(
                            "wrote 2 keys → read {} keys {}",
                            b.per_key.len(),
                            if ok { "OK" } else { "MISMATCH" }
                        );
                        if !ok {
                            fails += 1;
                        }
                    }
                    None => {
                        println!("readback failed");
                        fails += 1;
                    }
                }
            }
        }
        // Clear (empty per_key + custom mode → isCustom=0 on all cells)
        print!("   clear:  ");
        let mut t = t.clone();
        t.per_key.clear();
        match drv.apply(&Change::Lighting {
            area,
            state: t.clone(),
        }) {
            Err(e) => {
                println!("apply err: {e}");
                fails += 1;
            }
            Ok(_) => {
                // The device may return old values until the save (flash write) takes effect.
                std::thread::sleep(std::time::Duration::from_millis(80));
                match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                    Some(b) => {
                        println!(
                            "{} keys {}",
                            b.per_key.len(),
                            if b.per_key.is_empty() {
                                "OK"
                            } else {
                                "MISMATCH"
                            }
                        );
                        if !b.per_key.is_empty() {
                            fails += 1;
                        }
                    }
                    None => {
                        println!("readback failed");
                        fails += 1;
                    }
                }
            }
        }

        // --- 4) Zone on/off round trip (DoubleLighting only: [top,bottom] → jo enum) ---
        if !orig.zone_on.is_empty() {
            for (up, down) in [(false, true), (true, false)] {
                print!("   zone:  ");
                let mut t = orig.clone();
                t.zone_on = vec![up, down];
                t.on = up || down;
                match drv.apply(&Change::Lighting {
                    area,
                    state: t.clone(),
                }) {
                    Err(e) => {
                        println!("apply err: {e}");
                        fails += 1;
                    }
                    Ok(_) => {
                        // The device may return old values until the save (flash write) takes effect.
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                            Some(b) => {
                                let ok = b.zone_on == t.zone_on && b.on == t.on;
                                println!(
                                    "up={up} down={down} → read {:?} {}",
                                    b.zone_on,
                                    if ok { "OK" } else { "MISMATCH" }
                                );
                                if !ok {
                                    fails += 1;
                                }
                            }
                            None => {
                                println!("readback failed");
                                fails += 1;
                            }
                        }
                    }
                }
            }
        }

        // --- 5) Palette selection round trip ---
        if !orig.palette.is_empty() {
            print!("   pal:   ");
            let mut t = orig.clone();
            t.palette_sel = 2;
            t.color = Rgb::new(0x20, 0x40, 0x80);
            match drv.apply(&Change::Lighting {
                area,
                state: t.clone(),
            }) {
                Err(e) => {
                    println!("apply err: {e}");
                    fails += 1;
                }
                Ok(_) => {
                    // The device may return old values until the save (flash write) takes effect.
                    std::thread::sleep(std::time::Duration::from_millis(80));
                    match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                        Some(b) => {
                            let ok = b.palette_sel == 2 && b.color == t.color;
                            println!(
                                "sel=2 color=#{:06x} → read sel={} #{:06x} {}",
                                t.color.to_hex(),
                                b.palette_sel,
                                b.color.to_hex(),
                                if ok { "OK" } else { "MISMATCH" }
                            );
                            if !ok {
                                fails += 1;
                            }
                        }
                        None => {
                            println!("readback failed");
                            fails += 1;
                        }
                    }
                }
            }
        }

        // --- 6) Saturation (ColorCorrection) round trip ---
        if orig.color_correction.is_some() {
            print!("   cc:    ");
            let mut t = orig.clone();
            t.color_correction = Some(Rgb::new(60, 45, 90));
            match drv.apply(&Change::Lighting {
                area,
                state: t.clone(),
            }) {
                Err(e) => {
                    println!("apply err: {e}");
                    fails += 1;
                }
                Ok(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(80));
                    match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                        Some(b) => {
                            let ok = b.color_correction == t.color_correction;
                            println!(
                                "wrote (60,45,90) → read {:?} {}",
                                b.color_correction.map(|c| (c.r, c.g, c.b)),
                                if ok { "OK" } else { "MISMATCH" }
                            );
                            if !ok {
                                fails += 1;
                            }
                        }
                        None => {
                            println!("readback failed");
                            fails += 1;
                        }
                    }
                }
            }
        }

        // --- 6.5) Blackout toggle (enable → release immediately. The running LEDs turn fully off for a moment) ---
        print!("   blackout: ");
        match drv.run_action("blackout", None) {
            Err(e) => {
                println!("action err: {e}");
                fails += 1;
            }
            Ok(_) => match drv.run_action("blackout", None) {
                Err(e) => {
                    println!("off err: {e}");
                    fails += 1;
                }
                Ok(_) => println!("on/off OK"),
            },
        }

        // --- 7) Restore (including per_key — always written back as absolute state in area0) ---
        print!("   restore: ");
        match drv.apply(&Change::Lighting {
            area,
            state: orig.clone(),
        }) {
            Err(e) => {
                println!("apply err: {e}");
                fails += 1;
            }
            Ok(_) => {
                // The device may return old values until the save (flash write) takes effect.
                std::thread::sleep(std::time::Duration::from_millis(80));
                match drv.read().ok().and_then(|s| s.lighting.get(&area).cloned()) {
                    Some(b) => {
                        let ok = b.on == orig.on
                            && b.effect == orig.effect
                            && b.brightness == orig.brightness
                            && b.speed == orig.speed
                            && b.direction == orig.direction
                            && b.color == orig.color
                            && b.per_key == orig.per_key
                            && b.zone_on == orig.zone_on
                            && b.palette_sel == orig.palette_sel
                            && b.color_correction == orig.color_correction;
                        println!("{}", if ok { "OK" } else { "MISMATCH (needs review)" });
                        if !ok {
                            fails += 1;
                        }
                    }
                    None => {
                        println!("readback failed");
                        fails += 1;
                    }
                }
            }
        }
        println!("   => {fails} mismatch(es)");
    }
}
