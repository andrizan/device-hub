//! Read-only probe for RongYuan (VID 3151) keyboards / mice / 2.4G receivers.
//!
//! `cargo run -p devhub-hid --example ryprobe`                    … identify every FFFF:02 interface
//! `cargo run -p devhub-hid --example ryprobe -- 4010 87`         … one GET on PID 4010
//! `cargo run -p devhub-hid --example ryprobe -- 4010 89.0.0`     … GET with argument bytes
//! `cargo run -p devhub-hid --example ryprobe -- 402d f7 f6.5 8f f7 fc`
//!                                                              … several exchanges on one handle
//!                                                                (receiver relay by hand)
//!
//! Each command is hex bytes joined by `.`. Only opcodes on the allowlist below
//! are sent: plain GETs and the receiver's status / relay opcodes. `0xAC`
//! (flash erase), `0x7F`/`0x30` (boot entry) and every SET stay unreachable
//! from this tool.

use hidapi::{HidApi, HidDevice};
use std::{error::Error, io, time::Duration};

/// GET opcodes that only select what the next feature read returns.
const ALLOWED: &[u8] = &[
    0x80, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x97,
    0xad, 0xae, 0xd0, 0xd1, 0xd3, 0xd4, 0xe0, 0xf0, 0xf7, 0xfb, 0xfc, 0xfd,
];

fn exchange(dev: &HidDevice, body: &[u8]) -> Result<[u8; 64], Box<dyn Error>> {
    let op = *body.first().ok_or_else(|| io::Error::other("empty"))?;
    // 0xF6 = receiver relay target (RAM only): keyboard 10 / mouse 5 / both 13.
    let relay_select = op == 0xf6 && matches!(body.get(1), Some(5 | 10 | 13)) && body.len() == 2;
    if !ALLOWED.contains(&op) && !relay_select {
        return Err(
            io::Error::other(format!("opcode {op:#04x} is not on the read allowlist")).into(),
        );
    }
    // report ID 0 + 64B body, Bit7 checksum over body[0..7].
    let mut req = [0u8; 65];
    req[1..1 + body.len()].copy_from_slice(body);
    let sum: u32 = req[1..8].iter().map(|b| *b as u32).sum();
    req[8] = 0xff - (sum & 0xff) as u8;
    dev.send_feature_report(&req)?;
    std::thread::sleep(Duration::from_millis(40));
    let mut rep = [0u8; 65];
    let n = dev.get_feature_report(&mut rep)?;
    if n < 2 {
        return Err(io::Error::other("short feature report").into());
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(&rep[1..65]);
    Ok(out)
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let api = HidApi::new()?;
    let want_pid = args
        .first()
        .map(|s| u16::from_str_radix(s, 16))
        .transpose()?;
    let mut cmds: Vec<Vec<u8>> = Vec::new();
    for a in args.iter().skip(1) {
        cmds.push(
            a.split('.')
                .map(|s| u8::from_str_radix(s, 16))
                .collect::<Result<_, _>>()?,
        );
    }
    if cmds.is_empty() {
        cmds.push(vec![0x8f]);
    }
    for d in api.device_list() {
        if d.vendor_id() != 0x3151 || d.usage_page() != 0xffff || d.usage() != 0x02 {
            continue;
        }
        if want_pid.is_some_and(|p| p != d.product_id()) {
            continue;
        }
        println!(
            "3151:{:04x} if={} {:?}",
            d.product_id(),
            d.interface_number(),
            d.product_string()
        );
        let dev = match d.open_device(&api) {
            Ok(dev) => dev,
            Err(e) => {
                println!("  open failed: {e}");
                continue;
            }
        };
        for body in &cmds {
            std::thread::sleep(Duration::from_millis(110));
            match exchange(&dev, body) {
                Ok(r) => println!("  {:<12} -> {}", hex(body), hex(&r)),
                Err(e) => println!("  {:<12} failed: {e}", hex(body)),
            }
        }
    }
    Ok(())
}
