//! Measures the real polling rate through USBPcap — shared component of the OBS plugin and the Device HUB app.
//!
//! A mouse sends reports only "while it moves", so the number of completed interrupt IN
//! transfers of the target ≈ the actual device polling rate (for a wireless dongle, the receiver -> PC leg).
//! USBPcap streams the records as classic little-endian pcap (LINKTYPE_USBPCAP).
//! Record format: <https://desowin.org/usbpcap/captureformat.html>
//!
//! Usage:
//! 1. `Capture::start(target)` — opens `\\.\USBPcap{hub}` on a separate thread and starts reading.
//!    Opening a hub is exclusive (`share_mode(0)`): only one process in total can hold a given hub.
//! 2. `snapshot()` — gets the transfers in the last second, the maximum 1-second window seen so far, and the status.
//! 3. `stop()` / drop — releases the blocking read, then joins the worker.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Measurement target. hub/device can change on every connection — the persistent identity of a unit
/// (VID/PID, serial, etc.) is not kept here; the caller associates it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    /// USBPcap hub number (the n of `\\.\USBPcap{n}`).
    pub hub: u16,
    /// Current USB device address (1-127). Changes on replug.
    pub device: u8,
    /// Interrupt IN endpoint used for movement (including the IN bit, e.g. 0x81).
    pub endpoint: u8,
    /// Report ID to filter a shared endpoint by. None = count everything.
    pub report: Option<u8>,
}

/// Measurement status. Error states are terminal and keep the OS code (GetLastError).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Just started — no transfer of the target received yet.
    Waiting,
    /// Receiving. `Snapshot::hz` is the count over the last second (0 while idle).
    Live,
    /// USBPcap driver not installed, or no hub with that number exists.
    DriverMissing { os: Option<i32> },
    /// No permission to open the hub in read/write mode (must run as administrator).
    AccessDenied { os: Option<i32> },
    /// Another process/source holds the same hub.
    HubBusy { os: Option<i32> },
    /// The stream broke mid-capture (hub removed, driver stopped, etc.).
    Disconnected { os: Option<i32> },
    /// Any other failure (invalid stream, other I/O errors).
    Failed { os: Option<i32> },
}

impl Status {
    /// Whether this is a terminal error state.
    pub fn is_error(self) -> bool {
        matches!(
            self,
            Self::DriverMissing { .. }
                | Self::AccessDenied { .. }
                | Self::HubBusy { .. }
                | Self::Disconnected { .. }
                | Self::Failed { .. }
        )
    }
}

/// One set of values from `snapshot()`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub status: Status,
    /// Number of target transfers completed in the last second (0 when idle after receiving).
    pub hz: u32,
    /// Max count in any 1-second window `(t-1s, t]` observed since start.
    /// Updated on every receive rather than on the display refresh interval, so peaks in between are not missed.
    pub peak_hz: u32,
}

/// Whether USB measurement can be tried in this environment (always fails on non-Windows).
pub fn supported() -> bool {
    cfg!(windows)
}

/// One USB device offered as a measurement target (returned by `discover::devices`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UsbDevice {
    /// The n of `\\.\USBPcap{n}`. 0 means it could not be matched to a root hub.
    pub hub: u16,
    /// Current USB address. Changes on replug or port change.
    pub address: u8,
    pub vid: u16,
    pub pid: u16,
    /// Product name the device reports itself (BusReportedDeviceDesc), or FriendlyName if absent.
    pub name: String,
    /// PnP instance ID (`USB\VID_xxxx&PID_yyyy\<serial>`). Used to re-resolve after a reconnect.
    pub instance_id: String,
    /// Connection location such as `Port_#0002.Hub_#0001`.
    pub location: String,
    /// Has a mouse HID collection (UsagePage=1, Usage=2).
    pub is_mouse: bool,
}

impl UsbDevice {
    /// Builds the `Target` to start a measurement with.
    pub fn target(&self, endpoint: u8, report: Option<u8>) -> Target {
        Target {
            hub: self.hub,
            device: self.address,
            endpoint,
            report,
        }
    }

