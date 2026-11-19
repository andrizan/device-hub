//! Read-only probe for the active profile on a single wired MAYA X V2 (LM20).
//! Close Aurora before running: `cargo run -p devhub-hid --example lamzuprobe`.
//! Use `-- --details` to include the settings table and button queries.

use hidapi::{HidApi, HidDevice};
use std::{error::Error, io, time::Duration};

fn query(
    device: &HidDevice,
    name: &str,
    header @ [target, _, group, command]: [u8; 4],
    args: &[u8],
) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    // Report ID 0 followed by the 64-byte WebHID body. Only GET commands are
    // allowed; sending a Feature Report here selects which data to read.
    assert!(command & 0x80 != 0);
    let mut request = [0; 65];
    request[3..7].copy_from_slice(&header);
    request[7..7 + args.len()].copy_from_slice(args);
    device.send_feature_report(&request)?;
    std::thread::sleep(Duration::from_millis(120));

    let mut reply = [0; 65];
    let len = device.get_feature_report(&mut reply)?;
    let result = match reply[..len].get(..8) {
        Some([0, tag @ 0xa1..=0xa3, _, t, size, g, c, _])
            if (*t, *g, *c) == (target, group, command) =>
        {
            let end = 7 + usize::from(*size);
            let frame = reply[..len].get(..end).filter(|frame| frame.len() >= 8);
            frame.map(|frame| (frame, *tag == 0xa1))
        }
        _ => None,
    };
    // Log malformed frames without interpreting them. Keep capturing the other
    // profiles and buttons, even if one of them contains invalid data.
    if let Some((frame, success)) = result {
        println!("{name:18} {header:02x?} {args:02x?} -> {frame:02x?}");
        Ok(success.then(|| frame[7..].to_vec()))
    } else {
        let frame = &reply[..len.min(16)];
        println!("{name:18} {header:02x?} {args:02x?} -> invalid ({len} bytes) {frame:02x?}");
        Ok(None)
    }
}

fn active_profile(device: &HidDevice) -> Result<u8, Box<dyn Error>> {
    query(device, "active profile", [2, 1, 0, 0x85], &[])?
        .and_then(|payload| payload.first().copied())
        .filter(|profile| (1..=4).contains(profile))
        .ok_or_else(|| io::Error::other("invalid active profile").into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let api = HidApi::new()?;
    let mut matches = api.device_list().filter(|d| {
        d.vendor_id() == 0x37b0
            && d.product_id() == 0x0044
            && d.interface_number() == 2
            && d.usage_page() == 0xffff
            && d.usage() == 0
    });
    let info = matches
        .next()
        .ok_or_else(|| io::Error::other("wired MAYA X V2 (37b0:0044 if2 FFFF:00) not found"))?;
    if matches.next().is_some() {
        return Err(io::Error::other("connect only one wired MAYA X V2").into());
    }
    let device = info.open_device(&api)?;
    println!("37b0:0044 if2 FFFF:00 {:?}", info.product_string());
    let profile = active_profile(&device)?;
    for (name, sub, group, command) in [
        ("firmware", 16, 0, 0x81),
        ("battery", 2, 0, 0x83),
        ("sensor", 1, 1, 0x8f),
        ("glass mode", 1, 1, 0x8e),
    ] {
        query(&device, name, [2, sub, group, command], &[])?;
    }
    println!("profile {profile}:");
    // Uninitialized or damaged slots can acknowledge with invalid values. In
    // particular, querying the DPI table of such a slot has failed at the USB
    // level on FW 0.0.3.0. Do not enumerate inactive profiles automatically.
    let rate = query(&device, "polling", [2, 2, 1, 0x80], &[profile])?;
    let valid = matches!(rate.as_deref(), Some([p, 1 | 2 | 16 | 32 | 64 | 128]) if *p == profile);
    if !valid {
        return Err(io::Error::other("invalid active profile polling rate; stopping probe").into());
    }
    if std::env::args().any(|arg| arg == "--details") {
        let stage = query(&device, "active DPI", [2, 2, 1, 0x82], &[profile])?;
        let Some([p, stage @ 1..=5]) = stage.as_deref() else {
            return Err(io::Error::other("invalid active DPI stage; stopping probe").into());
        };
        if *p != profile {
            return Err(io::Error::other("active DPI profile mismatch; stopping probe").into());
        }

        let table = query(&device, "DPI table", [2, 10, 1, 0x81], &[profile, 6])?;
        let valid = matches!(table.as_deref(), Some([p, count @ 1..=5, stages @ ..])
            if *p == profile && stage <= count && stages.len() == 4 * usize::from(*count)
                && stages.as_chunks::<2>().0.iter().all(|x| (50..=50_000).contains(&u16::from_be_bytes(*x))));
        if !valid {
            return Err(io::Error::other("invalid DPI table; stopping probe").into());
        }
        for (name, sub, group, command) in [
            ("DPI colors", 19, 2, 0x81),
            ("DPI separate XY", 2, 1, 0x8d),
            ("LOD", 2, 1, 0x88),
            ("debounce", 2, 0, 0x88),
            ("sleep", 3, 0, 0x87),
            ("angle snap", 2, 1, 0x84),
            ("angle tune", 2, 1, 0x94),
            ("motion sync", 2, 1, 0x89),
            ("ripple", 2, 1, 0x8a),
            ("hyper mode", 2, 1, 0x8b),
            ("tracking mode", 2, 1, 0x93),
            ("rapid click L/R", 3, 0, 0x9a),
            ("scroll HP", 4, 0, 0x99),
            ("sensor offset", 3, 1, 0x95),
            ("adaptive DPI", 9, 1, 0x9a),
        ] {
            query(&device, name, [2, sub, group, command], &[profile])?;
        }
        for button in 1..=5 {
            query(
                &device,
                "button",
                [2, 15, 3, 0x80],
                &[profile, button, 0, 255, 10],
            )?;
        }
    }
    if active_profile(&device)? != profile {
        return Err(io::Error::other("active profile changed during the snapshot").into());
    }
    Ok(())
}
