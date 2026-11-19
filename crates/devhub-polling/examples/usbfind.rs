//! Probe to check endpoint auto-detection (requires admin).
//! `usbfind <hub> <addr> [secs]` — counts the target device's interrupt IN transfers
//! per endpoint and prints them, most frequent first. Run it while moving the target mouse.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hub: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2);
    let addr: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let secs: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);
    eprintln!("Scanning USBPcap{hub} addr {addr} for {secs} s — move the target device");
    match devhub_polling::scan_interrupt(hub, addr, std::time::Duration::from_secs(secs)) {
        Ok(eps) if eps.is_empty() => eprintln!("no transfers"),
        Ok(eps) => {
            for (ep, n) in eps {
                println!("EP 0x{ep:02X} -> {n} transfers");
            }
        }
        Err(e) => eprintln!("error: {e} (os {:?})", e.raw_os_error()),
    }
}
