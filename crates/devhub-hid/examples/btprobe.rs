//! Read-only probe for Bluetooth Classic (RFCOMM) audio devices.
//!
//! `cargo run -p devhub-hid --example btprobe`               … list paired devices
//! `cargo run -p devhub-hid --example btprobe -- earfun`     … firmware query on matching names
//! `cargo run -p devhub-hid --example btprobe -- earfun 0306 0307`
//!                                                          … extra GAIA reads (vendor 0x000A)
//!
//! Only queries are sent: the GAIA / EF firmware reads, plus any GAIA command
//! IDs given on the command line with an empty payload. EarFun GAIA setters
//! take a payload, so an empty one cannot change a setting; `0308` (reboot) and
//! factory reset are refused outright.

use devhub_hid::bt::{self, Rfcomm};
use std::{error::Error, time::Duration};

const UUID_GAIA: u128 = 0x0000_1107_D102_11E1_9B23_0002_5B00_A5A5;

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn ask(link: &Rfcomm, label: &str, frame: &[u8]) {
    if let Err(e) = link.send(frame) {
        println!("    {label}: send failed: {e}");
        return;
    }
    let mut buf = [0u8; 512];
    let mut got = Vec::new();
    // Collect for up to 600 ms; replies can arrive split across reads.
    for _ in 0..6 {
        match link.recv(&mut buf, Duration::from_millis(100)) {
            Ok(0) => {
                if !got.is_empty() {
                    break;
                }
            }
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) => {
                println!("    {label}: recv failed: {e}");
                return;
            }
        }
    }
    let text: String = got
        .iter()
        .map(|b| {
            if (0x20..0x7f).contains(b) {
                *b as char
            } else {
                '.'
            }
        })
        .collect();
    println!("    {label}: {} -> {}  |{text}|", hex(frame), hex(&got));
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let devices = bt::paired()?;
    let Some(needle) = args.first().map(|s| s.to_lowercase()) else {
        for d in &devices {
            println!(
                "{}  class={:06x}  connected={}  {:?}",
                bt::addr_string(d.addr),
                d.class,
                d.connected,
                d.name
            );
        }
        println!("-- {} paired --", devices.len());
        return Ok(());
    };
    // `btprobe <name> --listen <seconds>`: open the serial channel and print
    // whatever the device sends on its own. Nothing is written to it. Press
    // the device's buttons meanwhile to see what it reports.
    if let Some(at) = args.iter().position(|a| a == "--listen") {
        let secs: u64 = args.get(at + 1).ok_or("--listen <seconds>")?.parse()?;
        for d in devices
            .iter()
            .filter(|d| d.connected && d.name.to_lowercase().contains(&needle))
        {
            println!(
                "{} {:?}: listening {secs} s",
                bt::addr_string(d.addr),
                d.name
            );
            let link = Rfcomm::connect(d.addr, bt::UUID_SPP, Duration::from_secs(6))?;
            let end = std::time::Instant::now() + Duration::from_secs(secs);
            let mut buf = [0u8; 512];
            let mut total = 0;
            while std::time::Instant::now() < end {
                let n = link.recv(&mut buf, Duration::from_millis(250))?;
                if n > 0 {
                    total += n;
                    println!("  <- {}", hex(&buf[..n]));
                }
            }
            println!("  {total} bytes received");
        }
        return Ok(());
    }
    let mut extra: Vec<u16> = Vec::new();
    for a in args.iter().skip(1) {
        let id = u16::from_str_radix(a, 16)?;
        // 0x0308 = reboot, 0x03xx with bit 0x8000 = responses. Anything that is
        // not a plain status query in the 0x03 block is refused.
        if id == 0x0308 || id >> 8 != 0x03 {
            return Err(format!("command {id:#06x} is not on the read allowlist").into());
        }
        extra.push(id);
    }
    for d in devices
        .iter()
        .filter(|d| d.name.to_lowercase().contains(&needle))
    {
        println!(
            "{} {:?} connected={}",
            bt::addr_string(d.addr),
            d.name,
            d.connected
        );
        if !d.connected {
            println!("  not connected — skipped");
            continue;
        }
        for (name, uuid) in [("SPP 1101", bt::UUID_SPP), ("GAIA 1107", UUID_GAIA)] {
            match Rfcomm::connect(d.addr, uuid, Duration::from_secs(6)) {
                Err(e) => println!("  {name}: {e}"),
                Ok(link) => {
                    println!("  {name}: connected");
                    ask(
                        &link,
                        "GAIA fw 0x0309",
                        &[0xff, 0x04, 0, 0, 0, 0x0a, 0x03, 0x09],
                    );
                    ask(
                        &link,
                        "EF   fw 0x1010",
                        &[0xef, 0x10, 0x10, 0x00, 0x00, 0xfe],
                    );
                    for id in &extra {
                        let [hi, lo] = id.to_be_bytes();
                        ask(
                            &link,
                            &format!("GAIA {id:#06x}"),
                            &[0xff, 0x04, 0, 0, 0, 0x0a, hi, lo],
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
