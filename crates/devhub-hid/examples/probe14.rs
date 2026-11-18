//! Analysis probe #14: verify whether flag=0 releases the cell.
//! 1) Red flag255 on cell75 → 2) frame with flag=0 on all cells → 3) check the custom state via [5,3]
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
    let dd = |cells: &[[u8; 4]]| {
        for (chunk, part) in cells.chunks(15).enumerate() {
            let mut w = [0u8; 65];
            w[1] = 5;
            w[2] = 5;
            w[3] = 0;
            w[4] = chunk as u8;
            for (i, c) in part.iter().enumerate() {
                w[5 + i * 4..9 + i * 4].copy_from_slice(c);
            }
            let _ = d.write(&w);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    let custom_of = |cell: usize| -> u8 {
        let c = (cell / 15) as u8;
        cmd(5, 3, &[0, c])
            .map(|r| r[7 + (cell % 15) * 4])
            .unwrap_or(0xfe)
    };

    // 1) Override cell75 with red flag255
    let mut cells = [[0u8; 4]; 135];
    cells[75] = [0, 0, 255, 255];
    dd(&cells);
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("after flag255 red: cell75 custom={}", custom_of(75));

    // 2) Frame with flag=0 on all cells (release)
    let cells = [[0u8; 4]; 135];
    dd(&cells);
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("after flag0 release: cell75 custom={}", custom_of(75));
    println!("→ if custom becomes 0, flag=0 works as a release");
}
