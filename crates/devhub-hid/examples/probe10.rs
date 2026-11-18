//! Analysis probe #10: identify the correct LED cell indexes of ↑ and Enter on the real device.
//! Light the candidate cells of row 3 (Caps-Enter row) and row 4 (Shift-↑ row) in a rainbow gradient.
//! Cell index → hue is one-to-one, so the index can be read from the color on Enter/↑.

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

fn send_frame(d: &hidapi::HidDevice, cells: &[[u8; 4]]) -> bool {
    for (chunk, part) in cells.chunks(15).enumerate() {
        let mut w = [0u8; 65];
        w[1] = 5;
        w[2] = 5;
        w[3] = 0;
        w[4] = chunk as u8;
        for (i, c) in part.iter().enumerate() {
            w[5 + i * 4..9 + i * 4].copy_from_slice(c);
        }
        if d.write(&w).is_err() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    true
}

fn cmd(d: &hidapi::HidDevice, grp: u8, sub: u8, args: &[u8]) -> Option<Vec<u8>> {
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 10).unwrap_or(0) > 0 {}
    let mut w = [0u8; 65];
    w[1] = grp;
    w[2] = sub;
    w[3..3 + args.len()].copy_from_slice(args);
    d.write(&w).ok()?;
    let mut r = [0u8; 65];
    let n = d.read_timeout(&mut r, 300).unwrap_or(0);
    if n == 0 {
        None
    } else {
        Some(r[..n].to_vec())
    }
}

fn hue(i: usize) -> [u8; 3] {
    // Return B,G,R from the HSV hue for the cell index (one full cycle over 135 cells)
    let h = (i as f32 / 135.0) * 360.0;
    let x = 1.0 - ((h / 60.0) % 2.0 - 1.0).abs();
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [(b * 255.0) as u8, (g * 255.0) as u8, (r * 255.0) as u8]
}

fn main() {
    let api = hidapi::HidApi::new().unwrap();
    let d = open(&api).expect("SU66 not found");

    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("rows");

    let mut cells = [[0u8; 4]; 135];
    match mode {
        // Hue sweep over all of row 3 + row 4 — identify the index from the color on Enter/↑
        "rows" => {
            for i in 42..105 {
                let [b, g, r] = hue(i);
                cells[i] = [b, g, r, 255];
            }
            println!("Lighting row 3 + row 4 (cells 42-104) with a hue sweep — 30 s");
            println!("  cell42=yellow-green → hue is proportional to index (cell÷135×360°)");
            println!(
                "  color→index: yellow≈22-33 green≈45-56 cyan≈67-78 blue≈90-101 purple≈112-124 red≈0-11,124-134"
            );
        }
        // Identify the suspect cells at once: light Enter candidates 73-77 / ↑ candidates 96-99 in different colors
        "suspects" => {
            // (cell, B,G,R) — use easily distinguishable colors for indexes that are far apart
            for &(cell, b, g, r) in &[
                (73usize, 0u8, 0u8, 255u8), // red
                (74, 0, 255, 0),            // blue
                (75, 255, 0, 0),            // green  ← current Enter guess
                (76, 255, 255, 255),        // white
                (77, 0, 255, 255),          // yellow
                (78, 255, 0, 255),          // purple
                (79, 255, 255, 0),          // cyan
            ] {
                cells[cell] = [b, g, r, 255];
            }
            for &(cell, b, g, r) in &[
                (95usize, 255u8, 255u8, 255u8), // white
                (96, 0, 0, 255),                // red
                (97, 0, 255, 0),                // blue
                (98, 255, 0, 0),                // green  ← current ↑ guess
                (99, 0, 255, 255),              // yellow
                (100, 255, 0, 255),             // purple
                (101, 255, 255, 0),             // cyan
            ] {
                cells[cell] = [b, g, r, 255];
            }
            println!("Enter candidates: cells 73-79 = red/blue/green/white/yellow/purple/cyan (current guess=75:green)");
            println!("↑ candidates:     cells 95-101 = white/red/blue/green/yellow/purple/cyan (current guess=98:green)");
            println!("→ Tell me which colors appear on Enter and ↑");
        }
        // Settle which keys 74/75 and 97/98 belong to using red/blue
        "split" => {
            cells[74] = [0, 0, 255, 255]; // red
            cells[75] = [255, 0, 0, 255]; // blue (BGR order!)
            cells[97] = [0, 0, 255, 255]; // red
            cells[98] = [255, 0, 0, 255]; // blue
            println!("cell74=red cell75=blue / cell97=red cell98=blue");
            println!("→ If ' is red, '=74; if blue, 75. If ↑ is red, 97; if Del is blue, 98");
        }
        // Fix check (matrix_col correct): '=74+75 red, Enter=76 green, ↑=97 yellow, Del=98 blue
        "verify" => {
            cells[74] = [0, 0, 255, 255]; // ' = red
            cells[75] = [0, 0, 255, 255]; // ' right-end pad = red
            cells[76] = [0, 255, 0, 255]; // Enter = green (BGR order!)
            cells[97] = [0, 255, 255, 255]; // ↑ = yellow
            cells[98] = [255, 0, 0, 255]; // Del = blue
            println!("'=red Enter=green ↑=yellow Del=blue — if only these 4 keys light up, the fix is OK");
        }
        // Single-cell check: probe10 single <n> lights only cell n in white
        "single" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(98);
            cells[n] = [255, 255, 255, 255];
            println!("Only cell {n} lit in white — 30 s");
        }
        _ => {}
    }

    // Hold the frame for 30 seconds
    for _ in 0..300 {
        send_frame(&d, &cells);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if let Some(r) = cmd(&d, 5, 1, &[0, 0]) {
        let mut args = vec![0u8];
        args.extend_from_slice(&r[4..r.len().min(10)]);
        let _ = cmd(&d, 5, 2, &args);
        println!("firmware lighting restored");
    }
}
