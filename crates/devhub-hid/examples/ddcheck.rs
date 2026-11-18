//! Diagnostic that only dumps the direct_drive flag of caps.lighting.
fn main() {
    let hub = match devhub_hid::connect() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("connect: {e}");
            return;
        }
    };
    for info in hub.scan().unwrap_or_default() {
        if !info.supported {
            continue;
        }
        let Ok(drv) = hub.driver(&info.id) else {
            continue;
        };
        let caps = drv.caps();
        if caps.lighting.is_empty() {
            continue;
        }
        println!("== {} ({})", info.name, info.id);
        for a in &caps.lighting {
            println!(
                "  area{} name={} dd={} per_key={} zones={:?}",
                a.id, a.name, a.direct_drive, a.per_key, a.zones
            );
        }
    }
}
