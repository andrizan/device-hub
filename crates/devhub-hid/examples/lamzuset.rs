//! Manually send LAMZU 64B feature commands (for diagnostics).
//! `cargo run -p devhub-hid --example lamzuset -- <b2> <sub> <f> <cmd> [args…]`
//! Example: set polling to 8K on p1 → `lamzuset 2 2 1 0 1 128`

use devhub_hid::transport;

fn main() {
    let bytes: Vec<u8> = std::env::args()
        .skip(1)
        .map(|a| {
            u8::from_str_radix(a.trim_start_matches("0x"), 16)
                .unwrap_or_else(|_| a.parse().unwrap())
        })
        .collect();
    if bytes.len() < 4 {
        eprintln!("usage: lamzuset <b2> <sub> <f> <cmd> [args…]  (decimal or 0x hex)");
        std::process::exit(2);
    }
    let api = transport::open().expect("hidapi");
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == 0x37b0
                && (d.product_id() == 0x0044 || d.product_id() == 0x0048)
                && d.usage_page() == 0xffff
                && d.usage() == 0
        })
        .expect("LAMZU FFFF:00 collection");
    let dev = info.open_device(&api).expect("open");

    let mut w = [0u8; 65];
    w[3] = bytes[0];
    w[4] = bytes[1];
    w[5] = bytes[2];
    w[6] = bytes[3];
    let n = (bytes.len() - 4).min(56);
    w[7..7 + n].copy_from_slice(&bytes[4..4 + n]);
    dev.send_feature_report(&w).expect("send");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut r = [0u8; 65];
    match dev.get_feature_report(&mut r) {
        Ok(len) => println!("-> {:02x?}", &r[..len.min(40)]),
        Err(e) => println!("read err: {e}"),
    }
}
