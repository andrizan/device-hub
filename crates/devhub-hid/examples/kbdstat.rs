//! Quick device-state diagnosis — which command groups are alive.
use hidapi::HidApi;
use std::time::{Duration, Instant};

fn cmd(dev: &hidapi::HidDevice, args: &[u8]) -> Option<[u8; 64]> {
    let mut w = [0u8; 65];
    w[1] = args[0];
    w[2] = args[1];
    let n = (args.len() - 2).min(61);
    w[3..3 + n].copy_from_slice(&args[2..2 + n]);
    dev.write(&w).ok()?;
    let t = Instant::now() + Duration::from_millis(400);
    while Instant::now() < t {
        let mut b = [0u8; 65];
        match dev.read_timeout(&mut b, 60) {
            Ok(n) if n > 0 && b[0] == args[0] => {
                let mut out = [0u8; 64];
                out[..n.min(64)].copy_from_slice(&b[..n.min(64)]);
                return Some(out);
            }
            Err(_) => return None,
            _ => {}
        }
    }
    None
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

    for args in [
        vec![1u8, 1],
        vec![1, 2],
        vec![1, 3],
        vec![2, 3, 0],
        vec![2, 5, 1],
        vec![2, 8, 0],
        vec![3, 1, 0, 0],
        vec![4, 1, 0, 0],
        vec![5, 1, 0, 0],
        vec![5, 3, 0, 0],
        vec![6, 1, 0, 0],
        vec![7, 1],
        vec![10, 1, 0],
        vec![8, 0],
        vec![8, 10],
    ]
    .iter()
    {
        match cmd(&dev, args) {
            Some(r) => {
                let hx: String = r[..24]
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                let tag = if r[1] == 0xFF { "UNSUP" } else { "ECHO " };
                println!("{:?} -> {tag} {hx}", args);
            }
            None => println!("{args:?} -> TIMEOUT"),
        }
    }
}
