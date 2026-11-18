//! Embeds the icon into the executable (without a `.rsrc`, Explorer and the taskbar both show the default picture).
//! `gpui-pre-windows`'s `load_icon` reads its own resource ID 1, so the window icon is
//! attached at the same time. The content lives in `build-support/icon.rs` (same as the reference implementation).

#[path = "build-support/icon.rs"]
mod icon;

fn main() {
    println!("cargo:rerun-if-changed=build-support/icon.rs");
    icon::embed("assets/icon.ico");
    // GPUI's large view functions can overflow the default 1MB stack in debug builds
    // (a `stack overflow` was seen on the settings page in the reference implementation). Enlarge the main thread reservation.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bin=device-hub=/STACK:16777216");
    }
}
