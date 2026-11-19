use devhub_core::*;

fn main() {
    let path = std::env::args().nth(1).expect("usage: kbdiff <file>");
    let raw = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let plain = {
        use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
        use base64::Engine;
        let b64 = v.get("data").and_then(|d| d.as_str()).unwrap();
        let r = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .unwrap();
        let mut keyiv = Vec::new();
        let mut prev: Vec<u8> = Vec::new();
        use md5::{Digest, Md5};
        while keyiv.len() < 48 {
            let mut h = Md5::new();
            h.update(&prev);
            h.update(b"fDPy6vvnpPsYm2T0g1bh");
            h.update(&r[8..16]);
            prev = h.finalize().to_vec();
            keyiv.extend_from_slice(&prev);
        }
        let mut ct = r[16..].to_vec();
        let pt = cbc::Decryptor::<aes::Aes256>::new_from_slices(&keyiv[..32], &keyiv[32..48])
            .unwrap()
            .decrypt_padded::<Pkcs7>(&mut ct)
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&pt).unwrap()
    };
    let mut grid = std::collections::HashMap::new();
    for ko in plain["keyboards"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r.as_array().into_iter().flatten())
    {
        grid.insert(
            (
                ko["row"].as_u64().unwrap() as u8,
                ko["col"].as_u64().unwrap() as u8,
            ),
            ko,
        );
    }
    let bind = |ko: &serde_json::Value, l: usize| -> u32 {
        let b = &ko["customKeys"]["win"][format!("fn{l}")]["bindKeyValue"];
        match b {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(1) as u32,
            serde_json::Value::String(s) => s
                .split("keycode=")
                .nth(1)
                .and_then(|t| t.trim_end_matches('}').trim().parse().ok())
                .unwrap_or(1),
            _ => 1,
        }
    };
    let layout = devhub_core::registry::su66max_layout();
    let hub = devhub_hid::connect().unwrap();
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
        let st = drv.read().unwrap();
        let kb = st.keyboard.as_ref().unwrap();
        // ---- perf diff ----
        let mut pbad = 0;
        for (i, p) in kb.perf.iter().enumerate() {
            let (row, col) = devhub_core::registry::su66max_matrix_pos(layout[i].id);
            let Some(ko) = grid.get(&(row, col)) else {
                continue;
            };
            let fp = &ko["performance"];
            let f = |n: &str| fp.get(n).and_then(|x| x.as_f64()).unwrap_or(0.) as f32;
            let fmode = fp.get("mode").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
            let checks = [
                (
                    "mode",
                    if p.mode == devhub_core::TriggerMode::Normal {
                        0.
                    } else {
                        1.
                    },
                    fmode.min(1) as f32,
                ),
                ("normalPress", p.actuation, f("normalPress")),
                (
                    "normalRelease",
                    p.release.unwrap_or(p.actuation),
                    f("normalRelease"),
                ),
                ("rtFirstTouch", p.rt_first, f("rtFirstTouch")),
                ("rtPress", p.rt_press, f("rtPress")),
                ("rtRelease", p.rt_release, f("rtRelease")),
                ("pressDead", p.top_deadzone, f("pressDeadStroke")),
                ("releaseDead", p.bottom_deadzone, f("releaseDeadStroke")),
            ];
            for (n, got, want) in checks {
                if (got - want).abs() > 0.0015 {
                    pbad += 1;
                    println!(
                        "perf idx{i} ({row},{col}) '{}': {n} got={got} want={want}",
                        layout[i].label
                    );
                }
            }
        }
        println!("perf mismatches: {pbad}");
        let mut bad = 0;
        for (l, layer) in kb.keymap.iter().enumerate() {
            for (i, a) in layer.iter().enumerate() {
                let (row, col) = devhub_core::registry::su66max_matrix_pos(layout[i].id);
                let want = grid.get(&(row, col)).map(|ko| bind(ko, l)).unwrap_or(1);
                let got = match a {
                    KeyAction::Code(c) => *c,
                    KeyAction::Transparent => 1,
                    KeyAction::Combo { mods, code } => 0x1000 | ((*mods as u32) << 8) | *code,
                    _ => u32::MAX,
                };
                if got != want {
                    bad += 1;
                    println!(
                        "L{l} idx{i} ({row},{col}) '{}': got={got} want={want}",
                        layout[i].label
                    );
                }
            }
        }
        println!("keymap mismatches: {bad}");
    }
}
