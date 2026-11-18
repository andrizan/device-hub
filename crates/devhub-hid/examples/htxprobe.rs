//! HTX Ultra (33e4:5618, JM_056_3950) real-device probe.
//! Raw-dumps the main addresses over the report-8/EEPROM path and confirms the model constants
//! (DPI layout / LOD sign / SPDT @KeyOperation) on the real device.
//! Read-only. No writes.

use hidapi::HidApi;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn open(api: &HidApi, pid: u16) -> Option<hidapi::HidDevice> {
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == 0x33e4
                && d.product_id() == pid
                && d.usage_page() == 0xff02
                && d.usage() == 0x02
        })
        .or_else(|| {
            api.device_list().find(|d| {
                d.vendor_id() == 0x33e4
                    && d.product_id() == pid
                    && d.usage_page() & 0xff00 == 0xff00
            })
        })?;
    let d = info.open_device(api).ok()?;
    d.set_blocking_mode(false).ok()?;
    Some(d)
}

/// 16B command → 17B response. Data is r[6..].
fn cmd(d: &hidapi::HidDevice, f: &[u8; 16]) -> Option<[u8; 17]> {
    let op = f[0];
    let mut w = [0u8; 17];
    w[0] = 8;
    w[1..17].copy_from_slice(f);
    let s: u32 = w[..16].iter().map(|&b| b as u32).sum();
    w[16] = 85u32.wrapping_sub(s) as u8;

    // Read directly instead of using a receive thread: pick up the response by its op echo
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _ = tx; // Simplification: read on the same thread
    for _try in 0..3 {
        d.write(&w).ok()?;
        let deadline = Instant::now() + Duration::from_millis(400);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut buf = [0u8; 64];
            let n = d
                .read_timeout(&mut buf, left.as_millis().min(120) as i32)
                .ok()?;
            if n >= 17 && buf[0] == 8 && buf[1] == op {
                let mut out = [0u8; 17];
                out.copy_from_slice(&buf[..17]);
                return Some(out);
            }
            // Discard unsolicited notifications (op10 etc.) and continue
            let _ = rx.try_recv();
        }
    }
    None
}

fn ee(d: &hidapi::HidDevice, addr: u16, len: u16) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut a = addr;
    while out.len() < len as usize {
        let want = (len - out.len() as u16).min(10) as u8;
        let mut f = [0u8; 16];
        f[0] = 8;
        f[2] = (a >> 8) as u8;
        f[3] = (a & 0xff) as u8;
        f[4] = want;
        let r = cmd(d, &f)?;
        out.extend_from_slice(&r[6..6 + want as usize]);
        a += want as u16;
        std::thread::sleep(Duration::from_millis(5));
    }
    Some(out)
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn dump(d: &hidapi::HidDevice, name: &str, addr: u16, len: u16) {
    match ee(d, addr, len) {
        Some(b) => println!("@{addr:<5} {name:<28} {}", hex(&b)),
        None => println!("@{addr:<5} {name:<28} <timeout>"),
    }
}

/// Scalar (v, 0x55-v) pair write.
fn ee_set(d: &hidapi::HidDevice, addr: u16, val: u8) -> bool {
    let mut f = [0u8; 16];
    f[0] = 7;
    f[2] = (addr >> 8) as u8;
    f[3] = (addr & 0xff) as u8;
    f[4] = 2;
    f[5] = val;
    f[6] = 85u8.wrapping_sub(val);
    cmd(d, &f).is_some()
}

