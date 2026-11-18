//! Sets output_gain back to High (1) and checks it.

fn main() {
    let api = hidapi::HidApi::new().unwrap();
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == 0x3302 && d.usage_page() == 0x000C)
        .expect("no TP55");
    let d = dev.open_device(&api).unwrap();
    d.set_blocking_mode(false).ok();
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}

    let send = |frame: &[u8]| {
        let mut w = [0u8; 65];
        w[0] = 0x4B;
        w[1..1 + frame.len()].copy_from_slice(frame);
        d.write(&w).unwrap();
    };
    let r1 = |frame: &[u8]| -> Vec<u8> {
        send(frame);
        let mut r = [0u8; 65];
        let n = d.read_timeout(&mut r, 150).unwrap_or(0);
        r[..n].to_vec()
    };
    // Write High and save
    send(&[1, 25, 1, 1]);
    send(&[1, 1, 0]);
    std::thread::sleep(std::time::Duration::from_millis(150));
    for _ in 0..3 {
        let r = r1(&[0x80, 0x19]);
        println!("gain_mode after: {:02x?}", &r[..6.min(r.len())]);
        if !r.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
}
