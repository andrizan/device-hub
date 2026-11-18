//! PoC for measuring polling by reading the input interface directly.
//! `cargo run -p devhub-hid --example rateread` — keep moving the mouse.
//! Unlike WM_INPUT, it should not be affected by background throttling.

use devhub_hid::transport;
use std::time::Instant;

fn main() {
    let api = transport::open().unwrap();
    // Look for the standard mouse input collection (up=1, u=6) of the ATK dongle/wired.
    let d = api
        .device_list()
        .find(|x| {
            x.vendor_id() == 0x373b
                && (x.product_id() == 0x1278 || x.product_id() == 0x12a7)
                && x.usage_page() == 0x0001
                && x.usage() == 0x06
        })
        .expect("ATK input interface up1/u6")
        .open_device(&api)
        .expect("open input");
    d.set_blocking_mode(false).ok();
    println!("opened — move the mouse. 30 seconds, 1s buckets:");

    let start = Instant::now();
    let mut total = 0u64;
    let mut bucket = 0u64;
    let mut bucket_t = Instant::now();
    let mut intervals: Vec<f64> = vec![];
    let mut last = Instant::now();
    let mut buf = [0u8; 64];
    while start.elapsed().as_secs() < 30 {
        match d.read_timeout(&mut buf, 10) {
            Ok(n) if n > 0 => {
                let now = Instant::now();
                let dt = now.duration_since(last).as_secs_f64();
                if dt > 0.00002 {
                    intervals.push(dt);
                }
                last = now;
                total += 1;
                bucket += 1;
            }
            _ => {}
        }
        if bucket_t.elapsed().as_secs() >= 1 {
            println!("  {}s: {} reports", bucket_t.elapsed().as_secs(), bucket);
            bucket = 0;
            bucket_t = Instant::now();
        }
    }
    println!("total {total} in 30s -> {:.0} Hz", total as f64 / 30.0);
    if !intervals.is_empty() {
        intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = intervals[intervals.len() / 2];
        let p95 = intervals[intervals.len() * 95 / 100];
        println!(
            "interval: median {:.4}ms (~{:.0}Hz)  p95 {:.4}ms  min {:.4}ms",
            med * 1000.0,
            1.0 / med,
            p95 * 1000.0,
            intervals[0] * 1000.0
        );
    }
}
