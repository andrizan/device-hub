//! Dumps the raw data of all cells from the [5,3] readback while a dynamic effect is running.
//! The official HUB draws all cells from this readback as a live preview
//! (checking on the real device whether live colors also appear on non-custom cells).
use hidapi::HidApi;
use std::time::{Duration, Instant};

fn cmd(dev: &hidapi::HidDevice, args: &[u8]) -> [u8; 64] {
    let mut w = [0u8; 65];
    w[1] = args[0];
    w[2] = args[1];
    let n = (args.len() - 2).min(61);
    w[3..3 + n].copy_from_slice(&args[2..2 + n]);
    dev.write(&w).unwrap();
    let t = Instant::now() + Duration::from_millis(700);
    while Instant::now() < t {
        let mut b = [0u8; 65];
        let n = dev.read_timeout(&mut b, 100).unwrap_or(0);
        if n > 0 && b[0] == args[0] {
            let mut out = [0u8; 64];
            out[..n.min(64)].copy_from_slice(&b[..n.min(64)]);
            return out;
        }
    }
    [0xEE; 64]
}

fn dump(dev: &hidapi::HidDevice, tag: &str) {
    println!("--- {tag}");
    for c in 0..9u8 {
        let r = cmd(dev, &[5, 3, 0, c]);
        if r[0] == 0xEE {
            println!("chunk{c}: TIMEOUT");
            continue;
        }
        print!("chunk{c}:");
        for i in 0..15usize {
            let (b, g, rr, f) = (r[4 + i * 4], r[5 + i * 4], r[6 + i * 4], r[7 + i * 4]);
            if rr != 0 || g != 0 || b != 0 || f != 0 {
                print!(" [{}]={},{},{},f{}", c as usize * 15 + i, rr, g, b, f);
            }
        }
        println!();
    }
}

fn main() {
    let api = HidApi::new().unwrap();
    let mut di = None;
    for d in api.device_list() {
        if d.vendor_id() == 0x1ca6
            && d.product_id() == 0x3009
            && d.usage_page() == 0xFFB0
            && d.usage() == 1
        {
            di = Some(d.path().to_owned());
        }
    }
    let dev = api.open_path(&di.expect("su66 not found")).unwrap();
    dev.set_blocking_mode(false).unwrap();

    // Read the current base state
    let r = cmd(&dev, &[5, 1, 0, 0]);
    println!(
        "base: open={} mode={} bri={} spd={} dir={} sel={}",
        r[4], r[5], r[6], r[7], r[8], r[9]
    );
    // Set mode1 + sel0 (firmware rainbow)
    let w = cmd(&dev, &[5, 2, 0, 0, 3, 1, r[6], r[7], r[8], 0]);
    println!("write mode1 sel0: {:02x}", w[0]);
    std::thread::sleep(Duration::from_secs(1));

    for round in 0..3 {
        dump(&dev, &format!("mode1 rainbow t+{}s", round * 2));
        std::thread::sleep(Duration::from_secs(2));
    }
}
