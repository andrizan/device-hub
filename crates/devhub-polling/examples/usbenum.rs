//! Probe to check USB device enumeration — `cargo run -p devhub-polling --example usbenum`
//! No elevation needed. Prints the result of `devices()`.
fn main() {
    for (n, root) in devhub_polling::discover::hubs() {
        println!("USBPcap{n} -> {root}");
    }
    for d in devhub_polling::discover::devices() {
        println!(
            "hub={:2} addr={:3} {:04x}:{:04x} mouse={} {:?}\n    {} | {}",
            d.hub,
            d.address,
            d.vid,
            d.pid,
            d.is_mouse,
            d.name,
            d.instance_id,
            d.location
        );
    }
}
