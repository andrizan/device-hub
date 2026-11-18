//! Lighting state dump — outputs LightingState + caps via read().
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
                "  caps area{}: effects={:?} br_max={} spd_max={} dirs={:?} zones={:?} per_key={}",
                a.id,
                a.effects.iter().map(|e| e.id).collect::<Vec<_>>(),
                a.brightness_max,
                a.speed_max,
                a.directions,
                a.zones,
                a.per_key
            );
        }
        if let Ok(s) = drv.read() {
            for (a, l) in &s.lighting {
                println!(
                    "  area{a}: on={} zone={:?} effect={} color=#{:06X} sel={} pal={:?} cc={:?} br={} spd={} dir={} per_key={} keys",
                    l.on,
                    l.zone_on,
                    l.effect,
                    l.color.to_hex(),
                    l.palette_sel,
                    l.palette.iter().map(|c| c.to_hex()).collect::<Vec<_>>(),
                    l.color_correction.map(|c| (c.r, c.g, c.b)),
                    l.brightness,
                    l.speed,
                    l.direction,
                    l.per_key.len()
                );
                for (k, c) in l.per_key.iter().take(10) {
                    println!("      key{k} = #{:06X}", c.to_hex());
                }
            }
        }
    }
}
