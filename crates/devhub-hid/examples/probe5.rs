//! Analysis probe #5: actual layer count, factory defaults, layout style, Caps, unknown subs.
//! Reopens every time because an unknown command can make the firmware detach.
//! Never send [2,17] (USB detach).

fn open(api: &hidapi::HidApi) -> Option<hidapi::HidDevice> {
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() == 0xFFB0)
        .or_else(|| {
            api.device_list()
                .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() & 0xff00 == 0xff00)
        })?;
    let d = dev.open_device(api).ok()?;
    d.set_blocking_mode(false).ok()?;
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}
    Some(d)
}

/// Send 1 command → return the response (reopen method).
fn cmd(grp: u8, sub: u8, args: &[u8]) -> Option<Vec<u8>> {
    let api = hidapi::HidApi::new().ok()?;
    let d = open(&api)?;
    let mut w = [0u8; 65];
    w[1] = grp;
    w[2] = sub;
    w[3..3 + args.len()].copy_from_slice(args);
    if d.write(&w).is_err() {
        return None;
    }
    let mut r = [0u8; 65];
    let mut n = d.read_timeout(&mut r, 300).unwrap_or(0);
    if n > 0 && r[0] != grp {
        n = d.read_timeout(&mut r, 150).unwrap_or(0);
    }
    drop(d);
    drop(api);
    if n == 0 {
        None
    } else {
        Some(r[..n].to_vec())
    }
}

fn fmt(r: &[u8]) -> String {
    r.iter()
        .take(24)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    // === 1. Layer count: try layers 0..5 with [3,1,layer,0] and see whether there is real data ===
    println!("== keymap layers [3,1,layer,row0]");
    for layer in 0..5u8 {
        match cmd(3, 1, &[layer, 0]) {
            Some(r) if r[1] != 0xff => {
                // r[4..] = 30×u16 keycodes. Show only the first 8
                let codes: Vec<String> = (0..8)
                    .map(|i| format!("{:04x}", u16::from_le_bytes([r[4 + i * 2], r[5 + i * 2]])))
                    .collect();
                let nonzero = (4..64)
                    .step_by(2)
                    .filter(|&i| r[i] != 0 || r[i + 1] != 0)
                    .count();
                println!(
                    "  layer{layer}: {} nonzero-pairs, head: {}",
                    nonzero,
                    codes.join(" ")
                );
            }
            Some(_) => println!("  layer{layer}: unsupported(0xff)"),
            None => println!("  layer{layer}: timeout"),
        }
    }

    // === 2. Factory default keymap [3,6,(sys<<4)|fn,row], overview of all 8 sets ===
    println!("== factory default keymap [3,6,(sys<<4)|fn,row0]");
    for sys in 0..2u8 {
        for fnl in 0..4u8 {
            let sel = (sys << 4) | fnl;
            match cmd(3, 6, &[sel, 0]) {
                Some(r) if r[1] != 0xff => {
                    let codes: Vec<String> = (0..8)
                        .map(|i| {
                            format!("{:04x}", u16::from_le_bytes([r[4 + i * 2], r[5 + i * 2]]))
                        })
                        .collect();
                    println!("  sys{sys} fn{fnl}: {}", codes.join(" "));
                }
                Some(_) => println!("  sys{sys} fn{fnl}: unsupported"),
                None => println!("  sys{sys} fn{fnl}: timeout"),
            }
        }
    }

    // === 3. keyLayoutStyle [3,5,row], decode all rows ===
    println!("== keyLayoutStyle [3,5,row] (u16LE → row:5,col:7,ratio:4, /4)");
    for row in 0..6u8 {
        match cmd(3, 5, &[row]) {
            Some(r) if r[1] != 0xff => {
                let cells: Vec<String> = (0..21)
                    .map(|i| {
                        let off = 4 + i * 3;
                        if off + 1 < r.len() {
                            let v = u16::from_le_bytes([r[off], r[off + 1]]);
                            let rr = v >> 11;
                            let cc = (v >> 4) & 0x7f;
                            let ratio = v & 0xf;
                            format!("({:.2},{:.2},r{})", rr as f32 / 4., cc as f32 / 4., ratio)
                        } else {
                            "?".into()
                        }
                    })
                    .collect();
                println!("  row{row}: {}", cells.join(" "));
            }
            Some(_) => println!("  row{row}: unsupported"),
            None => println!("  row{row}: timeout"),
        }
    }

    // === 4. Lighting Caps [5,6] — read → write → read ===
    println!("== lighting caps [5,6]");
    for args in [&[0u8][..], &[1][..]] {
        match cmd(5, 6, args) {
            Some(r) => println!("  read arg{:?}: {}", args, fmt(&r)),
            None => println!("  read arg{:?}: timeout", args),
        }
    }
    // setLightingCaps = [5,6,B,G,R] — write and read back (restore with 0,0,0 at the end)
    for (b, g, r_) in [(0u8, 0u8, 255u8), (0, 0, 0)] {
        match cmd(5, 6, &[b, g, r_]) {
            Some(r) => println!("  write [{b},{g},{r_}]: {}", fmt(&r)),
            None => println!("  write [{b},{g},{r_}]: timeout (write-only?)"),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        match cmd(5, 6, &[0]) {
            Some(r) => println!("    read-back: {}", fmt(&r)),
            None => println!("    read-back: timeout"),
        }
    }

    // === 5. grp10 custom subs 3..11, arg 0/1 ===
    println!("== grp10 custom subs");
    for sub in 3..12u8 {
        for arg in [0u8, 1] {
            match cmd(10, sub, &[arg]) {
                Some(r) => println!("  [10,{sub},{arg}]: {}", fmt(&r)),
                None => println!("  [10,{sub},{arg}]: timeout"),
            }
        }
    }

    // === 6. grp2 unknown sub 15 + known 10/11 sanity ===
    println!("== grp2 misc");
    for (sub, arg) in [(15u8, 0u8), (15, 1), (10, 0), (11, 0)] {
        match cmd(2, sub, &[arg]) {
            Some(r) => println!("  [2,{sub},{arg}]: {}", fmt(&r)),
            None => println!("  [2,{sub},{arg}]: timeout"),
        }
    }
}