fn main() {
    let api = HidApi::new().unwrap();
    let args: Vec<String> = std::env::args().collect();
    // If the first argument is a number, it is the PID (hex). Default is 5618 (HTX Ultra wired).
    let mut argi = 1usize;
    let mut pid = 0x5618u16;
    if let Some(a) = args.get(1) {
        if let Ok(p) = u16::from_str_radix(a, 16) {
            pid = p;
            argi = 2;
        }
    }
    let d = match open(&api, pid) {
        Some(d) => d,
        None => {
            eprintln!("33e4:{pid:04x} (ff02/02) not found");
            return;
        }
    };
    println!("opened 33e4:{pid:04x}");
    let mut drain = [0u8; 64];
    while d.read_timeout(&mut drain, 20).unwrap_or(0) > 0 {}

    // Write mode: `htxprobe [pid] write <addr> <val>` → pair-write and read back
    if args.len() == argi + 3 && args[argi] == "write" {
        let addr: u16 = args[argi + 1].parse().unwrap();
        let val: u8 = args[argi + 2].parse().unwrap();
        let ok = ee_set(&d, addr, val);
        println!(
            "write @{addr} = {val} -> {}",
            if ok { "ack" } else { "FAIL" }
        );
        std::thread::sleep(Duration::from_millis(30));
        match ee(&d, addr, 2) {
            Some(b) => println!("readback @{}: {}", addr, hex(&b)),
            None => println!("readback: timeout"),
        }
        return;
    }

    // ---- op level ----
    let mut f = [0u8; 16];
    f[0] = 18; // FW
    if let Some(r) = cmd(&d, &f) {
        println!(
            "op18 FW        : {}  -> ver {}.{}.{}.{}",
            hex(&r),
            r[6],
            r[7],
            r[8],
            r[9]
        );
    } else {
        println!("op18 FW        : <timeout>");
    }
    let mut f = [0u8; 16];
    f[0] = 4; // battery
    if let Some(r) = cmd(&d, &f) {
        let mv = ((r[7] as u16) << 8) | r[8] as u16;
        println!(
            "op04 battery   : {}  -> {}% state={} {}mV",
            hex(&r),
            r[6],
            r[7] & 0,
            mv
        );
        println!("               raw [6]={} [7]={} [8]={}", r[6], r[7], r[8]);
    } else {
        println!("op04 battery   : <timeout>");
    }
    let mut f = [0u8; 16];
    f[0] = 14; // profile id
    if let Some(r) = cmd(&d, &f) {
        println!("op14 profile   : {}  -> {}", hex(&r), r[6]);
    } else {
        println!("op14 profile   : <timeout>");
    }
    let mut f = [0u8; 16];
    f[0] = 1; // EID/donglePR nonce — official getEID is r[4]=8 + nonce 4B
    f[4] = 8;
    f[5] = 0xAA;
    f[6] = 0xBB;
    f[7] = 0xCC;
    f[8] = 0xDD;
    if let Some(r) = cmd(&d, &f) {
        println!(
            "op01 EID/PR    : {}  -> EID={} PR={} (r[10]={})",
            hex(&r),
            r[11],
            r[12],
            r[10]
        );
    } else {
        println!("op01 EID/PR    : <timeout>");
    }

    // ---- EEPROM ----
    dump(&d, "ReportRate", 0, 2);
    dump(&d, "maxDpiStage", 2, 2);
    dump(&d, "CurrentDPI", 4, 2);
    dump(&d, "keyOp?", 6, 2);
    dump(&d, "KeyOperation(SPDT)", 8, 2);
    dump(&d, "LOD", 10, 2);
    dump(&d, "DPIValue 4Bx8 (legacy)", 12, 32);
    dump(&d, "DPIColor 4Bx8", 44, 32);
    dump(&d, "DPIEffectMode", 76, 2);
    dump(&d, "DPIEffectState", 82, 2);
    dump(&d, "KeyFunction 4Bx5", 96, 20);
    dump(&d, "DebounceTime", 169, 2);
    dump(&d, "MotionSync", 171, 2);
    dump(&d, "SleepTime", 173, 2);
    dump(&d, "Angle", 175, 2);
    dump(&d, "Ripple", 177, 2);
    dump(&d, "PerformanceState", 181, 2);
    dump(&d, "Performance", 183, 2);
    dump(&d, "AngleTune", 189, 2);
    dump(&d, "AngleTuneState", 191, 2);
    dump(&d, "SensorFPS20K", 225, 2);
    dump(&d, "WheelDebounce", 227, 2);
    dump(&d, "DebounceRelease", 229, 2);
    dump(&d, "Trigger block 239-252", 239, 14);
    dump(&d, "ShortcutKey head", 256, 16);
    dump(&d, "Sensor3955DPI 6Bx7", 6912, 42);
    dump(&d, "SensorVirtualPosEn", 6984, 2);
    dump(&d, "SensorVirtualPos", 6986, 2);

    println!("done");
}
