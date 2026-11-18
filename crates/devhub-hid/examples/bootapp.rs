//! Recovery from being stuck in the bootloader — sends [8,4] BootToApp.
//! (For rescue when the real device entered the bootloader via grpsweep's [8,1] AppToBoot)
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

    println!("pre-state:");
    for args in [vec![1u8, 1], vec![1, 3], vec![5, 1, 0, 0]] {
        println!(
            "  {args:?} -> {:?}",
            cmd(&dev, &args).map(|r| r[..4].to_vec())
        );
    }

    println!("sending [8,4] BootToApp...");
    let r = cmd(&dev, &[8, 4]);
    println!("  resp: {:?}", r.map(|r| r[..8].to_vec()));
    drop(dev);

    println!("waiting re-enumeration...");
    for i in 0..30 {
        std::thread::sleep(Duration::from_millis(500));
        let api2 = HidApi::new().unwrap();
        for d in api2.device_list() {
            if d.vendor_id() == 0x1ca6
                && d.product_id() == 0x3009
                && d.usage_page() == 0xFFB0
                && d.usage() == 1
            {
                if let Ok(dev2) = api2.open_path(d.path()) {
                    dev2.set_blocking_mode(false).ok();
                    let r = cmd(&dev2, &[5, 1, 0, 0]);
                    println!(
                        "  t+{}ms [5,1] -> {:?}",
                        i * 500,
                        r.map(|r| r[..8].to_vec())
                    );
                    if r.is_some() {
                        let r3 = cmd(&dev2, &[1, 3]);
                        println!("  [1,3] -> {:?}", r3.map(|r| r[..8].to_vec()));
                        println!("RECOVERED");
                        return;
                    }
                }
            }
        }
    }
    println!("STILL WEDGED — replug needed");
}
