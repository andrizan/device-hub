//! Analysis probe #13: isolating why ' (cell75) stays stuck red after DD stops.
//! 1) Read the isCustom of cell75 via [5,3] → 2) does resending [5,2] recover it?
//!    3) If not, clear all isCustom via [5,4]
fn main() {
    let api = hidapi::HidApi::new().unwrap();
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() == 0xFFB0)
        .or_else(|| {
            api.device_list()
                .find(|d| d.vendor_id() == 0x1ca6 && d.usage_page() & 0xff00 == 0xff00)
        })
        .expect("SU66 — close Device HUB");
    let d = dev.open_device(&api).unwrap();
    d.set_blocking_mode(false).unwrap();
    let mut drain = [0u8; 65];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}

    let cmd = |grp: u8, sub: u8, args: &[u8]| -> Option<Vec<u8>> {
        let mut drain = [0u8; 65];
        while d.read_timeout(&mut drain, 10).unwrap_or(0) > 0 {}
        let mut w = [0u8; 65];
        w[1] = grp;
        w[2] = sub;
        w[3..3 + args.len()].copy_from_slice(args);
        d.write(&w).ok()?;
        let mut r = [0u8; 65];
        let n = d.read_timeout(&mut r, 400).unwrap_or(0);
        if n == 0 {
            None
        } else {
            Some(r[..n].to_vec())
        }
    };

    // 1) Check the isCustom of cells 73-78 via the [5,3] readback
    let rb = cmd(5, 3, &[0, 4]); // chunk4 = cells 60-74
    let rb2 = cmd(5, 3, &[0, 5]); // chunk5 = cells 75-89
    if let (Some(a), Some(b)) = (&rb, &rb2) {
        let dump = |r: &Vec<u8>, base: usize| {
            for i in 0..15 {
                let c = base + i;
                if (73..=79).contains(&c) {
                    println!(
                        "  cell{c}: B={:02x} G={:02x} R={:02x} custom={}",
                        r[4 + i * 4],
                        r[5 + i * 4],
                        r[6 + i * 4],
                        r[7 + i * 4]
                    );
                }
            }
        };
        println!("[5,3] cells 73-79:");
        dump(a, 60);
        dump(b, 75);
    }

    // 2) Equivalent of stopping DD: [5,1] → resend [5,2]
    if let Some(r) = cmd(5, 1, &[0, 0]) {
        let mut args = vec![0u8];
        args.extend_from_slice(&r[4..10]);
        println!(
            "[5,2] resend: {:?}",
            cmd(5, 2, &args).map(|r| r[..8].to_vec())
        );
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    let rb = cmd(5, 3, &[0, 4]);
    let rb2 = cmd(5, 3, &[0, 5]);
    if let (Some(a), Some(b)) = (&rb, &rb2) {
        let c74 = &a[4 + 14 * 4..8 + 14 * 4];
        let c75 = &b[4..8];
        println!(
            "after [5,2]: cell74 custom={} cell75 custom={} (rgb {c74:02x?} {c75:02x?})",
            c74[3], c75[3]
        );
    }

    // 3) If still custom, clear the isCustom of all cells via [5,4] (colors keep their current values)
    let rb = cmd(5, 3, &[0, 5]);
    if rb.as_ref().is_some_and(|r| r[7] == 255) {
        println!("cell75 still custom → clearing only the flag via [5,4]");
        for c in 0..9u8 {
            if let Some(rb) = cmd(5, 3, &[0, c]) {
                let mut w = [0u8; 65];
                w[1] = 5;
                w[2] = 4;
                w[3] = 0;
                w[4] = c;
                w[5..65].copy_from_slice(&rb[4..64]);
                for i in 0..15 {
                    w[8 + i * 4] = 0;
                } // isCustom=0
                let _ = d.write(&w);
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
        }
        let _ = cmd(2, 2, &[3]); // save lighting
        println!("cleared + saved");
    }
}
