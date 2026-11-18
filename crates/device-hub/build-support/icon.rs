//! Embeds the icon into the executable (loaded from the build script via `#[path]`).
//!
//! The GPUI executable has no `.rsrc` as it is. Then
//! Explorer, the taskbar, Alt+Tab and the Start menu shortcut
//! all fall back to the default Windows icon. The window icon is missing for the same reason
//! (`gpui-pre-windows`'s `load_icon` loads its own resource ID 1 with `LoadImageW`, so
//! without the resource it falls back to `HICON::default()` and the title bar and taskbar stay empty).
//!
//! So we call `rc.exe` to build a `.res` that makes `assets/icon.ico` the icon with ID 1, and
//! pass it to the linker. It uses the same artwork the Tauri version (`desktop/src-tauri/`) embeds.
//!
//! `rc.exe` only ships with the Windows SDK. It is searched for in this order: the `RC` environment variable, `PATH`, the SDK's default location.
//! If it is not found, only a warning is printed and the build continues (no icon is attached, but the build still passes).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `icon` is a path relative to the package root (e.g. `assets/icon.ico`).
pub fn embed(icon: &str) {
    println!("cargo:rerun-if-changed={icon}");
    println!("cargo:rerun-if-env-changed=RC");
    // Nothing emits for non-Windows targets at the moment, and there is no `rc.exe` there either, so we simply give up.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let source = manifest.join(icon);
    if !source.is_file() {
        println!("cargo:warning=icon not found: {}", source.display());
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe (Windows SDK) not found. The executable will have no icon.");
        return;
    };
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    // Write the `.rc` in ASCII. The icon location is passed with `/i` (via the command line), so
    // paths that contain Japanese characters (this working directory) do not depend on rc.exe's character encoding.
    let script = out.join("app-icon.rc");
    let resource = out.join("app-icon.res");
    let name = source.file_name().expect("icon file name");
    // The version info (VERSIONINFO) is embedded through the same .rc.
    // FileDescription becomes the display name in Task Manager and file properties.
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut vparts = version.split('.').filter_map(|p| p.parse::<u32>().ok());
    let fv = format!(
        "{},{},{},0",
        vparts.next().unwrap_or(0),
        vparts.next().unwrap_or(0),
        vparts.next().unwrap_or(0)
    );
    fs::write(
        &script,
        format!(
            concat!(
                "1 ICON \"{name}\"\r\n",
                "1 VERSIONINFO\r\n",
                "FILEVERSION {fv}\r\n",
                "PRODUCTVERSION {fv}\r\n",
                "FILEFLAGSMASK 0x3fL\r\n",
                "FILEFLAGS 0x0L\r\n",
                "FILEOS 0x40004L\r\n",
                "FILETYPE 0x1L\r\n",
                "FILESUBTYPE 0x0L\r\n",
                "BEGIN\r\n",
                "    BLOCK \"StringFileInfo\"\r\n",
                "    BEGIN\r\n",
                "        BLOCK \"040904b0\"\r\n",
                "        BEGIN\r\n",
                "            VALUE \"CompanyName\", \"Device HUB\"\r\n",
                "            VALUE \"FileDescription\", \"Device HUB\"\r\n",
                "            VALUE \"FileVersion\", \"{ver}\"\r\n",
                "            VALUE \"InternalName\", \"device-hub\"\r\n",
                "            VALUE \"OriginalFilename\", \"device-hub.exe\"\r\n",
                "            VALUE \"ProductName\", \"Device HUB\"\r\n",
                "            VALUE \"ProductVersion\", \"{ver}\"\r\n",
                "        END\r\n",
                "    END\r\n",
                "    BLOCK \"VarFileInfo\"\r\n",
                "    BEGIN\r\n",
                "        VALUE \"Translation\", 0x409, 1200\r\n",
                "    END\r\n",
                "END\r\n"
            ),
            name = name.to_string_lossy(),
            fv = fv,
            ver = version,
        ),
    )
    .expect("write app-icon.rc");
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/i")
        .arg(source.parent().expect("icon directory"))
        .arg("/fo")
        .arg(&resource)
        .arg(&script)
        .current_dir(&manifest)
        .status();
    match status {
        Ok(status) if status.success() => {
            // Pass it to the linker as an input. `link.exe` accepts a `.res` as is.
            println!("cargo:rustc-link-arg-bins={}", resource.display());
        }
        Ok(status) => panic!(
            "rc.exe failed (exit code {:?}): {}",
            status.code(),
            rc.display()
        ),
        Err(error) => panic!("cannot run rc.exe ({}): {error}", rc.display()),
    }
}

/// `RC` → `PATH` → the Windows SDK default location (`%ProgramFiles(x86)%\Windows Kits\10\bin\<version>\<arch>`).
fn find_rc() -> Option<PathBuf> {
    if let Some(explicit) = env::var_os("RC").map(PathBuf::from) {
        if explicit.is_file() {
            return Some(explicit);
        }
    }
    if let Some(path) = env::var_os("PATH") {
        if let Some(found) = env::split_paths(&path)
            .map(|dir| dir.join("rc.exe"))
            .find(|c| c.is_file())
        {
            return Some(found);
        }
    }
    // Depending on the build environment, `WindowsSdkVerBinPath` is set directly (VS developer prompt).
    if let Some(bin) = env::var_os("WindowsSdkVerBinPath").map(PathBuf::from) {
        for arch in arch_names() {
            let candidate = bin.join(arch).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let roots = ["ProgramFiles(x86)", "ProgramFiles"]
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from);
    for root in roots {
        let bin = root.join("Windows Kits").join("10").join("bin");
        let Ok(entries) = fs::read_dir(&bin) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        // Several SDKs may be installed. Pick the newest version (with string comparison 10.0.9 would sort after 10.0.26).
        versions.sort_by_key(|a| version_key(a));
        for version in versions.into_iter().rev() {
            for arch in arch_names() {
                let candidate = version.join(arch).join("rc.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// SDK bin directory names for the target CPU. Look at x64 first (the x64 rc.exe works for any target).
fn arch_names() -> &'static [&'static str] {
    match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => &["arm64", "x64"],
        Ok("x86") => &["x86", "x64"],
        _ => &["x64"],
    }
}

fn version_key(path: &Path) -> Vec<u32> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.split('.')
                .map(|part| part.parse().unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}
