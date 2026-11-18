//! Raw response dump for the axis query [4,3,kind,row] — investigating OBS flicker.
//! Prints the r[0..4] echo of the response and the non-zero payload range.

use devhub_hid::transport::{open, Channel};
use std::time::{Duration, Instant};

fn dump(ch: &Channel, kind: u8, row: u8) {
    let w = [4u8, 3, kind, row];
    if ch.send(&w).is_err() {
        println!("  k{kind} r{row}: send err");
        return;
    }
    let dl = Instant::now() + Duration::from_millis(400);
    while Instant::now() < dl {
        match ch.recv(40) {
            Ok(b) if b.first() == Some(&4) => {
                let nz = b.iter().skip(4).filter(|v| **v != 0).count();
                println!(
                    "  q[{kind},{row}] -> [{},{},{},{}] len={} nz={} head={:02x?}",
                    b[0],
                    b[1],
                    b[2],
                    b[3],
                    b.len(),
                    nz,
                    &b[..8.min(b.len())]
                );
                return;
            }
            Ok(b) => println!("  q[{kind},{row}] stray grp={}", b.first().unwrap_or(&0xff)),
            Err(_) => {}
        }
    }
    println!("  q[{kind},{row}] timeout");
}

fn main() {
    let api = match open() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hid: {e}");
            return;
        }
    };
    let Some(path) = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1ca6 && d.product_id() == 0x3009 && d.usage_page() == 0xffb0)
        .map(|d| d.path().to_owned())
    else {
        eprintln!("SU66 vendor iface not found");
        return;
    };
    let dev = match api.open_path(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("open: {e}");
            return;
        }
    };
    dev.set_blocking_mode(false).ok();
    let ch = Channel::open(dev, 65, 65);
    println!("== drain ==");
    let dl = Instant::now() + Duration::from_millis(300);
    let mut n = 0;
    while Instant::now() < dl {
        if let Ok(b) = ch.recv(30) {
            println!(
                "  stray [{},{},{},{}] len={}",
                b[0],
                b[1],
                b[2],
                b[3],
                b.len()
            );
            n += 1;
        }
    }
    println!("drained {n}");
    for kind in [0u8, 1, 3] {
        println!("== kind={kind} ==");
        for row in 1u8..=5 {
            dump(&ch, kind, row);
        }
    }
}
