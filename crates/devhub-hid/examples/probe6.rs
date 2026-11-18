//! Analysis probe #6: correct decoding of keyLayoutStyle (3B/cell: row/4, col/4, ratio)
//! + content diff of layers 0-3 (check whether fn1-3 are independent)

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

fn main() {
    // === keyLayoutStyle: [3,5,row] → 21 cells×3B {row: b0/4, col: b1/4, ratio: b2} ===
    println!("== keyLayoutStyle decoded (3B/cell)");
    for row in 0..6u8 {
        match cmd(3, 5, &[row]) {
            Some(r) if r[1] != 0xff => {
                let cells: Vec<String> = (0..21)
                    .filter_map(|i| {
                        let off = 4 + i * 3;
                        if off + 2 < r.len() {
                            let (c, l, p) = (r[off], r[off + 1], r[off + 2]);
                            if c == 0 && l == 0 && p == 0 {
                                None
                            } else {
                                Some(format!(
                                    "({:.2},{:.2},w{:.2})",
                                    c as f32 / 4.,
                                    l as f32 / 4.,
                                    p as f32 / 4.
                                ))
                            }
                        } else {
                            None
                        }
                    })
                    .collect();
                println!("  row{row}: {}", cells.join(" "));
            }
            Some(_) => println!("  row{row}: unsupported"),
            None => println!("  row{row}: timeout"),
        }
    }

    // === Read all rows of layers 0-3 → judge the diff ===
    println!("== layer diff (first differing cell vs layer0)");
    let mut layers: Vec<Vec<u16>> = Vec::new();
    for layer in 0..4u8 {
        let mut stream = vec![0u16; 8 * 30];
        for row in 0..8u8 {
            if let Some(r) = cmd(3, 1, &[layer, row]) {
                if r[1] != 0xff && r[2] == layer && r[3] == row {
                    for i in 0..30 {
                        stream[row as usize * 30 + i] =
                            u16::from_le_bytes([r[4 + i * 2], r[5 + i * 2]]);
                    }
                }
            }
        }
        layers.push(stream);
    }
    for l in 1..4 {
        let diffs: Vec<String> = layers[0]
            .iter()
            .zip(layers[l].iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| format!("[{i}]{a:04x}->{b:04x}"))
            .take(12)
            .collect();
        println!(
            "  layer0 vs layer{l}: {} diff cells: {}",
            diffs.len(),
            diffs.join(" ")
        );
    }
}