    /// Short description for the picker.
    pub fn label(&self) -> String {
        format!(
            "{name} ({vid:04x}:{pid:04x}) {loc}",
            name = self.name,
            vid = self.vid,
            pid = self.pid,
            loc = self.location
        )
    }
}

#[cfg(windows)]
pub mod discover;

#[cfg(not(windows))]
/// Non-Windows stub of `discover`. Always returns empty.
pub mod discover {
    use super::UsbDevice;
    use std::io;
    pub fn devices() -> Vec<UsbDevice> {
        Vec::new()
    }
    pub fn hubs() -> Vec<(u16, String)> {
        Vec::new()
    }
    pub fn hub_symlink(_: u16) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "USBPcap requires Windows",
        ))
    }
    pub fn resolve(_: &str, _: u16, _: u16) -> Option<UsbDevice> {
        None
    }
}

#[derive(Default)]
struct Samples {
    /// Timestamps of the target records in the last second (capture time in µs).
    stamps: VecDeque<u64>,
    seen: bool,
    peak: u32,
    /// Terminal error state (the worker has already finished once this is Some).
    error: Option<Status>,
}

impl Samples {
    fn expire(&mut self, now: u64) {
        let cutoff = now.saturating_sub(1_000_000);
        while self.stamps.front().is_some_and(|&t| t <= cutoff) {
            self.stamps.pop_front();
        }
    }

