//! Search for an axis-stream disarm command — after arming, send each candidate and
//! judge whether it disarmed by the number of axis frames in the following 800ms.
//! Run: cargo run -p devhub-hid --example disarmcmd

fn axis_frames(dev: &hidapi::HidDevice, ms: u64) -> u32 {
    let mut buf = [0u8; 65];
    let mut n = 0;
    let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < end {
        if dev.read_timeout(&mut buf, 10).unwrap_or(0) > 0 && buf[0] == 4 && buf[1] == 3 {
            n += 1;
        }
    }
    n
}

fn main() -> devhub_core::backend::HubResult<()> {
    let api = hidapi::HidApi::new().expect("hidapi init");
    let mut path = None;
    for d in api.device_list() {
        if d.vendor_id() == 0x1CA6 && d.product_id() == 0x3009 && d.usage_page() == 0xFFB0 {
            path = Some(d.path().to_owned());
        }
    }
    let Some(path) = path else {
        println!("SU66 not found");
        return Ok(());
    };
    let dev = api.open_path(&path).expect("open");
    dev.set_blocking_mode(false).ok();

    let mut buf = [0u8; 65];
    while dev.read_timeout(&mut buf, 0).unwrap_or(0) > 0 {}

    // Arm.
    buf[1..5].copy_from_slice(&[4, 3, 1, 1]);
    dev.write(&buf).expect("arm");
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!("baseline: {} axis frames/800ms", axis_frames(&dev, 800));

    let candidates: &[(&str, &[u8])] = &[
        ("[4,1,1,1] GetPerformance row1", &[4, 1, 1, 1]),
        ("[4,3,255,0] axis kind=255", &[4, 3, 255, 0]),
        ("[4,3,4,1] axis kind=4", &[4, 3, 4, 1]),
        ("[4,3,1,0] axis row=0", &[4, 3, 1, 0]),
        ("[4,3,1,6] axis row=6", &[4, 3, 1, 6]),
        ("[2,3,0] config list", &[2, 3, 0]),
        ("[4,4,0,0] perf sub=4", &[4, 4, 0, 0]),
        ("[4,0,0,0] perf sub=0", &[4, 0, 0, 0]),
        ("[0,0,0,0] zeros", &[0, 0, 0, 0]),
    ];
    for (name, body) in candidates {
        // Re-arm (in case the previous candidate stopped the stream).
        buf[1..5].copy_from_slice(&[4, 3, 1, 1]);
        dev.write(&buf).ok();
        std::thread::sleep(std::time::Duration::from_millis(200));
        while dev.read_timeout(&mut buf, 0).unwrap_or(0) > 0 {}

        let mut cmd = [0u8; 65];
        cmd[1..1 + body.len()].copy_from_slice(body);
        dev.write(&cmd).expect("send candidate");
        let n = axis_frames(&dev, 800);
        println!(
            "{name:34} -> {n} axis frames/800ms {}",
            if n == 0 { "<<< DISARMED" } else { "" }
        );
    }
    Ok(())
}
