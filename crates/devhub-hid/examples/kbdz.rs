// Compare deadzone write → read back on an RT key (W = r2c2) and a normal key (Esc = r1c0).
use devhub_core::Change;

fn main() {
    let hub = devhub_hid::connect().unwrap();
    for info in hub.scan().unwrap_or_default() {
        if !info.supported {
            continue;
        }
        let drv = match hub.driver(&info.id) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if drv.caps().keyboard.is_none() {
            continue;
        }
        println!("== {}", info.name);
        let st = drv.read().unwrap();
        let kb = st.keyboard.as_ref().unwrap();
        // W (index 16 = r2c2) and Esc (index 0 = r1c0)
        for &i in &[0usize, 16] {
            let p = kb.perf[i];
            println!(
                "key{i}: mode={:?} tdz={} bdz={} rtF={}",
                p.mode, p.top_deadzone, p.bottom_deadzone, p.rt_first
            );
            let mut np = p;
            np.top_deadzone = 0.5;
            np.bottom_deadzone = 0.4;
            match drv.apply(&Change::KeyPerf {
                keys: vec![i],
                perf: np,
            }) {
                Ok(()) => {}
                Err(e) => {
                    println!("  apply err {e:?}");
                    continue;
                }
            }
            let st2 = drv.read().unwrap();
            let p2 = st2.keyboard.as_ref().unwrap().perf[i];
            println!(
                "  after: tdz={} bdz={}",
                p2.top_deadzone, p2.bottom_deadzone
            );
            // Restore
            let mut p3 = p2;
            p3.top_deadzone = p.top_deadzone;
            p3.bottom_deadzone = p.bottom_deadzone;
            drv.apply(&Change::KeyPerf {
                keys: vec![i],
                perf: p3,
            })
            .unwrap();
        }
        break;
    }
}
