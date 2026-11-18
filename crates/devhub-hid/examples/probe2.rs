//! Temporary probe: throws unsupported queries at the real device and looks at the responses.
//! U() frame = just put [grp, sub, args...] in report[1..].

fn main() {
    let api = hidapi::HidApi::new().unwrap();
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() == 0xFFB0)
        .or_else(|| {
            api.device_list()
                .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() & 0xff00 == 0xff00)
        })
        .expect("no xsyd device");
    println!(
        "pid {:04x} up={:04x} u={}",
        dev.product_id(),
        dev.usage_page(),
        dev.usage()
    );
    let d = dev.open_device(&api).unwrap();
    d.set_blocking_mode(false).ok();
    // Discard old responses the device has queued up
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}
    for (name, grp, sub, args) in [
        ("special_light  ", 2u8, 11u8, &[0u8][..]),
        ("battery_basic  ", 13, 1, &[][..]),
        ("blackout_stat  ", 10, 1, &[0][..]),
        ("cc_area0       ", 5, 1, &[0, 2][..]),
        ("cc_area1       ", 5, 1, &[1, 2][..]),
        ("decor1_custom  ", 5, 3, &[1, 0][..]),
        ("caps_area0     ", 5, 6, &[0][..]),
        ("caps_area1     ", 5, 6, &[1][..]),
        ("usb_mode       ", 2, 18, &[0][..]),
    ] {
        let mut w = [0u8; 65];
        w[1] = grp;
        w[2] = sub;
        w[3..3 + args.len()].copy_from_slice(args);
        if let Err(e) = d.write(&w) {
            println!("{name}: send err {e}");
            continue;
        }
        let mut r = [0u8; 65];
        let n = d.read_timeout(&mut r, 400).unwrap_or(0);
        if n == 0 {
            println!("{name}: timeout");
        } else {
            println!("{name}: {:02x?}", &r[..n.min(24)]);
        }
    }
}