    fn snapshot(&mut self, now: u64) -> Snapshot {
        self.expire(now);
        Snapshot {
            status: self.error.unwrap_or(if self.seen {
                Status::Live
            } else {
                Status::Waiting
            }),
            hz: self.stamps.len() as u32,
            peak_hz: self.peak,
        }
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid USBPcap stream")
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Reads the pcap stream and counts only the target's successful interrupt IN completions (with data,
/// and matching the Report ID when one is given). Timestamps use the capture time.
fn consume(
    mut input: impl Read,
    target: Target,
    stop: &AtomicBool,
    samples: &Mutex<Samples>,
) -> io::Result<()> {
    let mut global = [0; 24];
    input.read_exact(&mut global)?;
    if global[..8] != [0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0] || global[20..] != 249u32.to_le_bytes() {
        return Err(invalid());
    }
    let mut packet = [0; 1024];
    while !stop.load(Ordering::Acquire) {
        let mut header = [0; 16];
        input.read_exact(&mut header)?;
        let size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        if !(27..=packet.len()).contains(&size) {
            // Transfers of a non-target size (short control records, large
            // reports of other devices, etc.) must not stop the measurement — discard the body and move on.
            io::copy(&mut input.by_ref().take(size as u64), &mut io::sink())?;
            continue;
        }
        let data = packet.get_mut(..size).ok_or_else(invalid)?;
        input.read_exact(data)?;
        let offset = u16::from_le_bytes([data[0], data[1]]) as usize;
        if offset < 27
            || offset >= size
            || data[10..14] != [0; 4]
            || data[16] & 1 == 0
            || data[19..21] != u16::from(target.device).to_le_bytes()
            || data[21] != target.endpoint
            || data[22] != 1
            || data[23..27] == [0; 4]
            || target.report.is_some_and(|id| id != data[offset])
        {
            continue;
        }
        let sec = u32::from_le_bytes(header[..4].try_into().unwrap());
        let us = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let stamp = u64::from(sec) * 1_000_000 + u64::from(us);
        let mut s = samples.lock().unwrap_or_else(|e| e.into_inner());
        s.expire(stamp);
        s.seen = true;
        s.stamps.push_back(stamp);
        // Fold the count of (t - 1 s, t] into the peak on every receive — taking
        // the peak only on the display interval would miss peaks between updates.
        s.peak = s.peak.max(s.stamps.len() as u32);
    }
    Ok(())
}

/// Stage at which a failure occurred. The meaning of the OS code differs between open/ioctl and the receive loop.
#[derive(Clone, Copy, Debug)]
enum Stage {
    Open,
    Run,
}

/// Maps an `io::Error` to a terminal state for the user.
fn status_of(err: &io::Error, stage: Stage) -> Status {
    let os = err.raw_os_error();
    match (stage, os) {
        // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND — no driver, or the wrong number.
        (Stage::Open, Some(2 | 3)) => Status::DriverMissing { os },
        // ERROR_ACCESS_DENIED — opening the hub for read/write needs admin rights.
        (Stage::Open, Some(5)) => Status::AccessDenied { os },
        // ERROR_SHARING_VIOLATION — opened with share_mode(0), so only one holder at a time.
        (Stage::Open, Some(32)) => Status::HubBusy { os },
        // Disconnect while receiving: EOF (the driver stopped the capture) / NOT_CONNECTED / GEN_FAILURE.
        (Stage::Run, Some(2 | 3 | 31 | 1167)) => Status::Disconnected { os },
        (Stage::Run, _) if err.kind() == io::ErrorKind::UnexpectedEof => {
            Status::Disconnected { os }
        }
        _ => Status::Failed { os },
    }
}

/// Diagnostic log — if `DEVHUB_USB_LOG` holds a file path, records the worker's
/// open/run/exit and error transitions (does nothing when unset).
#[cfg(windows)]
fn diag(msg: &str) {
    let Ok(path) = std::env::var("DEVHUB_USB_LOG") else {
        return;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "[{stamp}] {msg}");
    }
}

#[cfg(windows)]
fn capture(
    target: Target,
    stop: &AtomicBool,
    samples: &Mutex<Samples>,
) -> Result<(), (Stage, io::Error)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let stage = |e: io::Error| (Stage::Open, e);
    if target.hub == 0
        || !(1..=127).contains(&target.device)
        || !(0x81..=0x8f).contains(&target.endpoint)
    {
        return Err((
            Stage::Open,
            io::Error::new(io::ErrorKind::InvalidInput, "invalid USB target"),
        ));
    }
    let path = format!(r"\\.\USBPcap{}", target.hub);
    let open = |share: u32| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(share)
            .open(&path)
    };
    let file = match open(0) {
        Ok(f) => f,
        // Verified on a real device: USBPcap's driver rejects a second open of a held hub with
        // STATUS_ACCESS_DENIED (5), not a sharing violation (32). Re-open with sharing allowed to
        // tell them apart — 32 means another process holds it (HubBusy), 5 a real lack of permission.
        Err(e) if e.raw_os_error() == Some(5) => {
            return Err((
                Stage::Open,
                match open(0x3) {
                    Err(p) if p.raw_os_error() == Some(32) => io::Error::from_raw_os_error(32),
                    _ => e,
                },
            ));
        }
        Err(e) => return Err((Stage::Open, e)),
    };
    // USBPcapDriver/include/USBPcap.h: FILE_DEVICE_UNKNOWN, METHOD_BUFFERED.
    let ioctl = |code, input: &[u8]| {
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    };
    ioctl(0x0022_6010, &64u32.to_le_bytes()).map_err(stage)?; // snapshot length
    ioctl(0x0022_6000, &(1u32 << 20).to_le_bytes()).map_err(stage)?; // buffer size
    let mut filter = [0u8; 17];
    filter[usize::from(target.device / 8)] |= 1 << (target.device % 8);
    ioctl(0x0022_e004, &filter).map_err(stage)?; // address filter
    consume(
        io::BufReader::with_capacity(65536, file),
        target,
        stop,
        samples,
    )
    .map_err(|e| (Stage::Run, e))
}

#[cfg(not(windows))]
fn capture(_: Target, _: &AtomicBool, _: &Mutex<Samples>) -> Result<(), (Stage, io::Error)> {
    Err((
        Stage::Open,
        io::Error::new(io::ErrorKind::Unsupported, "USBPcap requires Windows"),
    ))
}

