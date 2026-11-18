//! Thin HID transport layer.
//!
//! Report send/receive is gathered in one place, and each driver (xsyd.rs / mousefit.rs)
//! only writes the protocol semantics.

use devhub_core::backend::{HubError, HubResult};

/// Opens the HID enumerator. No special privileges are needed on Windows
/// (devices on a vendor-defined usage page can be read and written with user privileges).
pub fn open() -> HubResult<hidapi::HidApi> {
    hidapi::HidApi::new().map_err(|e| HubError::Io(format!("hidapi init: {e}")))
}

/// Report channel to one device.
///
/// TODO(backend): starting point for the implementation.
/// - Send/receive 64/65 bytes with the report ID first (`write` puts the report ID at the start)
/// - read with a timeout
/// - feature reports if needed (`send_feature_report` / `get_feature_report`)
///   — many mouse.fit models work with feature reports only
#[allow(dead_code)]
pub struct Channel {
    inner: Inner,
    /// Device IN report length (including the report ID). Assume 64 if 0.
    pub in_report_len: usize,
    /// OUT report length.
    pub out_report_len: usize,
}

enum Inner {
    /// `HidDevice` is Send but not Sync (the inner backend is not thread-safe).
    /// Wrap it in a Mutex to make it Sync. Calls come from a single driver worker, so there is no contention.
    Hid(std::sync::Mutex<hidapi::HidDevice>),
    #[cfg(test)]
    Mock(std::sync::Mutex<MockIo>),
}

/// Send/receive stub for tests: send is recorded, recv returns from the queue in order.
#[cfg(test)]
#[derive(Default)]
pub struct MockIo {
    pub sent: Vec<Vec<u8>>,
    pub replies: std::collections::VecDeque<Vec<u8>>,
    fail_send: Option<Vec<u8>>,
}

#[allow(dead_code)]
impl Channel {
    pub fn open(dev: hidapi::HidDevice, in_len: usize, out_len: usize) -> Self {
        Self {
            inner: Inner::Hid(std::sync::Mutex::new(dev)),
            in_report_len: in_len.max(1),
            out_report_len: out_len.max(1),
        }
    }

    #[cfg(test)]
    pub fn mock() -> Self {
        Self {
            inner: Inner::Mock(std::sync::Mutex::new(MockIo::default())),
            in_report_len: 64,
            out_report_len: 64,
        }
    }

    /// For tests: queues the replies that recv returns in order.
    #[cfg(test)]
    pub fn mock_reply(&self, reply: &[u8]) {
        if let Inner::Mock(m) = &self.inner {
            m.lock()
                .unwrap_or_else(|p| p.into_inner())
                .replies
                .push_back(reply.to_vec());
        }
    }

    /// For tests: every body that came to send.
    #[cfg(test)]
    pub fn mock_sent(&self) -> Vec<Vec<u8>> {
        match &self.inner {
            Inner::Mock(m) => m.lock().unwrap_or_else(|p| p.into_inner()).sent.clone(),
            _ => vec![],
        }
    }

    /// For tests: makes only the next send whose header matches fail with an I/O error.
    #[cfg(test)]
    pub fn mock_fail_send(&self, prefix: &[u8]) {
        if let Inner::Mock(m) = &self.inner {
            m.lock().unwrap_or_else(|p| p.into_inner()).fail_send = Some(prefix.to_vec());
        }
    }

    /// Sends `body` as an OUT report with report ID 0 first.
    /// Padding is zero-filled.
    pub fn send(&self, body: &[u8]) -> HubResult<()> {
        match &self.inner {
            Inner::Hid(dev) => {
                let mut buf = vec![0u8; self.out_report_len];
                let n = body.len().min(buf.len() - 1);
                buf[1..1 + n].copy_from_slice(&body[..n]);
                dev.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .write(&buf)
                    .map_err(|e| HubError::Io(format!("hid write: {e}")))?;
                Ok(())
            }
            #[cfg(test)]
            Inner::Mock(m) => {
                let mut io = m.lock().unwrap_or_else(|p| p.into_inner());
                io.sent.push(body.to_vec());
                if io
                    .fail_send
                    .as_ref()
                    .is_some_and(|prefix| body.starts_with(prefix))
                {
                    io.fail_send = None;
                    return Err(HubError::Io("mock: write failed".into()));
                }
                Ok(())
            }
        }
    }

