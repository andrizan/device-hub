//! Measure the actual input report rate with Raw Input (Windows).
//!
//! Receives raw input from the mouse (usage 1/2) and keyboard (usage 1/6) in a message-only
//! window with `RIDEV_INPUTSINK`, and counts the arriving events per hDevice → VID/PID.
//! Unlike OS cursor/key events, this gives the arrival rate of the
//! input reports the device actually sent (for an 8KHz mouse, ~8000/s when moved).
//! A keyboard sends a report on every key state change, so it is only counted while typing.
//!
//! High-frequency handling: on `WM_INPUT`, read that event with `GetRawInputData`,
//! then drain the accumulated ones in bulk with `GetRawInputBuffer`
//! (the procedure Microsoft recommends for high-frequency devices. GetMessage removes the current event from
//! the queue, so nothing is double counted).
//!
//! Meaning of the "measured polling" values (docs/POLLING_RATE.md):
//! - A mouse sends reports "only when it moves", so arrivals ≤ counts/s.
//! - A rate is computed per segment (a stretch where gaps stay ≤ 2ms), excluding idle time.
//! - Sum |dx|+|dy| of RAWMOUSE to get "the theoretical ceiling at this speed and DPI ≒ counts/s".
//!
//! Note: Windows 11 (Moment 3 / KB5022497 and later) **restricts raw input delivery to background
//! processes**. In the background only values dropped to ~150Hz can be obtained, so
//! measurements are reliable only while this process's window is in front (same constraint as MouseTester).
//! The UI switches its display with `last_window_active`.

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::mem::size_of;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::{
        GetRawInputBuffer, GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices,
        HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RIDI_DEVICENAME,
        RIM_TYPEMOUSE, RID_INPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassExW,
        TranslateMessage, CW_USEDEFAULT, MSG, WM_INPUT, WNDCLASSEXW, WS_OVERLAPPED,
    };

    /// Hz measurement window.
    const WINDOW_SECS: f32 = 1.0;
    /// Bucket width of the graph.
    const BUCKET_MS: u64 = 250;
    /// Number of buckets kept for the graph (~15 s).
    const HIST_LEN: usize = 60;
    /// Number of recent gaps used for interval statistics (~64ms at 8KHz, ~0.5s at 1KHz).
    const GAP_LEN: usize = 512;
    /// A gap longer than this is treated as "stopped moving" and ends the segment (ms).
    const SEG_BREAK_MS: u64 = 2;
    /// Minimum duration to accept as a segment (ms).
    const SEG_MIN_SPAN_MS: u64 = 100;
    /// Minimum number of reports to accept as a segment.
    const SEG_MIN_N: u32 = 8;
    /// Number of finalized segments kept.
    const SEGS_LEN: usize = 128;
    /// Period covered by the "peak" (ms).
    const PEAK_WINDOW_MS: u64 = 10_000;
    /// Freshness of the end that counts as the "current segment" (ms). Finalized segments older than this are not used.
    const SEG_FRESH_MS: u64 = 1_500;
    /// Initial size of the raw input buffer (element count = u64 → 64KiB).
    const BUF_LEN: usize = 8192;

    /// A finalized "moving segment". start/last are ms since the epoch.
    #[derive(Clone, Copy, Default)]
    struct Seg {
        start_ms: u64,
        last_ms: u64,
        n: u32,
        /// Number of reports with |dx|+|dy| ≤ 1 (an indicator of insufficient DPI/speed).
        low: u32,
        /// Total movement (|dx|+|dy|).
        counts: u64,
    }

    impl Seg {
        fn span_ms(&self) -> u64 {
            self.last_ms.saturating_sub(self.start_ms)
        }
        fn rate_hz(&self) -> f32 {
            self.n as f32 / (self.span_ms() as f32 / 1000.).max(0.001)
        }
        /// Whether this is an acceptable segment (excludes short jitters and isolated events).
        fn usable(&self) -> bool {
            self.n >= SEG_MIN_N && self.span_ms() >= SEG_MIN_SPAN_MS
        }
    }

    struct Dev {
        /// VID/PID taken from the device name (None if not obtained).
        vidpid: Option<Option<(u16, u16)>>,
        /// Event times of the last ~1.2 s (ms since the epoch).
        events: VecDeque<u64>,
        /// Recent event intervals (ms).
        gaps: VecDeque<f32>,
        /// 250ms buckets (bucket_idx, count).
        buckets: VecDeque<(u64, u32)>,
        /// Segment being aggregated (mouse only - key events have counts=None and make no segment).
        seg: Seg,
        /// Finalized segments (newest at the tail).
        segs: VecDeque<Seg>,
    }

    impl Dev {
        fn new() -> Self {
            Self {
                vidpid: None,
                events: VecDeque::new(),
                gaps: VecDeque::new(),
                buckets: VecDeque::new(),
                seg: Seg::default(),
                segs: VecDeque::new(),
            }
        }

        fn bump(&mut self, ms: u64, counts: Option<u32>) {
            if let Some(&prev) = self.events.back() {
                let gap = ms.saturating_sub(prev) as f32;
                if gap > 0. {
                    self.gaps.push_back(gap);
                    while self.gaps.len() > GAP_LEN {
                        self.gaps.pop_front();
                    }
                }
            }
            self.events.push_back(ms);
            while self
                .events
                .front()
                .is_some_and(|t| ms.saturating_sub(*t) > 1200)
            {
                self.events.pop_front();
            }
            let idx = ms / BUCKET_MS;
            match self.buckets.back_mut() {
                Some(back) if back.0 == idx => back.1 += 1,
                _ => self.buckets.push_back((idx, 1)),
            }
            while self.buckets.len() > HIST_LEN {
                self.buckets.pop_front();
            }
            if let Some(c) = counts {
                self.seg_bump(ms, c);
            }
        }

        fn seg_bump(&mut self, ms: u64, c: u32) {
            if self.seg.n > 0 && ms.saturating_sub(self.seg.last_ms) > SEG_BREAK_MS {
                self.close_seg();
            }
            if self.seg.n == 0 {
                self.seg.start_ms = ms;
            }
            self.seg.last_ms = ms;
            self.seg.n += 1;
            self.seg.counts += c as u64;
            if c <= 1 {
                self.seg.low += 1;
            }
        }

        /// Finalize the segment being aggregated, and push it to the history if acceptable.
        fn close_seg(&mut self) {
            let s = self.seg;
            self.seg = Seg::default();
            if s.usable() {
                self.segs.push_back(s);
                while self.segs.len() > SEGS_LEN {
                    self.segs.pop_front();
                }
            }
        }
    }

    /// Public form of one segment in `stats`.
    #[derive(Clone, Copy, Debug)]
    pub struct SegInfo {
        /// Reports/s within the segment.
        pub rate_hz: f32,
        /// Duration of the segment (ms).
        pub span_ms: u64,
        /// Seconds elapsed since the segment ended (at the time of the stats call).
        pub age_s: f32,
    }

    /// Measurement snapshot - `stats` returns everything under a single lock.
    #[derive(Clone, Debug, Default)]
    pub struct PollStats {
        /// Events/s over the last 1 second (a lower bound, since idle time is also in the denominator).
        pub hz: f32,
        /// Rate history in 250ms buckets (oldest → newest, ~15 s).
        pub hist: Vec<f32>,
        /// Minimum interval of recent events (ms).
        pub min_gap_ms: Option<f32>,
        /// (median, p95) of recent event intervals [ms] - shows the true report period.
        pub gap_stats: Option<(f32, f32)>,
        /// Rate of the latest "moving segment" (in progress ≥60ms or finalized ≤1.5s ago).
        pub seg_hz: Option<f32>,
        /// Segment width of `seg_hz` (ms).
        pub seg_span_ms: Option<u64>,
        /// Maximum rate of segments that ended in the last 10 s (the "peak").
        pub peak_hz: Option<f32>,
        /// Total movement/s of the latest segment (|dx|+|dy|) - the theoretical ceiling at this speed and DPI ≒ this.
        pub counts_per_sec: Option<f32>,
        /// Share of reports with |d|≤1 in the latest segment - the higher it is, the more DPI/speed is lacking.
        pub low_frac: Option<f32>,
        /// Finalized segments (newest at the tail, up to 128) - used for windowing such as a 5-second measurement.
        pub segs: Vec<SegInfo>,
    }

    struct Meter {
        devs: HashMap<usize, Dev>,
        /// Time origin (time of the first event).
        epoch: Option<Instant>,
    }

    static METER: std::sync::LazyLock<Mutex<Meter>> = std::sync::LazyLock::new(|| {
        Mutex::new(Meter {
            devs: HashMap::new(),
            epoch: None,
        })
    });
    static STARTED: AtomicBool = AtomicBool::new(false);

    /// Call once at startup. Message-only window + raw input registration.
    pub fn start() {
        start_meter(true);
    }
    pub fn start_mouse() {
        start_meter(false);
    }
    fn start_meter(keyboard: bool) {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::Builder::new()
            .name("devhub-rawinput".into())
            .spawn(move || unsafe { run(keyboard) })
            .ok();
    }

    unsafe extern "system" fn wnd(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if msg == WM_INPUT {
            on_input(l);
        }
        DefWindowProcW(hwnd, msg, w, l)
    }

    unsafe fn run(keyboard: bool) {
        // Prioritize draining so that even an 8KHz mouse does not overflow the queue.
        // MMCSS "Pro Audio" gets scheduling favor beyond a mere priority boost
        // (the standard approach for a measurement-only thread). If unavailable, fall back to a normal priority boost.
        // (this helps accuracy while in front. The background delivery limit is on the OS side, so this cannot solve it)
        let mmcss_name: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
        let mut idx = 0u32;
        if windows_sys::Win32::System::Threading::AvSetMmThreadCharacteristicsW(
            mmcss_name.as_ptr(),
            &mut idx,
        )
        .is_null()
        {
            windows_sys::Win32::System::Threading::SetThreadPriority(
                windows_sys::Win32::System::Threading::GetCurrentThread(),
                windows_sys::Win32::System::Threading::THREAD_PRIORITY_ABOVE_NORMAL,
            );
        }
        let name: Vec<u16> = "DevHubRawInput\0".encode_utf16().collect();
        let hinst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd),
            hInstance: hinst,
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        // Message-only window (HWND_MESSAGE = -3).
        let hwnd = CreateWindowExW(
            0,
            name.as_ptr(),
            name.as_ptr(),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (-3isize) as HWND,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        let devs = [
            RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x02, // mouse
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: hwnd,
            },
            RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x06, // keyboard
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: hwnd,
            },
        ];
        RegisterRawInputDevices(
            devs.as_ptr(),
            if keyboard { devs.len() as u32 } else { 1 },
            size_of::<RAWINPUTDEVICE>() as u32,
        );
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    thread_local! {
        /// Read buffer for raw input (8B aligned, reused).
        static RAW_BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    }

    /// From one record: (hDevice, mouse movement |dx|+|dy|).
    /// Non-mouse devices such as keyboards get counts=None (excluded from segments and movement).
    fn read_raw(raw: &RAWINPUT) -> (usize, Option<u32>) {
        let hdev = raw.header.hDevice as usize;
        let counts = if raw.header.dwType == RIM_TYPEMOUSE {
            let m = unsafe { raw.data.mouse };
            Some(m.lLastX.unsigned_abs().saturating_add(m.lLastY.unsigned_abs()))
        } else {
            None
        };
        (hdev, counts)
    }

    fn push(hdev: usize, counts: Option<u32>, now: Instant) {
        let mut m = METER.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = *m.epoch.get_or_insert(now);
        let ms = now.duration_since(epoch).as_millis() as u64;
        m.devs
            .entry(hdev)
            .or_insert_with(Dev::new)
            .bump(ms, counts);
    }

    fn on_input(lp: LPARAM) {
        unsafe {
            let hdr_size = size_of::<RAWINPUTHEADER>() as u32;
            let now = Instant::now();
            RAW_BUF.with(|cell| {
                let mut guard = cell.borrow_mut();
                if guard.is_empty() {
                    guard.resize(BUF_LEN, 0);
                }
                let buf = &mut *guard;
                // 1) The event carried by this WM_INPUT (it leaves the queue via GetMessage, so
                //    it is not included in the buffer read - no double counting).
                let mut size = (buf.len() * 8) as u32;
                let got = GetRawInputData(
                    lp as HRAWINPUT,
                    RID_INPUT,
                    buf.as_mut_ptr().cast(),
                    &mut size,
                    hdr_size,
                );
                if got != u32::MAX && got as usize >= size_of::<RAWINPUTHEADER>() {
                    let (hdev, counts) = read_raw(&*(buf.as_ptr() as *const RAWINPUT));
                    push(hdev, counts, now);
                }
                // 2) Drain the rest accumulated in the queue in bulk (the official procedure for high frequencies).
                loop {
                    let mut sz = (buf.len() * 8) as u32;
                    let n = GetRawInputBuffer(buf.as_mut_ptr().cast(), &mut sz, hdr_size);
                    if n == 0 {
                        break;
                    }
                    if n == u32::MAX {
                        // Buffer too small → grow to the required size or 2x and retry.
                        let need = (sz as usize).div_ceil(8);
                        buf.resize(need.max(buf.len() * 2), 0);
                        continue;
                    }
                    let mut p = buf.as_ptr() as *const RAWINPUT;
                    for _ in 0..n {
                        let raw = &*p;
                        let (hdev, counts) = read_raw(raw);
                        push(hdev, counts, now);
                        // Equivalent of NEXTRAWINPUTBLOCK: advance by header.dwSize aligned to the pointer boundary.
                        let adv =
                            (raw.header.dwSize as usize + size_of::<usize>() - 1)
                                & !(size_of::<usize>() - 1);
                        p = (p as *const u8).add(adv) as *const RAWINPUT;
                    }
                }
            });
        }
    }

    /// hDevice → (vid, pid). The device name is `…VID_xxxx&PID_xxxx…`.
    fn vidpid_of(hdev: usize) -> Option<(u16, u16)> {
        unsafe {
            let mut size = 0u32;
            GetRawInputDeviceInfoW(hdev as _, RIDI_DEVICENAME, std::ptr::null_mut(), &mut size);
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u16; size as usize];
            if GetRawInputDeviceInfoW(
                hdev as _,
                RIDI_DEVICENAME,
                buf.as_mut_ptr().cast(),
                &mut size,
            ) == u32::MAX
            {
                return None;
            }
            let name = String::from_utf16_lossy(&buf);
            let hex = |tag: &str| {
                name.find(tag)
                    .and_then(|i| u16::from_str_radix(&name[i + 4..i + 8], 16).ok())
            };
            Some((hex("VID_")?, hex("PID_")?))
        }
    }

    /// Measurement snapshot of the device with the given vid/pid. None if absent.
    pub fn stats(vid: u16, pid: u16) -> Option<PollStats> {
        let mut m = METER.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let now_ms = m
            .epoch
            .map(|e| now.duration_since(e).as_millis() as u64)
            .unwrap_or(0);

        let mut st = PollStats::default();
        let mut merged_buckets: HashMap<u64, u32> = HashMap::new();
        let mut all_gaps: Vec<f32> = Vec::new();
        let mut found = false;

        for (&hdev, d) in m.devs.iter_mut() {
            let vp = match d.vidpid {
                Some(v) => v,
                None => {
                    let v = vidpid_of(hdev);
                    d.vidpid = Some(v);
                    v
                }
            };
            if vp != Some((vid, pid)) {
                continue;
            }
            found = true;

            // Number of events in the last 1 second.
            st.hz += d
                .events
                .iter()
                .filter(|t| now_ms.saturating_sub(**t) <= (WINDOW_SECS * 1000.) as u64)
                .count() as f32;
            for &(i, c) in &d.buckets {
                *merged_buckets.entry(i).or_default() += c;
            }
            all_gaps.extend(d.gaps.iter().copied());

            // Segments: finalized ones are published and used for the peak calculation; the in-progress one is used only for the peak and "current".
            let mut newest: Option<(u64, Seg, bool)> = None; // (last_ms, seg, is_running)
            for &s in &d.segs {
                let age_ms = now_ms.saturating_sub(s.last_ms);
                if age_ms <= PEAK_WINDOW_MS {
                    st.peak_hz = Some(st.peak_hz.map_or(s.rate_hz(), |p: f32| p.max(s.rate_hz())));
                }
                st.segs.push(SegInfo {
                    rate_hz: s.rate_hz(),
                    span_ms: s.span_ms(),
                    age_s: age_ms as f32 / 1000.,
                });
                if newest.is_none_or(|(t, ..)| s.last_ms > t) {
                    newest = Some((s.last_ms, s, false));
                }
            }
            if d.seg.n > 0 {
                let s = d.seg;
                if now_ms.saturating_sub(s.last_ms) <= PEAK_WINDOW_MS {
                    st.peak_hz = Some(st.peak_hz.map_or(s.rate_hz(), |p: f32| p.max(s.rate_hz())));
                }
                if newest.is_none_or(|(t, ..)| s.last_ms > t) {
                    newest = Some((s.last_ms, s, true));
                }
            }
            // "Current segment" = in progress ≥60ms and fresh, or finalized ≤1.5s ago.
            if let Some((last, s, live)) = newest {
                let ok = if live {
                    s.span_ms() >= 60 && now_ms.saturating_sub(last) <= 300
                } else {
                    s.usable() && now_ms.saturating_sub(last) <= SEG_FRESH_MS
                };
                if ok {
                    st.seg_hz = Some(st.seg_hz.map_or(s.rate_hz(), |p: f32| p.max(s.rate_hz())));
                    st.seg_span_ms = Some(s.span_ms());
                    let secs = (s.span_ms() as f32 / 1000.).max(0.001);
                    st.counts_per_sec = Some(s.counts as f32 / secs);
                    st.low_frac = Some(s.low as f32 / s.n.max(1) as f32);
                }
            }
        }
        if !found {
            return None;
        }
        st.hz /= WINDOW_SECS;

        // History graph (buckets expanded into sequential numbers).
        let (mut lo, mut hi) = (u64::MAX, 0u64);
        for &i in merged_buckets.keys() {
            lo = lo.min(i);
            hi = hi.max(i);
        }
        if lo <= hi {
            st.hist = (lo..=hi)
                .map(|i| *merged_buckets.get(&i).unwrap_or(&0) as f32 / (BUCKET_MS as f32 / 1000.))
                .collect();
        }

        // Interval statistics (anything under 0.05ms is excluded as dispatch coalescing noise - the true interval at 8K is 0.125ms).
        const FLOOR_MS: f32 = 0.05;
        all_gaps.retain(|&g| g >= FLOOR_MS);
        if !all_gaps.is_empty() {
            st.min_gap_ms = all_gaps.iter().copied().reduce(f32::min);
        }
        if all_gaps.len() >= 8 {
            all_gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let pct = |q: f32| all_gaps[((all_gaps.len() - 1) as f32 * q) as usize];
            st.gap_stats = Some((pct(0.5), pct(0.95)));
        }
        st.segs.sort_by(|a, b| a.age_s.partial_cmp(&b.age_s).unwrap_or(std::cmp::Ordering::Equal));
        Some(st)
    }
}

#[cfg(windows)]
pub use imp::{start, start_mouse, stats, PollStats, SegInfo};

#[cfg(not(windows))]
mod stub {
    #[derive(Clone, Copy, Debug)]
    pub struct SegInfo {
        pub rate_hz: f32,
        pub span_ms: u64,
        pub age_s: f32,
    }
    #[derive(Clone, Debug, Default)]
    pub struct PollStats {
        pub hz: f32,
        pub hist: Vec<f32>,
        pub min_gap_ms: Option<f32>,
        pub gap_stats: Option<(f32, f32)>,
        pub seg_hz: Option<f32>,
        pub seg_span_ms: Option<u64>,
        pub peak_hz: Option<f32>,
        pub counts_per_sec: Option<f32>,
        pub low_frac: Option<f32>,
        pub segs: Vec<SegInfo>,
    }
    pub fn start() {}
    pub fn start_mouse() {}
    pub fn stats(_vid: u16, _pid: u16) -> Option<PollStats> {
        None
    }
}

#[cfg(not(windows))]
pub use stub::{start, start_mouse, stats, PollStats, SegInfo};
