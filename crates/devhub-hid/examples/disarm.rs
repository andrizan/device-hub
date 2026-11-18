//! Checks whether closing an armed handle stops the stream.
//! Send [4,3,1,1] on A to arm → close A → read on B for 10 seconds.
//! Run: cargo run -p devhub-hid --example disarm

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
    let a = api.open_path(&path).expect("open A");
    a.set_blocking_mode(false).ok();
    let b = api.open_path(&path).expect("open B");
    b.set_blocking_mode(false).ok();

    let mut buf = [0u8; 65];
    while b.read_timeout(&mut buf, 0).unwrap_or(0) > 0 {}

    let mut req = [0u8; 65];
    req[1..5].copy_from_slice(&[4, 3, 1, 1]);
    a.write(&req).expect("arm on A");
    std::thread::sleep(std::time::Duration::from_millis(800));

    let mut n = 0u32;
    while b.read_timeout(&mut buf, 0).unwrap_or(0) > 0 {
        n += 1;
    }
    println!("B drained {n} buffered frames before A close");

    drop(a);
    println!("closed A (the arming handle) — reading on B for 10s");

    let start = std::time::Instant::now();
    let (mut got, mut last) = (0u64, start);
    while start.elapsed() < std::time::Duration::from_secs(10) {
        if b.read_timeout(&mut buf, 20).unwrap_or(0) > 0 {
            got += 1;
            last = std::time::Instant::now();
        }
    }
    println!(
        "B got {got} frames in 10s after A closed (last at {:.1}s)",
        last.duration_since(start).as_secs_f64()
    );
    if got == 0 {
        println!("=> stream dies when the arming handle closes");
    } else {
        println!("=> stream survives arming-handle close (armed on the device)");
    }
    Ok(())
}
