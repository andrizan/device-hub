//! Discovery probe that tallies interrupt IN completions on a hub per (device, endpoint).
//! Use it when the target is unknown, to find which endpoint the moving mouse transfers to.
//!
//! ```text
//! cargo run --release -p devhub-polling --example usbscan -- <hub> [seconds]
//! ```
//!
//! Run as admin. Moving the target mouse makes the count for its (addr, ep) jump.

use std::collections::HashMap;
use std::io::Read;

#[cfg(not(windows))]
fn main() {
    eprintln!("usbscan requires Windows");
}

#[cfg(windows)]
fn main() {
    let mut a = std::env::args().skip(1);
    let hub: u16 = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let secs: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(20);
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(format!(r"\\.\USBPcap{hub}"))
        .expect("open USBPcap hub (run as admin)");
    let ioctl = |code: u32, input: &[u8]| {
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        assert!(ok != 0, "ioctl {code:#x} failed");
    };
    ioctl(0x0022_6010, &64u32.to_le_bytes());
    ioctl(0x0022_6000, &(1u32 << 20).to_le_bytes());
    // Set every bit of the address filter — lets all devices on the hub through.
    ioctl(0x0022_e004, &[0xffu8; 17]);

    let mut input = io::BufReader::with_capacity(65536, file);
    let mut global = [0u8; 24];
    input.read_exact(&mut global).expect("pcap global header");
    assert_eq!(&global[..8], &[0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0]);

    println!("USBPcap{hub} — move the target mouse ({secs}s)");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut counts: HashMap<(u16, u8), u32> = HashMap::new();
    let mut packet = [0u8; 64];
    while std::time::Instant::now() < deadline {
        let mut header = [0u8; 16];
        if input.read_exact(&mut header).is_err() {
            break;
        }
        let size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        let Some(data) = packet.get_mut(..size).filter(|d| d.len() >= 27) else {
            break;
        };
        if input.read_exact(data).is_err() {
            break;
        }
        let offset = u16::from_le_bytes([data[0], data[1]]) as usize;
        let dev = u16::from_le_bytes(data[19..21].try_into().unwrap());
        let ep = data[21];
        // Only tally interrupt IN completions that carry data.
        if offset < 27
            || offset >= size
            || data[10..14] != [0; 4]
            || data[16] & 1 == 0
            || data[22] != 1
            || data[23..27] == [0; 4]
        {
            continue;
        }
        *counts.entry((dev, ep)).or_default() += 1;
    }
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for ((dev, ep), n) in rows.iter().take(15) {
        println!("  addr {dev:>3}  EP 0x{ep:02X}  -> {n} transfers");
    }
    if rows.is_empty() {
        println!("  (no interrupt-IN completions captured)");
    }
}

#[cfg(windows)]
use std::io;
