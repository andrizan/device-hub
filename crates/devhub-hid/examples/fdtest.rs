//! ATK dongle (373b:1278) raw-command probe.
use devhub_hid::transport;
use std::time::{Duration, Instant};

fn cmd(dev: &hidapi::HidDevice, c: u8, addr: u16, data: &[u8]) -> Vec<u8> {
    let mut f = [0u8; 17];
    f[0] = 8;
    f[1] = c;
    f[3] = (addr >> 8) as u8;
    f[4] = addr as u8;
    f[5] = data.len().min(10) as u8;
    f[6..6 + data.len().min(10)].copy_from_slice(&data[..data.len().min(10)]);
    let s: u32 = f[..16].iter().map(|&b| b as u32).sum();
    f[16] = 85u32.wrapping_sub(s) as u8;
    dev.write(&f).unwrap();
    let dl = Instant::now() + Duration::from_millis(500);
    loop {
        let mut b = [0u8; 64];
        match dev.read_timeout(&mut b, 100) {
            Ok(n) if n >= 17 && b[1] == c => return b[..17].to_vec(),
            _ => {
                if Instant::now() > dl {
                    return vec![];
                }
            }
        }
    }
}

#[allow(dead_code)] // kept for manual EEPROM reads
fn ee(dev: &hidapi::HidDevice, addr: u16) -> Vec<u8> {
    let mut f = [0u8; 17];
    f[0] = 8;
    f[1] = 8;
    f[3] = (addr >> 8) as u8;
    f[4] = addr as u8;
    f[5] = 10;
    let s: u32 = f[..16].iter().map(|&b| b as u32).sum();
    f[16] = 85u32.wrapping_sub(s) as u8;
    dev.write(&f).unwrap();
    let dl = Instant::now() + Duration::from_millis(500);
    loop {
        let mut b = [0u8; 64];
        match dev.read_timeout(&mut b, 100) {
            Ok(n) if n >= 17 && b[1] == 8 => return b[..17].to_vec(),
            _ => {
                if Instant::now() > dl {
                    return vec![];
                }
            }
        }
    }
}

fn main() {
    let api = transport::open().unwrap();
    let d = api
        .device_list()
        .find(|x| {
            x.vendor_id() == 0x373b
                && x.product_id() == 0x1278
                && x.usage_page() == 0xff02
                && x.usage() == 2
        })
        .unwrap()
        .open_device(&api)
        .unwrap();
    d.set_blocking_mode(false).ok();
    // Enable cmd10 ReportMouseStatus and observe the asynchronous reports
    println!("cmd10 resp: {:?}", cmd(&d, 10, 0, &[1]));
    println!("--- listening 10s (move the mouse) ---");
    let start = Instant::now();
    let mut counts = std::collections::BTreeMap::new();
    let mut total = 0u64;
    while start.elapsed().as_secs() < 10 {
        let mut b = [0u8; 64];
        if let Ok(n) = d.read_timeout(&mut b, 50) {
            if n >= 17 {
                total += 1;
                *counts.entry(b[1]).or_insert(0u64) += 1;
                if total <= 5 {
                    println!("  rx: {:?}", &b[..17]);
                }
            }
        }
    }
    println!("total {total}, by cmd byte: {counts:?}");
    println!("cmd10 off: {:?}", cmd(&d, 10, 0, &[0]));
}
