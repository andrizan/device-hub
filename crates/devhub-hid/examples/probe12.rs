//! Analysis probe #12: [2,8,0] light-area query — all bytes of the official EffectAreaQuery
fn main() {
    let api = hidapi::HidApi::new().unwrap();
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() == 0xFFB0)
        .or_else(|| {
            api.device_list()
                .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() & 0xff00 == 0xff00)
        })
        .expect("SU66");
    let d = dev.open_device(&api).unwrap();
    d.set_blocking_mode(false).unwrap();
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}
    let mut w = [0u8; 65];
    w[1] = 2;
    w[2] = 8;
    w[3] = 0;
    d.write(&w).unwrap();
    let mut r = [0u8; 65];
    let n = d.read_timeout(&mut r, 300).unwrap();
    print!("[2,8,0] n={n}:");
    for b in &r[..n.min(65)] {
        print!(" {b:02x}");
    }
    println!();
}