/// Short scan that tallies the target device's interrupt IN completions per endpoint.
/// Used to auto-detect the EP number. Holds the hub while running and needs read/write rights on the hub.
/// Returns a list of (EP, count) sorted by count, descending.
#[cfg(windows)]
pub fn scan_interrupt(
    hub: u16,
    device: u8,
    dur: std::time::Duration,
) -> io::Result<Vec<(u8, u32)>> {
    use std::collections::BTreeMap;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    if hub == 0 || !(1..=127).contains(&device) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid USB target",
        ));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(format!(r"\\.\USBPcap{hub}"))?;
    let ioctl = |code, input: &[u8]| {
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    };
    ioctl(0x0022_6010, &64u32.to_le_bytes())?;
    ioctl(0x0022_6000, &(1u32 << 20).to_le_bytes())?;
    let mut filter = [0u8; 17];
    filter[usize::from(device / 8)] |= 1 << (device % 8);
    ioctl(0x0022_e004, &filter)?;
    let counts = Arc::new(Mutex::new(BTreeMap::<u8, u32>::new()));
    let tally = counts.clone();
    let worker = std::thread::spawn(move || {
        let mut input = io::BufReader::with_capacity(65536, file);
        let mut global = [0; 24];
        if input.read_exact(&mut global).is_err() {
            return;
        }
        let mut packet = [0; 1024];
        loop {
            let mut header = [0; 16];
            if input.read_exact(&mut header).is_err() {
                return;
            }
            let size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
            if !(27..=packet.len()).contains(&size) {
                // Discard transfers of a non-target size and move on (same reason as in the measurement).
                if io::copy(&mut input.by_ref().take(size as u64), &mut io::sink()).is_err() {
                    return;
                }
                continue;
            }
            let data = packet.get_mut(..size).unwrap();
            if input.read_exact(data).is_err() {
                return;
            }
            // Count only successful interrupt IN completions (with data) at the target address.
            if data[10..14] == [0; 4]
                && data[16] & 1 == 1
                && data[19..21] == u16::from(device).to_le_bytes()
                && (0x81..=0x8f).contains(&data[21])
                && data[22] == 1
                && data[23..27] != [0; 4]
            {
                *tally
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(data[21])
                    .or_default() += 1;
            }
        }
    });
    std::thread::sleep(dur);
    while !worker.is_finished() {
        unsafe {
            windows_sys::Win32::System::IO::CancelSynchronousIo(worker.as_raw_handle());
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let _ = worker.join();
    let mut list: Vec<(u8, u32)> = counts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(ep, n)| (*ep, *n))
        .collect();
    list.sort_by_key(|e| std::cmp::Reverse(e.1));
    Ok(list)
}

#[cfg(not(windows))]
pub fn scan_interrupt(_: u16, _: u8, _: std::time::Duration) -> io::Result<Vec<(u8, u32)>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "USBPcap requires Windows",
    ))
}

