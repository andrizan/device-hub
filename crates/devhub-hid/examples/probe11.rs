//! Analysis probe #11: correctly decode all cells of [3,5] keyLayoutStyle and examine their order
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
    let mut all = Vec::new();
    for page in 0..8u8 {
        let mut w = [0u8; 65];
        w[1] = 3;
        w[2] = 5;
        w[3] = page;
        if d.write(&w).is_err() {
            break;
        }
        let mut r = [0u8; 65];
        let n = d.read_timeout(&mut r, 300).unwrap_or(0);
        if n == 0 {
            println!("page {page}: timeout");
            break;
        }
        // r[0]=3 r[1]=5 r[2]=page r[3..] = u16LE cells
        let body = &r[3..n.min(65)];
        let cnt = body.len() / 2;
        println!("page {page}: {cnt} cells (n={n})");
        for i in 0..cnt {
            let v = u16::from_le_bytes([body[i * 2], body[i * 2 + 1]]);
            let row = v & 0x1f;
            let col = (v >> 5) & 0x7f;
            let ratio = (v >> 12) & 0xf;
            if row == 0 && col == 0 && ratio == 0 {
                continue;
            }
            all.push((row, col, ratio));
        }
    }
    println!(
        "\n== {} cells (index: row/4.0, col/4.0, ratio) ==",
        all.len()
    );
    for (i, (row, col, ratio)) in all.iter().enumerate() {
        println!(
            "  [{i:3}] y={:.2} x={:.2} ratio={ratio}",
            *row as f32 / 4.0,
            *col as f32 / 4.0
        );
    }
}
