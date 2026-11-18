//! Analysis probe #9: measure the real DirectDrive throughput limit + FPS sweep
//! 1) Write rate limit (continuous sending without sleep)
//! 2) Whether the device is still alive after the burst (ping)
//! 3) Wave animation at several FPS values for visual comparison (needs visual check)

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

fn send_frame(d: &hidapi::HidDevice, cells: &[[u8; 4]], gap_us: u64) -> bool {
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
        if gap_us > 0 {
            std::thread::sleep(std::time::Duration::from_micros(gap_us));
        }
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

/// Frame of a rainbow wave flowing horizontally (135 cells, phase t)
fn wave(t: f32) -> [[u8; 4]; 135] {
    let mut buf = [[0u8; 4]; 135];
    for i in 0..135 {
        let x = (i % 21) as f32; // 21 columns
        let phase = t + x * 0.35;
        let s = phase.sin();
        let c = phase.cos();
        let r = ((s + 1.0) * 127.0) as u8;
        let g = ((c + 1.0) * 127.0) as u8;
        let b = ((1.0 - s * c) * 200.0) as u8;
        buf[i] = [b, g, r, 255];
    }
    buf
}

fn main() {
    let api = hidapi::HidApi::new().unwrap();
    let d = open(&api).expect("SU66 not found (close Device HUB)");

    // 1) Throughput: send 500 chunks continuously
    let cells = wave(0.0);
    let t0 = std::time::Instant::now();
    let mut ok = 0;
    for _ in 0..55 {
        if send_frame(&d, &cells, 0) {
            ok += 1;
        }
    }
    let dt = t0.elapsed();
    println!(
        "[throughput] {ok}/55 frames in {:?} → {:.1} fps potential ({:.2} ms/frame, {:.0} us/chunk)",
        dt,
        ok as f64 / dt.as_secs_f64(),
        dt.as_secs_f64() * 1000.0 / ok.max(1) as f64,
        dt.as_secs_f64() * 1e6 / (ok.max(1) as f64 * 9.0)
    );

    // 2) Does the device respond right after the burst?
    match cmd(&d, 1, 1, &[]) {
        Some(r) => println!("[ping after burst] alive: {:02x} {:02x}", r[0], r[1]),
        None => println!("[ping after burst] TIMEOUT — the firmware is congested"),
    }

    // 3) FPS sweep — 2.5 seconds each, flowing rainbow wave (for visual comparison)
    for &fps in &[10u32, 25, 40, 60, 90] {
        let frame_us = 1_000_000 / fps as u64;
        // Wait after subtracting the send cost of one frame
        let send_cost = dt / ok.max(1) as u32;
        let wait = std::time::Duration::from_micros(frame_us).saturating_sub(send_cost);
        println!(
            "[anim] {fps} fps target (frame={frame_us}us, wait={:?})",
            wait
        );
        let start = std::time::Instant::now();
        let mut n = 0u32;
        while start.elapsed() < std::time::Duration::from_millis(2500) {
            let t = start.elapsed().as_secs_f32() * 4.0;
            send_frame(&d, &wave(t), 0);
            n += 1;
            std::thread::sleep(wait);
        }
        let real_fps = n as f64 / start.elapsed().as_secs_f64();
        println!("   sent {n} frames → real {real_fps:.1} fps");
        match cmd(&d, 1, 1, &[]) {
            Some(_) => println!("   device alive after {fps}fps"),
            None => println!("   device TIMEOUT at {fps}fps — congestion"),
        }
    }

    // 4) Return to firmware control
    if let Some(r) = cmd(&d, 5, 1, &[0, 0]) {
        let mut args = vec![0u8];
        args.extend_from_slice(&r[4..r.len().min(10)]);
        let _ = cmd(&d, 5, 2, &args);
        println!("[restore] firmware lighting resent");
    }
}
