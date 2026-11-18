//! Deeper look at the unknown commands found by grpsweep.
//! - group8 sub list (read-only ones)
//! - Meaning of group2 15/17/11 (told apart by reading with different args)
//! - Meaning of group10 subs 3,5,7,9,11
//! - Confirming the meaning of the group0/sub0 echo
use hidapi::HidApi;
use std::time::{Duration, Instant};

fn open(api: &HidApi) -> Option<hidapi::HidDevice> {
    for d in api.device_list() {
        if d.vendor_id() == 0x1ca6
            && d.product_id() == 0x3009
            && d.usage_page() == 0xFFB0
            && d.usage() == 1
        {
            if let Ok(dev) = api.open_path(d.path()) {
                dev.set_blocking_mode(false).ok();
                return Some(dev);
            }
        }
    }
    None
}

fn cmd(dev: &hidapi::HidDevice, args: &[u8]) -> Option<[u8; 64]> {
    let mut w = [0u8; 65];
    w[1] = args[0];
    w[2] = args[1];
    let n = (args.len() - 2).min(61);
    w[3..3 + n].copy_from_slice(&args[2..2 + n]);
    dev.write(&w).ok()?;
    let t = Instant::now() + Duration::from_millis(300);
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

fn ensure(api: &HidApi, dev: &mut Option<hidapi::HidDevice>) -> bool {
    if dev.is_some() {
        return true;
    }
    for _ in 0..40 {
        if let Some(d) = open(api) {
            *dev = Some(d);
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

fn probe(api: &HidApi, dev: &mut Option<hidapi::HidDevice>, args: &[u8]) -> bool {
    if !ensure(api, dev) {
        println!("{:>2} {:>3} NODEV", args[0], args[1]);
        return false;
    }
    match cmd(dev.as_ref().unwrap(), args) {
        Some(r) => {
            let tag = if r[1] == 0xFF { "UNSUP  " } else { "ECHO   " };
            let hx: String = r[..32]
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ");
            println!("{:>2} {:>3} {:<6} {}", args[0], args[1], tag, hx);
        }
        None => {
            let alive = cmd(dev.as_ref().unwrap(), &[1, 1]).is_some();
            if alive {
                println!("{:>2} {:>3} TIMEOUT", args[0], args[1]);
            } else {
                println!("{:>2} {:>3} DISCONN", args[0], args[1]);
                *dev = None;
                ensure(api, dev);
            }
        }
    }
    true
}

fn main() {
    let api = HidApi::new().unwrap();
    let mut dev = open(&api);

    println!("== group8 subs ==");
    for s in 0u8..=16 {
        if !probe(&api, &mut dev, &[8, s, 0]) {
            break;
        }
    }

    println!("== group2 detailed read (arg variations) ==");
    for s in [0u8, 9, 10, 11, 15, 17] {
        for a in [1u8, 2, 3] {
            probe(&api, &mut dev, &[2, s, a]);
        }
    }

    println!("== group10 subs detail (arg variations) ==");
    for s in [0u8, 3, 5, 7, 9, 11] {
        for a in [1u8, 2, 3] {
            probe(&api, &mut dev, &[10, s, a]);
        }
    }

    println!("== group1 known (control) ==");
    probe(&api, &mut dev, &[1, 1]);
    probe(&api, &mut dev, &[1, 2]);
    probe(&api, &mut dev, &[1, 3]);

    println!("== group3 sub0/4 sub0/5 sub0/6 sub0/7 sub0 meaning (arg variations) ==");
    for (g, s) in [(3u8, 0u8), (4, 0), (5, 0), (6, 0), (7, 0), (10, 0), (2, 0)] {
        for a in [1u8, 2] {
            probe(&api, &mut dev, &[g, s, a]);
        }
    }
}
