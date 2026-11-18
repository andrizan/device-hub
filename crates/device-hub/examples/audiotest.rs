//! Audio capture check — prints band values for 5 seconds.
//! Run it with `cargo run -p device-hub --example audiotest` and
//! check that the values are non-zero while music is playing.

fn main() {
    device_hub::audio::start();
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let b = device_hub::audio::bands();
        let max = b.iter().copied().fold(0f32, f32::max);
        let sum: f32 = b.iter().sum();
        println!(
            "max={max:.3} sum={sum:.3}  [{}]",
            b.iter()
                .map(|v| format!("{:.2}", v))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}