    /// Numbered report: sends `body` as an OUT report with report ID `rid`
    /// (buf[0]=rid). For devices that have report numbers (WalkPlay's 0x4B etc.).
    pub fn send_report(&self, rid: u8, body: &[u8]) -> HubResult<()> {
        match &self.inner {
            Inner::Hid(dev) => {
                let mut buf = vec![0u8; self.out_report_len];
                buf[0] = rid;
                let n = body.len().min(buf.len() - 1);
                buf[1..1 + n].copy_from_slice(&body[..n]);
                dev.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .write(&buf)
                    .map_err(|e| HubError::Io(format!("hid write: {e}")))?;
                Ok(())
            }
            #[cfg(test)]
            Inner::Mock(m) => {
                let mut f = vec![rid];
                f.extend_from_slice(body);
                m.lock().unwrap_or_else(|p| p.into_inner()).sent.push(f);
                Ok(())
            }
        }
    }

    /// One IN report within `timeout_ms` (drops the report ID and returns only the body).
    pub fn recv(&self, timeout_ms: i32) -> HubResult<Vec<u8>> {
        match &self.inner {
            Inner::Hid(dev) => {
                let mut buf = vec![0u8; self.in_report_len];
                let n = dev
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .read_timeout(&mut buf, timeout_ms)
                    .map_err(|e| HubError::Io(format!("hid read: {e}")))?;
                if n == 0 {
                    return Err(HubError::Protocol("read timeout".into()));
                }
                // For an unnumbered report buf[0] is the start of the data. For a numbered one buf[0] is the report ID.
                // Telling them apart is up to the caller's protocol, so return it as is.
                Ok(buf[..n].to_vec())
            }
            #[cfg(test)]
            Inner::Mock(m) => m
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .replies
                .pop_front()
                .ok_or_else(|| HubError::Protocol("mock: no reply".into())),
        }
    }

    /// Sends a 64B feature report (report ID 0). Used for every mouse.fit command.
    pub fn send_feature(&self, body: &[u8; 64]) -> HubResult<()> {
        match &self.inner {
            Inner::Hid(dev) => {
                let mut buf = [0u8; 65];
                buf[1..].copy_from_slice(body);
                dev.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .send_feature_report(&buf)
                    .map_err(|e| HubError::Io(format!("hid feature write: {e}")))?;
                Ok(())
            }
            #[cfg(test)]
            Inner::Mock(m) => {
                m.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .sent
                    .push(body.to_vec());
                Ok(())
            }
        }
    }

    /// Reads a 64B feature report (report ID 0). Returns 65B including the report ID.
    /// In `get_feature_report` the request→response round trip is one transaction on the kernel side,
    /// so the timeout is OS dependent (usually ~1s). On failure it returns Err and the caller retries.
    pub fn get_feature(&self) -> HubResult<[u8; 65]> {
        match &self.inner {
            Inner::Hid(dev) => {
                let mut buf = [0u8; 65];
                let n = dev
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get_feature_report(&mut buf)
                    .map_err(|e| HubError::Io(format!("hid feature read: {e}")))?;
                if n < 2 {
                    return Err(HubError::Protocol("empty feature report".into()));
                }
                Ok(buf)
            }
            #[cfg(test)]
            Inner::Mock(m) => {
                let mut buf = [0u8; 65];
                if let Some(r) = m
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .replies
                    .pop_front()
                {
                    let n = r.len().min(64);
                    buf[1..1 + n].copy_from_slice(&r[..n]);
                }
                Ok(buf)
            }
        }
    }
}
