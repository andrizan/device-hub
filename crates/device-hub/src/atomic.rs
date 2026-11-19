//! Shared helper: write to a unique temp file in the same directory, then replace the target.
//! A failure mid-write never destroys the old file, and since no fixed-name temp file
//! is shared, concurrent saves do not collide. Failures are returned to the caller.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp_path(path: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let stem = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("tmp");
    path.with_file_name(format!(".{stem}.tmp.{pid}.{n}"))
}

/// Atomically write `contents` to `path` (temp file → rename).
pub fn write(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = tmp_path(path);
    let r = std::fs::write(&tmp, contents).and_then(|_| std::fs::rename(&tmp, path));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}
