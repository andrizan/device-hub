//! Probe that safely sweeps the SU66 Max command space (robust version).
//! Read-only commands only. Write-type ones (group2 sub1=ResetFactory, sub2=SaveParam,
//! sub2=SET variants of each group, 5,2/5,4/5,5, group-13 pairing, etc.) are excluded.
//! Automatically reopens and continues after a disconnect (confirmed that the real device
//! reboots on unknown subs of group1).
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

/// Returns 64B if there is a response. None on failure/disconnect.
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
    None // timeout
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

fn hexdump(r: &[u8; 64]) -> String {
    r[..24]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn probe(api: &HidApi, dev: &mut Option<hidapi::HidDevice>, args: &[u8]) -> bool {
    if !ensure(api, dev) {
        println!("{:>2} {:>3} NODEV (reopen failed)", args[0], args[1]);
        return false;
    }
    let r = cmd(dev.as_ref().unwrap(), args);
    match r {
        Some(r) => {
            let tag = if r[1] == 0xFF { "UNSUP  " } else { "ECHO   " };
            println!("{:>2} {:>3} {:<6} {}", args[0], args[1], tag, hexdump(&r));
        }
        None => {
            // Tell a timeout from a disconnect — try a light read
            let alive = cmd(dev.as_ref().unwrap(), &[1, 1]).is_some();
            if alive {
                println!("{:>2} {:>3} TIMEOUT", args[0], args[1]);
            } else {
                println!("{:>2} {:>3} DISCONN (device rebooted?)", args[0], args[1]);
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

    println!("== A: check whether unknown groups exist ([g,1]) ==");
    for g in [0u8, 8, 9, 11, 12, 14, 15, 16, 17, 18, 19, 20] {
        probe(&api, &mut dev, &[g, 1]);
    }

    println!("== B: group1 DEVICE unknown subs (may disconnect) ==");
    for s in [0u8, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
        if !probe(&api, &mut dev, &[1, s]) {
            break;
        }
    }

    println!("== C: group2 GLOBAL unknown subs (arg0=0 = read/list convention) ==");
    // 1=ResetFactory and 2=SaveParam are excluded.
    for s in [
        0u8, 9, 10, 11, 15, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 30,
    ] {
        if !probe(&api, &mut dev, &[2, s, 0]) {
            break;
        }
    }

    println!("== D: group3 LAYOUT_KEY unknown subs ==");
    for s in [0u8, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16] {
        if !probe(&api, &mut dev, &[3, s, 0]) {
            break;
        }
    }

    println!("== E: group4 PERFORMANCE unknown subs ==");
    for s in [0u8, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
        if !probe(&api, &mut dev, &[4, s, 0]) {
            break;
        }
    }

    println!("== F: group5 LIGHTING unknown subs ==");
    for s in [0u8, 7, 8, 9, 10, 11, 12] {
        if !probe(&api, &mut dev, &[5, s, 0]) {
            break;
        }
    }
    probe(&api, &mut dev, &[5, 6, 0]);
    probe(&api, &mut dev, &[5, 6, 1]);

    println!("== G: group6 HIGHER_KEY unknown subs ==");
    for s in [0u8, 3, 4, 5, 6, 7, 8, 9, 10] {
        if !probe(&api, &mut dev, &[6, s, 0, 0]) {
            break;
        }
    }

    println!("== H: group7 MACRO unknown subs ==");
    for s in [0u8, 5, 6, 7, 8, 9, 10] {
        if !probe(&api, &mut dev, &[7, s, 0]) {
            break;
        }
    }

    println!("== I: group10 CUSTOM unknown subs ==");
    for s in [0u8, 3, 5, 7, 9, 11, 13, 15] {
        if !probe(&api, &mut dev, &[10, s, 0]) {
            break;
        }
    }

    println!("== J: group12 DISPLAYER ==");
    for s in 1u8..=8 {
        if !probe(&api, &mut dev, &[12, s]) {
            break;
        }
    }

    println!("== K: group13 THREE_MODE known reads ==");
    probe(&api, &mut dev, &[13, 1]);
    println!("(other subs of 13 are excluded as they may be pairing operations)");

    println!("== L: check whether group5 area 2 onward exist ==");
    for a in 0u8..=5 {
        let r = cmd(dev.as_ref().unwrap(), &[5, 1, a, 0]);
        println!(
            "area{a}: {}",
            r.map(|r| hexdump(&r)).unwrap_or_else(|| "TIMEOUT".into())
        );
    }
}
