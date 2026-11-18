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
    // RT precision [2,12,0] → r[3]/1000
    let r = cmd(&dev, &[2, 12, 0]);
    println!(
        "rt_precision: r[3]={} -> step={}mm",
        r[3],
        r[3] as f32 / 1000.0
    );
}
