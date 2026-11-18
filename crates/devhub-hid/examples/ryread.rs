//! Reads every RongYuan (VID 3151) device through the driver and prints a
//! summary. Read-only unless `--roundtrip` is given.
//!
//! `cargo run -p devhub-hid --example ryread`
//! `cargo run -p devhub-hid --example ryread -- --roundtrip`
//!
//! `--roundtrip` changes a few values, reads them back and restores the
//! originals: debounce, one key on each layer of the keyboard, and on the
//! mouse debounce, the last DPI stage and the DPI button. Leave the devices
//! alone while it runs.

use devhub_core::backend::{DeviceDriver, HubResult};
use devhub_core::model::*;
use std::error::Error;

fn setting(d: &dyn DeviceDriver, id: &str) -> HubResult<Option<SettingValue>> {
    Ok(d.read()?.settings.get(id).cloned())
}

/// Write `new`, check it reads back, write `old`, check again.
fn round_trip(
    d: &dyn DeviceDriver,
    what: &str,
    new: Change,
    old: Change,
    check: &dyn Fn(&DeviceState) -> String,
) {
    let run = || -> HubResult<(String, String, String)> {
        let before = check(&d.read()?);
        d.apply(&new)?;
        // The original goes back even when the read in between fails.
        let changed = d.read().map(|s| check(&s));
        let restored = d.apply(&old);
        let after = d.read().map(|s| check(&s));
        restored?;
        Ok((before, changed?, after?))
    };
    match run() {
        Ok((before, changed, after)) => {
            let ok = before == after && before != changed;
            println!(
                "  {} {what}: {before} -> {changed} -> {after}",
                if ok { "OK  " } else { "FAIL" }
            );
        }
        Err(e) => println!("  FAIL {what}: {e}"),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let roundtrip = std::env::args().any(|a| a == "--roundtrip");
    let hub = devhub_hid::connect()?;
    for info in hub.scan()? {
        if info.driver != DriverKind::Ry {
            continue;
        }
        // `--pid 4010` limits the run to one device.
        let args: Vec<String> = std::env::args().collect();
        if let Some(at) = args.iter().position(|a| a == "--pid") {
            if u16::from_str_radix(&args[at + 1], 16)? != info.pid {
                continue;
            }
        }
        println!(
            "{} [{}] {:04x}:{:04x}",
            info.name, info.model, info.vid, info.pid
        );
        let d = match hub.driver(&info.id) {
            Ok(d) => d,
            Err(e) => {
                println!("  open failed: {e}");
                continue;
            }
        };
        let i = d.info();
        println!(
            "  model={} firmware={:?} battery={:?} asleep={}",
            i.model, i.firmware, i.battery, i.asleep
        );
        let caps = d.caps();
        let st = match d.read() {
            Ok(st) => st,
            Err(e) => {
                println!("  read failed: {e}");
                continue;
            }
        };
        println!("  profile={} settings={:?}", st.profile.active, st.settings);
        if let (Some(kc), Some(ks)) = (&caps.keyboard, &st.keyboard) {
            for (layer, map) in ks.keymap.iter().enumerate() {
                let named: Vec<String> = kc
                    .layout
                    .iter()
                    .zip(map)
                    .filter_map(|(k, a)| match a {
                        KeyAction::Code(c) => Some(format!(
                            "{}={}",
                            k.label,
                            kc.code_label(*c)
                                .map(|(l, _)| l.to_string())
                                .unwrap_or_else(|| format!("{c:08x}"))
                        )),
                        _ => None,
                    })
                    .collect();
                println!(
                    "  layer {layer} ({} keys): {}",
                    named.len(),
                    named.join(" ")
                );
            }
            println!("  lighting={:?}", st.lighting.get(&0));
        }
        if let Some(m) = &st.mouse {
            println!(
                "  dpi={:?} active={} buttons={:?}",
                m.stages.iter().map(|s| (s.x, s.y)).collect::<Vec<_>>(),
                m.active_stage,
                m.buttons
            );
        }
        // `--set <id> <value>` writes one setting on every device that has it.
        let args: Vec<String> = std::env::args().collect();
        if let Some(at) = args.iter().position(|a| a == "--set") {
            let (id, raw) = (&args[at + 1], &args[at + 2]);
            if let Some(spec) = caps.setting(id) {
                let value = match &spec.kind {
                    SettingKind::Toggle => SettingValue::Bool(raw == "true"),
                    SettingKind::Slider { .. } => SettingValue::Num(raw.parse()?),
                    SettingKind::Choice { .. } => SettingValue::Choice(raw.parse()?),
                };
                let r = d.apply(&Change::Setting {
                    id: id.clone(),
                    value,
                });
                println!("  set {id}={raw}: {r:?} -> {:?}", setting(d.as_ref(), id));
            }
        }
        if !roundtrip {
            continue;
        }
        let d = d.as_ref();
        // Every declared setting: move it to a neighbouring value and back.
        for spec in &caps.settings {
            let Some(old) = setting(d, &spec.id)? else {
                println!("  SKIP {}: not readable", spec.id);
                continue;
            };
            let other = match (&spec.kind, &old) {
                (SettingKind::Toggle, v) => SettingValue::Bool(!v.as_bool()),
                (SettingKind::Slider { max, step, .. }, v) => {
                    let n = v.as_num();
                    SettingValue::Num(if n + step > *max { n - step } else { n + step })
                }
                (SettingKind::Choice { options }, v) => {
                    match options.iter().find(|o| o.value != v.as_choice()) {
                        Some(o) => SettingValue::Choice(o.value),
                        None => continue,
                    }
                }
            };
            let set = |value: SettingValue| Change::Setting {
                id: spec.id.clone(),
                value,
            };
            let id = spec.id.clone();
            round_trip(d, &spec.id, set(other), set(old), &move |s| {
                format!("{:?}", s.settings.get(&id))
            });
        }
        if let (Some(kc), Some(ks)) = (&caps.keyboard, &st.keyboard) {
            // F13: a key no application acts on, should it be pressed mid-test.
            let f13 = KeyAction::Code(0x0000_6800);
            for (layer, label) in [(0u8, "−"), (1u8, "Q")] {
                let key = kc.layout.iter().position(|k| k.label == label).unwrap();
                let old = ks.keymap[layer as usize][key];
                let set = |action| Change::KeyMap { layer, key, action };
                round_trip(
                    d,
                    &format!("layer {layer} key {label}"),
                    set(f13),
                    set(old),
                    &|s| {
                        format!(
                            "{:?}",
                            s.keyboard.as_ref().unwrap().keymap[layer as usize][key]
                        )
                    },
                );
            }
        }
        if let Some(m) = &st.mouse {
            let mut bumped = m.stages.clone();
            let last = bumped.len() - 1;
            bumped[last].x += 50;
            bumped[last].y = None;
            let set = |stages: Vec<DpiStage>| Change::DpiStages {
                stages,
                active: m.active_stage,
            };
            round_trip(
                d,
                "last DPI stage",
                set(bumped),
                set(m.stages.clone()),
                &|s| format!("{:?}", s.mouse.as_ref().unwrap().stages.last().map(|s| s.x)),
            );
            let slot = 5;
            let set = |action| Change::Button { slot, action };
            let f13 = ButtonAction::Key(KeyStroke {
                mods: 0,
                usage: 0x68,
            });
            round_trip(
                d,
                "DPI button",
                set(f13),
                set(m.buttons[slot].clone()),
                &|s| format!("{:?}", s.mouse.as_ref().unwrap().buttons[slot]),
            );
        }
    }
    Ok(())
}
