//! ATK F1 V3 ULTIMATE+ (373b:12a7) real-device probe — read-only.
//! Confirmed: ff02/usage2 collection, reportId 0x08, 16B packets.
//!   req:  [0]=cmd [1]=0 [2..4]=addr u16BE [4]=len [5..15]=data [15]=sum
//!   resp: [0]=0x08 [1]=cmd [2]=status [3..5]=addr [5]=len [6..]=data [16]=sum
//!   sum = 0x55 - ((8 + sum(bytes0..14)) & 0xff)

use hidapi::HidApi;
use std::time::{Duration, Instant};

fn pkt(cmd: u8, addr: u16, len: u8) -> [u8; 17] {
    let mut p = [0u8; 17];
    p[0] = 8;
    p[1] = cmd;
    p[3] = (addr >> 8) as u8;
    p[4] = addr as u8;
    p[5] = len;
    let s: u32 = 8 + p[1..16].iter().map(|&b| b as u32).sum::<u32>();
    p[16] = 0x55u32.wrapping_sub(s) as u8;
    p
}

fn cmd(d: &hidapi::HidDevice, p: &[u8; 17]) -> Option<[u8; 17]> {
    for _ in 0..3 {
        d.write(p).ok()?;
        let deadline = Instant::now() + Duration::from_millis(300);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut buf = [0u8; 64];
            let n = d
                .read_timeout(&mut buf, left.as_millis().min(80) as i32)
                .ok()?;
            if n >= 17 && buf[1] == p[1] {
                let mut out = [0u8; 17];
                out.copy_from_slice(&buf[..17]);
                return Some(out);
            }
        }
    }
    None
}

fn main() {
    let api = HidApi::new().unwrap();
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == 0x373b
                && d.product_id() == 0x12a7
                && d.usage_page() == 0xff02
                && d.usage() == 2
        })
        .expect("ff02/usage2 collection");
    let d = info.open_device(&api).expect("open");
    d.set_blocking_mode(false).ok();
    // drain
    let mut buf = [0u8; 64];
    while d.read_timeout(&mut buf, 0).unwrap_or(0) > 0 {}

    let reads: Vec<(&str, u8, u16, u8)> = vec![
        ("cidmid", 16, 0, 0),
        ("version", 18, 0, 0),
        ("battery", 4, 0, 0),
        ("dongle_ver", 29, 0, 0),
        ("online", 3, 0, 0),
        ("fardist", 23, 0, 0),
        ("@0 rate/cfg", 8, 0, 10),
        ("@10 lod", 8, 10, 2),
        ("@12 dpi1", 8, 12, 4),
        ("@20 dpi3", 8, 20, 4),
        ("@44 dpi1color", 8, 44, 4),
        ("@76-95 rgb", 8, 76, 20),
        ("@96 key0", 8, 96, 4),
        ("@100 key1", 8, 100, 4),
        ("@104 key2", 8, 104, 4),
        ("@108 key3", 8, 108, 4),
        ("@112 key4", 8, 112, 4),
        ("@160 declight", 8, 160, 8),
        ("@169-190 perf", 8, 169, 22),
        ("@227 rolldelay", 8, 227, 2),
        ("@6912 p3955dpi", 8, 6912, 48),
        ("@6960 dynsens", 8, 6960, 24),
        ("@6984 center", 8, 6984, 16),
        ("@256 sc0", 8, 256, 32),
        ("@288 sc1", 8, 288, 32),
        ("@320 sc2", 8, 320, 32),
        ("@352 sc3", 8, 352, 32),
        ("@384 sc4", 8, 384, 32),
        ("@416 sc5", 8, 416, 32),
        ("@448 sc6", 8, 448, 32),
    ];
    for (tag, c, addr, len) in reads {
        let mut cur = addr;
        let mut remain = len;
        let mut all = String::new();
        while remain > 0 {
            let n = remain.min(10);
            match cmd(&d, &pkt(c, cur, n)) {
                Some(r) => {
                    let ln = r[5] as usize;
                    all += &format!(" {:?}", &r[6..6 + ln.min(10)]);
                    cur += n as u16;
                    remain -= n;
                }
                None => {
                    all += " TIMEOUT";
                    break;
                }
            }
        }
        println!("{tag:16} @{addr}{all}");
    }
}
