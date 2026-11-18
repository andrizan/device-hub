//! Analysis probe #8: [3,6] echo format + [2,10]/[2,11] retry + DD flag byte difference

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

fn send_no_resp(grp: u8, sub: u8, args: &[u8]) -> bool {
    let Some(api) = hidapi::HidApi::new().ok() else {
        return false;
    };
    let Some(d) = open(&api) else { return false };
    let mut w = [0u8; 65];
    w[1] = grp;
    w[2] = sub;
    w[3..3 + args.len()].copy_from_slice(args);
    d.write(&w).is_ok()
}

fn fmt(r: &[u8]) -> String {
    r.iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    // [3,6] echo format: are sel/row returned in r[2], r[3]?
    println!("== [3,6] echo check");
    for sel in [0x01u8, 0x02, 0x11] {
        match cmd(3, 6, &[sel, 2]) {
            Some(r) => println!(
                "  sel={sel:02x} row=2: r[0..4]={:02x} {:02x} {:02x} {:02x}  data={}",
                r[0],
                r[1],
                r[2],
                r[3],
                fmt(&r[4..])
            ),
            None => println!("  sel={sel:02x}: timeout"),
        }
    }

    // [2,10]/[2,11] retry (timed out last time — check whether it is flaky)
    println!("== [2,10]/[2,11] retry");
    for (sub, arg) in [(10u8, 0u8), (10, 1), (11, 0), (11, 1)] {
        match cmd(2, sub, &[arg]) {
            Some(r) => println!("  [2,{sub},{arg}]: {}", fmt(&r)),
            None => println!("  [2,{sub},{arg}]: timeout"),
        }
    }

    // DirectDrive flag byte difference: first half flag=0xff (implemented) vs second half flag=0x00
    // → if flag=0 means "off/default", the lower half should light differently (visual check)
    println!("== DirectDrive flag variant");
    let area = 0u8;
    let mut cells = Vec::new();
    for i in 0..135u8 {
        let (b, g, r, flag) = if i < 68 {
            (0u8, 0u8, 255u8, 0xffu8) // red, flag=255
        } else {
            (0u8, 255u8, 0u8, 0x00u8) // blue, flag=0
        };
        cells.extend_from_slice(&[b, g, r, flag]);
    }
    let mut ok = true;
    for (chunk, part) in cells.chunks(60).enumerate() {
        let mut args = vec![area, chunk as u8];
        args.extend_from_slice(part);
        if !send_no_resp(5, 5, &args) {
            println!("  chunk {chunk}: send fail");
            ok = false;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    println!("  frames sent: {ok} (first half=red flag255, second half=blue flag0 — 3 s)");
    std::thread::sleep(std::time::Duration::from_secs(3));
    // Recover: read the normal lighting settings and resend them (return to firmware control)
    if let Some(r) = cmd(5, 1, &[0, 0]) {
        let mut args = vec![0u8];
        args.extend_from_slice(&r[4..r.len().min(64)]);
        let _ = cmd(5, 2, &args);
        println!("  restore: lighting setting resent");
    }
}
