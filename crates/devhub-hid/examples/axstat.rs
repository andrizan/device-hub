//! Observing raw KeyStatus (kind=3) values — isolating why the press display flickers.
//! Reads all rows every 100ms and prints the non-zero cells.

use std::time::Duration;

fn main() {
    let hub = match devhub_hid::connect() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("connect: {e}");
            return;
        }
    };
    let info = hub
        .scan()
        .unwrap_or_default()
        .into_iter()
        .find(|i| i.supported && i.vid == 0x1ca6 && i.pid == 0x3009);
    let Some(info) = info else {
        eprintln!("SU66 not found");
        return;
    };
    let d = match hub.driver(&info.id) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("driver: {e}");
            return;
        }
    };
    println!("device: {}", info.name);
    let n: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(30);
    for t in 0..n {
        match d.live_axis() {
            Ok(samples) => {
                let mut nz = Vec::new();
                for (i, s) in samples.iter().enumerate() {
                    if s.status != 0 || s.route > 0.05 {
                        nz.push(format!("k{i}:st={} r={:.2}", s.status, s.route));
                    }
                }
                println!("t{t:02} nz[{}]: {}", nz.len(), nz.join(" "));
            }
            Err(e) => println!("t{t:02} err {e}"),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