/// Measurement handle. `start` launches the worker, `snapshot` gives the current values, `stop`/drop ends it.
pub struct Capture {
    pub target: Target,
    samples: Arc<Mutex<Samples>>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Capture {
    pub fn start(target: Target) -> Self {
        let samples = Arc::new(Mutex::new(Samples::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let state = samples.clone();
        let flag = cancel.clone();
        let result = std::thread::Builder::new()
            .name("devhub-usb-rate".into())
            .spawn(move || {
                // A disconnect while receiving is often transient (device re-enumeration,
                // driver-side drops, etc.) — reopen the handle and carry on.
                // Give up and go to a terminal state only if it keeps failing right away.
                let mut quick_retries = 0u32;
                loop {
                    let began = std::time::Instant::now();
                    #[cfg(windows)]
                    diag("capture: start");
                    match capture(target, &flag, &state) {
                        Ok(_) => break,
                        Err((stage, err)) => {
                            // A read interrupted by our own stop request is not an error.
                            if flag.load(Ordering::Acquire) {
                                break;
                            }
                            #[cfg(windows)]
                            diag(&format!(
                                "capture: {stage:?} error {} (os {:?})",
                                err,
                                err.raw_os_error()
                            ));
                            // A failure at the Open stage (permission, held by another, not installed) is terminal immediately.
                            if matches!(stage, Stage::Open) || quick_retries >= 20 {
                                state.lock().unwrap_or_else(|e| e.into_inner()).error =
                                    Some(status_of(&err, stage));
                                break;
                            }
                            quick_retries = if began.elapsed() < std::time::Duration::from_secs(2) {
                                quick_retries + 1
                            } else {
                                0
                            };
                            std::thread::sleep(std::time::Duration::from_millis(300));
                        }
                    }
                }
            });
        if let Err(err) = &result {
            samples.lock().unwrap_or_else(|e| e.into_inner()).error = Some(Status::Failed {
                os: err.raw_os_error(),
            });
        }
        Self {
            target,
            samples,
            cancel,
            worker: result.ok(),
        }
    }

    /// Current values. After an error exit it still returns the last observed values (`status` becomes a terminal state).
    pub fn snapshot(&self) -> Snapshot {
        self.samples
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot(now_us())
    }

    /// Stops the measurement. So that it does not wait in a blocking read while idle,
    /// cancels the worker's synchronous I/O before joining (normally returns in a few ms).
    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            #[cfg(windows)]
            while !worker.is_finished() {
                // An idle mouse blocks ReadFile indefinitely. To also absorb the race between the stop check and
                // the next read, keep cancelling repeatedly until it finishes.
                use std::os::windows::io::AsRawHandle;
                unsafe {
                    windows_sys::Win32::System::IO::CancelSynchronousIo(worker.as_raw_handle());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let _ = worker.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test builder that appends one record to a pcap stream.
    struct Pcap(Vec<u8>);

    impl Pcap {
        fn new() -> Self {
            let mut stream = vec![0; 24];
            stream[..8].copy_from_slice(&[0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0]);
            stream[20] = 249;
            Self(stream)
        }

        #[expect(clippy::too_many_arguments)]
        fn record(
            &mut self,
            sec: u32,
            us: u32,
            dev: u8,
            ep: u8,
            info: u8,
            transfer: u8,
            status: u8,
            len: u8,
            report: u8,
        ) -> &mut Self {
            let mut header = [0u8; 16];
            header[..4].copy_from_slice(&sec.to_le_bytes());
            header[4..8].copy_from_slice(&us.to_le_bytes());
            header[8] = 28;
            header[12] = 28;
            let mut p = [0; 28];
            p[0] = 27; // header_len (interrupt IN header 27B -> payload starts at 27)
            p[10] = status;
            p[16] = info;
            p[19] = dev;
            p[21] = ep;
            p[22] = transfer;
            p[23] = len;
            p[27] = report;
            self.0.extend(header);
            self.0.extend(p);
            self
        }

        /// A record of arbitrary size (payload zero-filled).
        fn raw(&mut self, sec: u32, us: u32, size: u32) -> &mut Self {
            let mut header = [0u8; 16];
            header[..4].copy_from_slice(&sec.to_le_bytes());
            header[4..8].copy_from_slice(&us.to_le_bytes());
            header[8..12].copy_from_slice(&size.to_le_bytes());
            header[12..16].copy_from_slice(&size.to_le_bytes());
            self.0.extend(header);
            self.0.extend(vec![0; size as usize]);
            self
        }

        /// `count` successful interrupt IN records of the target, `interval_us` apart.
        fn motion(&mut self, sec: u32, count: u32, interval_us: u32) -> &mut Self {
            for i in 0..count {
                self.record(sec, i * interval_us, 5, 129, 1, 1, 0, 1, 0);
            }
            self
        }

        fn run(&self, target: Target) -> Samples {
            let samples = Mutex::new(Samples::default());
            let _ = consume(&self.0[..], target, &AtomicBool::new(false), &samples);
            samples.into_inner().unwrap()
        }
    }

    const TARGET: Target = Target {
        hub: 1,
        device: 5,
        endpoint: 129,
        report: None,
    };

    #[test]
    fn counts_only_target_interrupt_in_completions_with_data() {
        let mut s = Pcap::new();
        s.motion(10, 3, 1000)
            // Excluded: other address, other EP, request side (info bit0=0), non-interrupt, failure, empty data.
            .record(10, 3000, 6, 129, 1, 1, 0, 1, 0)
            .record(10, 3001, 5, 130, 1, 1, 0, 1, 0)
            .record(10, 3002, 5, 129, 0, 1, 0, 1, 0)
            .record(10, 3003, 5, 129, 1, 3, 0, 1, 0)
            .record(10, 3004, 5, 129, 1, 1, 1, 1, 0)
            .record(10, 3005, 5, 129, 1, 1, 0, 0, 0);
        let mut samples = s.run(TARGET);
        let snap = samples.snapshot(10_500_000);
        assert_eq!(snap.status, Status::Live);
        assert_eq!(snap.hz, 3);
        assert_eq!(snap.peak_hz, 3);
    }

    #[test]
    fn oversized_and_short_records_are_skipped_not_fatal() {
        // Even if other devices on the hub emit large transfers (64B reports, etc.) or short
        // control records, the measurement does not die — it used to end as Failed on a record
        // beyond the buffer (it stopped when procmon's HID writes got mixed in as another app came to the foreground).
        let mut s = Pcap::new();
        s.record(10, 0, 5, 129, 1, 1, 0, 1, 0)
            .raw(10, 100, 91) // transfer exceeding the 64B buffer
            .raw(10, 200, 10) // record shorter than the header
            .raw(10, 300, 0) // empty
            .record(10, 400, 5, 129, 1, 1, 0, 1, 0)
            .record(10, 500, 5, 129, 1, 1, 0, 1, 0);
        let samples = Mutex::new(Samples::default());
        // Reads all the records and ends with an UnexpectedEof at the end of the stream.
        // A failure on an out-of-size record would give InvalidData; stopping midway would lower hz.
        let err = consume(&s.0[..], TARGET, &AtomicBool::new(false), &samples).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        let mut samples = samples.into_inner().unwrap();
        let snap = samples.snapshot(10_500_000);
        assert_eq!(snap.status, Status::Live);
        assert_eq!(snap.hz, 3);
    }

    #[test]
    fn report_id_filter_selects_matching_records() {
        let target = Target {
            report: Some(2),
            ..TARGET
        };
        let mut s = Pcap::new();
        s.record(10, 0, 5, 129, 1, 1, 0, 1, 2)
            .record(10, 1, 5, 129, 1, 1, 0, 1, 3)
            .record(10, 2, 5, 129, 1, 1, 0, 1, 0)
            .record(10, 3, 5, 129, 1, 1, 0, 1, 2);
        let mut samples = s.run(target);
        assert_eq!(samples.snapshot(10_500_000).hz, 2);
    }

    #[test]
    fn simulated_rates_fill_one_second_window() {
        for (rate, interval_us) in [(125u32, 8_000u32), (1000, 1000), (8000, 125)] {
            let mut s = Pcap::new();
            s.motion(10, rate, interval_us);
            let mut samples = s.run(TARGET);
            let snap = samples.snapshot(11_000_000);
            // The window is (t-1s, t], so rate - 1 records fit in the last second.
            assert_eq!(snap.hz, rate - 1, "rate {rate}");
            // The peak is the window at each receive — the whole span (~1 s) fits, so rate records.
            assert_eq!(snap.peak_hz, rate);
        }
    }

    #[test]
    fn one_second_boundary_excludes_older_records() {
        let mut s = Pcap::new();
        s.record(10, 0, 5, 129, 1, 1, 0, 1, 0) // t = 10.000000 (just outside the boundary)
            .record(10, 1, 5, 129, 1, 1, 0, 1, 0); // t = 10.000001
        let mut samples = s.run(TARGET);
        assert_eq!(samples.snapshot(10_500_000).hz, 2);
        // now = 11.000000 -> the window is (10.000000, 11.000000]. Exactly 10.000000 is not included.
        assert_eq!(samples.snapshot(11_000_000).hz, 1);
        // Once everything has expired, it is 0 Hz while idle.
        assert_eq!(samples.snapshot(12_000_000).hz, 0);
    }

    #[test]
    fn peak_keeps_densest_window_during_run() {
        let mut s = Pcap::new();
        // Slow span -> a dense 1-second span (8000 records) -> one isolated record.
        s.motion(10, 10, 100_000);
        for i in 0..8000 {
            s.record(11, i * 125, 5, 129, 1, 1, 0, 1, 0);
        }
        s.record(12, 500_000, 5, 129, 1, 1, 0, 1, 0);
        let mut samples = s.run(TARGET);
        // now = 13.0004 -> only the single record at 12.5 is in the last second, but the peak remains.
        let snap = samples.snapshot(13_000_400);
        assert_eq!(snap.peak_hz, 8000);
        assert_eq!(snap.hz, 1);
        assert_eq!(snap.status, Status::Live);
    }

    #[test]
    fn short_record_and_truncated_stream_are_errors() {
        // A record smaller than the header (27B) is discarded as out of scope — EOF once everything is read.
        let mut s = Pcap::new();
        s.0.extend([0u8; 16]); // incl_len = 0
        let samples = Mutex::new(Samples::default());
        let err = consume(&s.0[..], TARGET, &AtomicBool::new(false), &samples).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        // EOF midway -> UnexpectedEof.
        let mut t = Pcap::new();
        t.motion(10, 2, 1000);
        t.0.truncate(t.0.len() - 7);
        let err = consume(
            &t.0[..],
            TARGET,
            &AtomicBool::new(false),
            &Mutex::new(Samples::default()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        // A wrong global header is invalid as well.
        let err = consume(
            &[0u8; 24][..],
            TARGET,
            &AtomicBool::new(false),
            &Mutex::new(Samples::default()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn status_maps_os_errors_by_stage() {
        let e = |code: i32| io::Error::from_raw_os_error(code);
        assert_eq!(
            status_of(&e(2), Stage::Open),
            Status::DriverMissing { os: Some(2) }
        );
        assert_eq!(
            status_of(&e(5), Stage::Open),
            Status::AccessDenied { os: Some(5) }
        );
        assert_eq!(
            status_of(&e(32), Stage::Open),
            Status::HubBusy { os: Some(32) }
        );
        assert_eq!(
            status_of(&e(1167), Stage::Run),
            Status::Disconnected { os: Some(1167) }
        );
        assert_eq!(
            status_of(
                &io::Error::new(io::ErrorKind::UnexpectedEof, "x"),
                Stage::Run
            ),
            Status::Disconnected { os: None }
        );
        assert_eq!(
            status_of(&e(1167), Stage::Open),
            Status::Failed { os: Some(1167) }
        );
        assert_eq!(
            status_of(&e(87), Stage::Run),
            Status::Failed { os: Some(87) }
        );
    }

    #[test]
    fn waiting_until_first_record_then_zero_when_idle() {
        // Before the first receive: Waiting.
        let mut fresh = Samples::default();
        assert_eq!(fresh.snapshot(0).status, Status::Waiting);
        // If more than 1 second passes after receiving, it stays Live at 0 Hz.
        let mut s = Pcap::new();
        s.motion(10, 1, 1000);
        let mut samples = s.run(TARGET);
        let snap = samples.snapshot(12_500_000);
        assert_eq!(snap.status, Status::Live);
        assert_eq!(snap.hz, 0);
        // An error stays in status as a terminal state.
        samples.error = Some(Status::HubBusy { os: Some(32) });
        assert_eq!(samples.snapshot(0).status, Status::HubBusy { os: Some(32) });
    }
}
