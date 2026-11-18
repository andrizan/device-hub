//! Deep-dive probe: tries each undocumented command one at a time, reopening the device each time.
//! (Reopens every time because an unknown command can make the firmware USB-detach.)

fn open(api: &hidapi::HidApi) -> Option<hidapi::HidDevice> {
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() == 0xFFB0)
        .or_else(|| {
            api.device_list()
                .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() & 0xff00 == 0xff00)
        })?;
    let d = dev.open_device(api).ok()?;
    d.set_blocking_mode(false).ok();
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}
    Some(d)
}

fn main() {
    let probes: Vec<(&str, u8, u8, Vec<u8>)> = vec![
        ("g3 getcode l0r1c0", 3, 3, vec![0, 1, 0]),
        ("g3 style r0    ", 3, 5, vec![0]),
        ("g3 style r1    ", 3, 5, vec![1]),
        ("g3 deflt f0w0r0", 3, 6, vec![0x00, 0]),
        ("g3 deflt f0w0r1", 3, 6, vec![0x00, 1]),
        ("g3 deflt f1w0r0", 3, 6, vec![0x01, 0]),
        ("g3 deflt f0w1r0", 3, 6, vec![0x10, 0]),
        ("g5 caps arg0   ", 5, 6, vec![0]),
        ("g5 caps arg1   ", 5, 6, vec![1]),
        ("g5 s5 dd arg0  ", 5, 5, vec![0]),
        ("g9 s1 params   ", 9, 1, vec![0]),
        ("g9 s2 listpos  ", 9, 2, vec![0]),
        ("g9 s6 rt       ", 9, 6, vec![0]),
        ("g9 s7 func     ", 9, 7, vec![0]),
        ("g9 s8 runtime  ", 9, 8, vec![0]),
        ("g9 s9 localsw  ", 9, 9, vec![0]),
        ("g10 s2         ", 10, 2, vec![0]),
        ("g10 s3         ", 10, 3, vec![0]),
        ("g10 s3 a1      ", 10, 3, vec![1]),
        ("g10 s4         ", 10, 4, vec![0]),
        ("g10 s12        ", 10, 12, vec![0]),
        ("g1 s1 proto    ", 1, 1, vec![0]),
        ("g4 s4          ", 4, 4, vec![0]),
        ("g4 s5          ", 4, 5, vec![0]),
        ("g6 s4          ", 6, 4, vec![0]),
        ("g6 s5          ", 6, 5, vec![0]),
    ];
    for (name, grp, sub, args) in probes {
        let api = hidapi::HidApi::new().unwrap();
        let Some(d) = open(&api) else {
            println!("{name}: open fail (device gone?)");
            std::thread::sleep(std::time::Duration::from_millis(800));
            continue;
        };
        let mut w = [0u8; 65];
        w[1] = grp;
        w[2] = sub;
        w[3..3 + args.len()].copy_from_slice(&args);
        if let Err(e) = d.write(&w) {
            println!("{name}: send err {e}");
            continue;
        }
        let mut r = [0u8; 65];
        let mut n = d.read_timeout(&mut r, 300).unwrap_or(0);
        if n > 0 && r[0] != grp {
            n = d.read_timeout(&mut r, 150).unwrap_or(0);
        }
        if n == 0 {
            println!("{name}: timeout");
        } else if r[0] == grp && r[1] == 0xff {
            println!("{name}: unsupported(0xff)");
        } else {
            let body: Vec<String> = r[..n.min(24)].iter().map(|b| format!("{b:02x}")).collect();
            println!("{name}: {}", body.join(" "));
        }
        drop(d);
        drop(api);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}
