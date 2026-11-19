//! Audio sync — captures the playback sound via WASAPI loopback and
//! splits it into band levels (0..1) with Goertzel. Input for `livefx::FxMode::Audio`.
//!
//! Capture runs on a dedicated thread and puts the latest band list into a shared slot.
//! Read it with `bands()` (a copy of the Mutex contents). Samples are mixed down to mono and
//! every 1024 samples 15 bands (60Hz..16kHz, log-spaced) are computed,
//! then smoothed with a fast attack and a slow release.

use std::sync::{Mutex, OnceLock};

/// Number of bands (matches the keyboard's 15 columns).
pub const NBANDS: usize = 15;
/// Goertzel window width (about 21ms at 48kHz).
const N: usize = 1024;

static BANDS: OnceLock<Mutex<Vec<f32>>> = OnceLock::new();
static STARTED: OnceLock<()> = OnceLock::new();

/// Current band level list (an empty list when silent or not started).
pub fn bands() -> Vec<f32> {
    BANDS
        .get()
        .map(|m| m.lock().map(|b| b.clone()).unwrap_or_default())
        .unwrap_or_default()
}

/// Starts the capture thread exactly once (safely stays silent on failure).
pub fn start() {
    STARTED.get_or_init(|| {
        BANDS.set(Mutex::new(vec![0.; NBANDS])).ok();
        std::thread::Builder::new()
            .name("devhub-audio".into())
            .spawn(|| unsafe { run() })
            .ok();
    });
}

fn put(v: &[f32]) {
    if let Some(m) = BANDS.get() {
        if let Ok(mut b) = m.lock() {
            *b = v.to_vec();
        }
    }
}

/// Band center frequencies (60..16000Hz, log-spaced).
fn band_freqs() -> [f32; NBANDS] {
    let mut f = [0f32; NBANDS];
    let lo = 60f32.ln();
    let hi = 16000f32.ln();
    for (i, f) in f.iter_mut().enumerate() {
        *f = (lo + (hi - lo) * i as f32 / (NBANDS - 1) as f32).exp();
    }
    f
}

/// Goertzel — normalized strength (around 0..1) of the freq component in buf.
fn goertzel(buf: &[f32], sr: f32, freq: f32) -> f32 {
    let w = 2. * std::f32::consts::PI * freq / sr;
    let cw = 2. * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for &x in buf {
        let s0 = x + cw * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let p = (s1 * s1 + s2 * s2 - cw * s1 * s2).sqrt() / (N as f32 * 0.5);
    // Log-like lift + clip.
    (p * 2.4).powf(0.6).clamp(0., 1.)
}

#[cfg(windows)]
unsafe fn run() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
        return;
    }
    // Acquire and capture again after a pause, so we recover even if it dies once on an
    // output device (un)plug or a default-device switch. Silence while failing.
    loop {
        capture_once();
        put(&[0.; NBANDS]);
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Open the default output and run the loopback. Returns on an error (e.g. device lost).
#[cfg(windows)]
unsafe fn capture_once() {
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEXTENSIBLE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

    let en: IMMDeviceEnumerator = match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
        Ok(e) => e,
        Err(_) => return,
    };
    let dev = match en.GetDefaultAudioEndpoint(eRender, eConsole) {
        Ok(d) => d,
        Err(_) => return,
    };
    let cli: IAudioClient = match dev.Activate(CLSCTX_ALL, None) {
        Ok(c) => c,
        Err(_) => return,
    };
    let fmt = match cli.GetMixFormat() {
        Ok(f) => f,
        Err(_) => return,
    };
    let wfx = &*fmt;
    let sr = wfx.nSamplesPerSec as f32;
    // Whether it is float32 (PCM16 is 0x1, float is 0x3 / WAVE_FORMAT_EXTENSIBLE 0xfffe).
    let tag = wfx.wFormatTag;
    let is_float = if tag == 0x3 {
        true
    } else if tag == 0xfffe {
        let ext = &*(fmt as *const WAVEFORMATEXTENSIBLE);
        ext.SubFormat.data1 == 3
    } else {
        false
    };
    let ch = wfx.nChannels.max(1) as usize;
    let bpframe = wfx.nBlockAlign as usize;
    if !is_float || wfx.wBitsPerSample != 32 || bpframe < ch * 4 {
        CoTaskMemFree(Some(fmt as *const _));
        return;
    }
    if cli
        .Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            0,
            0,
            fmt,
            None,
        )
        .is_err()
    {
        CoTaskMemFree(Some(fmt as *const _));
        return;
    }
    CoTaskMemFree(Some(fmt as *const _));
    let cap: IAudioCaptureClient = match cli.GetService() {
        Ok(c) => c,
        Err(_) => return,
    };
    if cli.Start().is_err() {
        return;
    }
    let freqs = band_freqs();
    let mut acc: Vec<f32> = Vec::with_capacity(N * 2);
    let mut smooth = [0f32; NBANDS];
    // Recent peak per band — auto-normalization so even quiet sounds reach full scale.
    let mut peaks = [0.05f32; NBANDS];
    loop {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let mut pkt = match cap.GetNextPacketSize() {
            Ok(p) => p,
            Err(_) => break,
        };
        while pkt > 0 {
            let mut data = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            if cap
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .is_err()
            {
                return;
            }
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                acc.resize(acc.len() + frames as usize, 0.);
            } else if !data.is_null() && frames > 0 {
                let n = frames as usize * bpframe;
                let raw = std::slice::from_raw_parts(data as *const u8, n);
                for fr in raw.chunks_exact(bpframe) {
                    // Mix float32 × ch down to mono.
                    let mut s = 0f32;
                    for c in 0..ch {
                        s += f32::from_le_bytes([
                            fr[c * 4],
                            fr[c * 4 + 1],
                            fr[c * 4 + 2],
                            fr[c * 4 + 3],
                        ]);
                    }
                    acc.push(s / ch as f32);
                }
            }
            let _ = cap.ReleaseBuffer(frames);
            pkt = cap.GetNextPacketSize().unwrap_or(0);
        }
        while acc.len() >= N {
            let buf: Vec<f32> = acc.drain(..N).collect();
            for (i, &f) in freqs.iter().enumerate() {
                let v = goertzel(&buf, sr, f);
                // Peak tracking: instant rise, decay halves in about 7 s, floor 0.03.
                peaks[i] = (peaks[i] * 0.995).max(v).max(0.03);
                let v = (v / peaks[i]).min(1.);
                // Smoothing with attack 0.55 / release 0.88.
                smooth[i] = if v > smooth[i] {
                    smooth[i] * 0.45 + v * 0.55
                } else {
                    smooth[i] * 0.88 + v * 0.12
                };
            }
            put(&smooth);
        }
    }
}

#[cfg(not(windows))]
unsafe fn run() {}
