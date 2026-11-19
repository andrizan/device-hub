//! Determines the input-report delivery model by opening the same HID interface twice.
//! Send an axis request from A and read on both A and B:
//! - The response also reaches B → fan-out to all handles
//! - It does not reach B, only A → shared queue consumption (first handle wins)
//!
//! Run: cargo run -p devhub-hid --example dupread

fn main() -> devhub_core::backend::HubResult<()> {
    let api = hidapi::HidApi::new().expect("hidapi init");
    let mut path = None;
    for d in api.device_list() {
        if d.vendor_id() == 0x1CA6 && d.product_id() == 0x3009 && d.usage_page() == 0xFFB0 {
            path = Some(d.path().to_owned());
            println!("found: {}", d.path().to_string_lossy());
        }
    }
    let Some(path) = path else {
        println!("SU66 config interface not found");
        return Ok(());
    };

    let a = api.open_path(&path).expect("open A");
    a.set_blocking_mode(false).ok();
    let b = api.open_path(&path).expect("open B (second handle)");
    b.set_blocking_mode(false).ok();
    println!("opened two handles on the same interface");

    let drain = |dev: &hidapi::HidDevice, tag: &str| {
        let mut buf = [0u8; 65];
        let mut n = 0;
        while dev.read_timeout(&mut buf, 0).unwrap_or(0) > 0 {
            n += 1;
        }
        println!("pre-drain {tag}: {n} frames");
    };
    drain(&a, "A");
    drain(&b, "B");

    // Send just one command (axis request route/row1) from A.
    let mut req = [0u8; 65];
    req[1..5].copy_from_slice(&[4, 3, 1, 1]);
    a.write(&req).expect("write A");
    println!("sent [4,3,1,1] on A only");

    // While the stream is flowing, send a normal command ([1,2] device info) and
    // check whether the stream stops.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut req2 = [0u8; 65];
    req2[1..3].copy_from_slice(&[1, 2]);
    a.write(&req2).expect("write A [1,2]");
    println!("sent [1,2] device-info on A — does the axis stream stop?");

    let start = std::time::Instant::now();
    let deadline = start + std::time::Duration::from_secs(3);
    let (mut axis_n, mut other_n) = (0u32, 0u32);
    let mut buf = [0u8; 65];
    while std::time::Instant::now() < deadline {
        if a.read_timeout(&mut buf, 10).unwrap_or(0) > 0 {
            if buf[0] == 4 && buf[1] == 3 {
                axis_n += 1;
            } else {
                other_n += 1;
                if other_n <= 3 {
                    println!("non-axis <= {:02x?}", &buf[..8]);
                }
            }
        }
    }
    println!("after [1,2]: axis frames={axis_n}, other frames={other_n} in 3s");
    if axis_n == 0 {
        println!("=> non-axis command DISARMS the stream — coexistence possible");
    } else {
        println!("=> stream persists through normal commands");
    }
    Ok(())
}
