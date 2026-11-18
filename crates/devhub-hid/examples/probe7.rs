//! Analysis probe #7: raw-byte check of keyLayoutStyle (for identifying offsets)

fn open(api: &hidapi::HidApi) -> Option<hidapi::HidDevice> {
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() == 0xFFB0)
        .or_else(|| {
            api.device_list()
                .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() & 0xff00 == 0xff00)
        })?;
    let d = dev.open_device(api).ok()?;
    d.set_blocking_mode(false).ok()?;
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}
    Some(d)
}

fn cmd(grp: u8, sub: u8, args: &[u8]) -> Option<Vec<u8>> {
    let api = hidapi::HidApi::new().ok()?;
    let d = open(&api)?;
    let mut w = [0u8; 65];
    w[1] = grp;
    w[2] = sub;
    w[3..3 + args.len()].copy_from_slice(args);
    if d.write(&w).is_err() {
        return None;
    }
    let mut r = [0u8; 65];
    let mut n = d.read_timeout(&mut r, 300).unwrap_or(0);
    if n > 0 && r[0] != grp {
        n = d.read_timeout(&mut r, 150).unwrap_or(0);
    }
    drop(d);
    drop(api);
    if n == 0 {
        None
    } else {
        Some(r[..n].to_vec())
    }
}

fn main() {
    for row in 0..6u8 {
        match cmd(3, 5, &[row]) {
            Some(r) => {
                let hex: Vec<String> = r.iter().map(|b| format!("{b:02x}")).collect();
                println!("row{row} n={}: {}", r.len(), hex.join(" "));
            }
            None => println!("row{row}: timeout"),
        }
    }
}
