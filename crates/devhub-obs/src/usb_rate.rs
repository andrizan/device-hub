//! A thin boundary mapping USBPcap measurements to OBS display strings. The measurement itself is devhub_polling —
//! it yields the same values as the app side (test screen).
pub(crate) use devhub_polling::{Capture, Snapshot, Status, Target};

/// Transfers over the last 1 s, or the status, on one line.
pub(crate) fn label(s: &Snapshot) -> String {
    match s.status {
        Status::Waiting => "Waiting for mouse input".into(),
        Status::Live => format!("USB   {} Hz", s.hz),
        Status::DriverMissing { .. } => "USBPcap driver / hub not found".into(),
        Status::AccessDenied { .. } => "USBPcap access denied".into(),
        Status::HubBusy { .. } => "USBPcap hub already in use".into(),
        Status::Disconnected { .. } | Status::Failed { .. } => "USB capture stopped".into(),
    }
}
