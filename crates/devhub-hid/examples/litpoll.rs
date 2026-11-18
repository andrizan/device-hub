//! Repeatedly calls `live_lighting(0)` through the same path as the app's worker and
//! prints the returned color map or the error. For isolating why live lighting does not show up in the UI.

fn main() {
    let backend = devhub_hid::connect().expect("connect");
    let devs = backend.scan().expect("scan");
    let kbd = devs
        .iter()
        .find(|d| d.vid == 0x1ca6 && d.pid == 0x3009)
        .expect("SU66 not found");
    let d = backend.driver(&kbd.id).expect("driver");
    println!("driver for {}", kbd.name);
    for i in 0..20 {
        let t = std::time::Instant::now();
        match d.live_lighting(0) {
            Ok(m) => {
                let n = m.len();
                let sample: Vec<_> = m.iter().take(6).map(|(k, c)| (*k, *c)).collect();
                println!(
                    "poll {i}: {n} keys lit in {:?}; first: {sample:?}",
                    t.elapsed()
                );
            }
            Err(e) => println!("poll {i}: ERR {e:?}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}
