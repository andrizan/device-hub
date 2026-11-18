//! Observing unsolicited reports from the G-Wolves dongle ff02 — keep moving the mouse.
use devhub_hid::transport;
use std::time::Instant;
fn main() {
    let api = transport::open().unwrap();
    let d = api
        .device_list()
        .find(|x| {
            x.vendor_id() == 0x33e4
                && x.product_id() == 0x3854
                && x.usage_page() == 0xff02
                && x.usage() == 2
        })
        .unwrap()
        .open_device(&api)
        .unwrap();
    d.set_blocking_mode(false).ok();
    println!("listening 15s on ff02 — move the HTS mouse");
    let start = Instant::now();
    let mut total = 0u64;
    let mut bucket = 0u64;
    let mut bt = Instant::now();
    let mut seen = std::collections::BTreeMap::new();
    while start.elapsed().as_secs() < 15 {
        let mut b = [0u8; 65];
        if let Ok(n) = d.read_timeout(&mut b, 20) {
            if n > 0 {
                total += 1;
                bucket += 1;
                *seen.entry((b[0], b[1])).or_insert(0u64) += 1;
                if total <= 6 {
                    println!("  rx[{}]: {:02x?}", n, &b[..n.min(20)]);
                }
            }
        }
        if bt.elapsed().as_secs() >= 1 {
            println!("  {}s: {}", bt.elapsed().as_secs(), bucket);
            bucket = 0;
            bt = Instant::now();
        }
    }
    println!("total {total} — by (rid,cmd): {seen:?}");
}
