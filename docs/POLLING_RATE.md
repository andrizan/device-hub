# Verification of the "measured polling rate" and improvement proposals (2026-11-16)

Symptom: the HTS Plus Pro is set to 8K, but the "measured polling rate" shows ~2700Hz.

## Measuring with the game in the foreground on Windows (2026-11-18)

The Device HUB Test screen counts the events that Windows delivered to the app. Mouse input to a window in the background is batched by the OS, so the original transfer count cannot be recovered from the number of events received. For a wired USB mouse and a USB dongle, counting the completions of the mouse-movement interrupt IN endpoint transfers with USBPcap allows a comparison that does not depend on window focus. What can be measured with a dongle is the transfers from the dongle to the PC. Bluetooth is out of scope for this procedure. [Microsoft background article](https://blogs.windows.com/windowsdeveloper/2023/05/26/delivering-delightful-performance-for-more-than-one-billion-users-worldwide/) / [USBPcap tour](https://desowin.org/usbpcap/tour.html)

### Measuring from the app (only USBPcap is required — no Python/TShark)

The measurement code is `crates/devhub-polling` (a Rust implementation that opens the USBPcap driver directly). It gives the same value as OBS's "Device HUB USB Polling Rate" source. Verified on a real device (2026-11-18, G-Wolves Receiver RS 8K @ USBPcap2/addr2/EP0x81 — ~7,800Hz with continuous movement, peak 7,923Hz). Administrator rights are required.

1. Install [USBPcap](https://desowin.org/usbpcap/) and restart (first time only). In some environments, opening the hub for read/write requires administrator rights — if "insufficient privileges" is shown, restart Device HUB as administrator.
2. On the device's "Test" tab → "Measured polling rate" → set the measurement method to **USB transfer (USBPcap)**.
3. Select the product by name from the "Target device" list — the hub number and USB address are filled in automatically (enumeration needs no administrator rights; it is matched with `IOCTL_USBPCAP_GET_HUB_SYMLINK` and the Configuration Manager. After unplugging/replugging or changing ports, rescan with "Refresh"). For the IN EP, "Auto-detect EP" counts for 3 seconds while you move the target and adopts the one with the most transfers. If you do not know it or want to specify it manually, you can enter it directly in the detail fields below (hub number / USB address / IN EP number 0x81→1 / Report ID -1=all). For diagnostics, the probes `cargo run --release -p devhub-polling --example usbenum` (enumeration, no administrator rights needed) / `usbscan <hub> [seconds]` (scan the whole hub, requires administrator) / `usbfind <hub> <addr> [seconds]` (EP detection, requires administrator) are also kept. The address changes whenever you unplug and replug.
4. Move the mouse for a few seconds with another app in the foreground. "Peak" updates the count of (t−1 s, t] on every received report, so it does not miss peaks between display updates. When you end with "Stop", the target, the measurement source, and the peak value remain. Only 1 process can hold the same hub at a time (measuring simultaneously with OBS's USB source or TShark is not possible — "Another process is measuring the same hub" is shown).

### Measuring with the TShark script (a separate path for comparison)

1. Prepare Wireshark's TShark and USBPcap on Windows. In Wireshark's capture settings, find the USBPcap interface and the USB device address that the target mouse is connected to. Take a short trial capture and choose the HID mouse interrupt IN that follows movement. Read the bus number and endpoint address from the packet details. Re-check the device address after reconnecting.
2. Replace the values below with those of your real device and run it, then go back to the game and move the mouse for a few seconds. The interface name is the USBPcap name shown by `tshark -D`. In environments where capture requires privileges, use an administrator PowerShell.

   ```powershell
   .\scripts\measure-usb-rate.ps1 -Interface '\\.\USBPcap1' -Bus 1 -Device 5 -Endpoint 0x81 -Seconds 10
   ```

3. In the table at the end, read the **Frames of a full 1-second row** in which you kept moving. For example, if it is 7,950, the measured value for that second is 7,950 transfers/s. Exclude the partial intervals at the start and end. If zeros continue, check whether the trial capture contains packets for the chosen address and endpoint. The filter requires a successful, data-bearing, completed receive. For models where several Report IDs share the same endpoint, the value is the total. USB polling attempts while idle and the cycle of the wireless segment cannot be observed.

A USBPcap capture saved from Wireshark with only the target device can also be analyzed by passing `-CaptureFile .\mouse.pcapng` instead of `-Interface`. When comparing the foreground and the background, keep the DPI, the configured rate, and the way of moving the same. [Wireshark USB fields](https://www.wireshark.org/docs/dfref/u/usb.html)

The in-app delivery interval is recorded in µs, and the history display is limited to the most recent 60 buckets. Batched-read input is timestamped per batch, so the median is approximate. The continuous-span check tolerates up to 12 ms so that 125 Hz (8 ms interval) is included. What follows is the investigation record as of November 16.

## 1. How the current measurement works
- `devhub-obs/src/rawinput.rs`: receives Raw Input (mouse/keyboard) with `RIDEV_INPUTSINK` in a message-only window,
  and for each `WM_INPUT` message calls `GetRawInputData` → records the time per hDevice.
- Displayed value = `rawinput::hz()` = **the number of WM_INPUT messages that arrived in the last 1.0 seconds**. `Job::PollRate` in `store.rs` fetches it about every ~400ms.
- Note: `rawinput::gap_stats()` (median/p95 of the intervals) and `min_gap_ms()` are also already computed, but they are not used for the big number.

## 2. Conclusion: correct as "input reports that arrived per second", but it reads low as a "polling rate"

### Cause A (the biggest factor): a mouse sends reports "only when there is motion"
Even at 8K polling, no report is sent unless the sensor has accumulated at least 1 count (movement) within the 125µs.
So **reports that arrive ≤ counts/second = movement speed × DPI**.

| DPI | Speed needed for 8000 reports/second |
| --- | --- |
| 400 | 50.8 cm/s |
| 800 | 25.4 cm/s |
| 1600 | 12.7 cm/s |
| 3200 | 6.4 cm/s |

Example: moving at 10 cm/s at 800 DPI gives about 3150 counts/second → an upper limit of about 3150Hz. **2700Hz is a typical value for "capped by DPI and speed"**,
and does not necessarily mean 8K is not working. MouseTester and similar tools have the same limitation (the standard approach is to draw fast circles at high DPI).

### Cause B: a fixed 1-second window includes time when the mouse is stopped or slowing down
Slowing down and turning around in the middle of a circle also enter the denominator, so the value always reads lower than the peak.

### Cause C: WM_INPUT is processed one message at a time
8000 dispatches/second + a Vec allocation each time + a Mutex leave room for missed events and merged dispatches under heavy load.
Reading in bulk with `GetRawInputBuffer` is the procedure Microsoft recommends for high-frequency input.
Also, the timestamp is the "processing time", not the "arrival time", so the interval statistics include jitter.

### Cause D (foreground/background): Windows 11 restricts raw input to background processes
→ The UI already distinguishes this with "the OS is restricting input because the window is in the background". Measure in the foreground.

### Separate issue: the "8K Hz" display in the OBS overlay is also something else
`mousefit8::live_report_hz()` counts the **number of op10 / report4 notifications on the vendor IF** (`rate_tick`).
These are not mouse-movement reports, so displaying it as a "polling rate" is wrong. On the OBS side, use rawinput or change the label.

## 3. Improvement proposals (in priority order)

### P1 Make the display a meaningful value as a "polling rate" (frontend + rawinput.rs)
1. **Measure only over spans of continuous movement**: when an interval exceeds 2ms, treat it as "stopped" and cut the span,
   and output (number of reports / span length) within the span. The maximum over spans that lasted 100ms or more is the "peak", and the latest span is the "current".
2. **Estimate from the median interval**: show `1000 / gap_stats().median` (intervals under 0.05ms are already excluded) alongside as the "estimated polling period".
   If 8K is working, the median should land near ~0.125ms (= 8000Hz) — even with insufficient speed/DPI, the intervals of the reports that do arrive are multiples of 0.125ms.
3. **Cap diagnosis**: aggregate the "counts per report" from RAWMOUSE `lLastX/lLastY`.
   If most reports are ±1 count, show "Reports are being thinned out because DPI or speed is too low — set DPI to 3200 or higher and draw fast, large circles".
   Also showing `counts/second` can indicate "theoretical limit at this speed and DPI ≈ X Hz".
4. The big number is the "peak", with a small "average (1 s)" and "estimated period 0.125ms". The graph can stay with 250ms buckets.
5. A "Measure for 5 seconds" button: after pressing it, show a fixed result of the maximum span rate and the median interval over 5 seconds (used like MouseTester).

### P2 Countermeasures for missed measurements (rawinput.rs)
- On `WM_INPUT`, read everything that has accumulated in one go with `GetRawInputBuffer` (multiple RAWINPUT per syscall).
  Reuse the buffer and take the lock once per batch (currently a Vec allocation + Mutex per event).
- Make the event times a ring buffer (fixed 16K entries) of u64 (QPC ticks) instead of Vec<Instant>, and have the UI-side fetch return only
  aggregated numbers (currently `devs_for` copies nearly 10,000 Instant/gap values while holding the lock).
- If possible, take QPC immediately together with `RAWINPUTHEADER` instead of `GetMessageTime` (same as now) — keep the note that it is not the arrival time.

### P3 The Hz display in OBS
- Either state clearly that `live_report_hz` is a "notification rate", or use rawinput in the OBS plugin too (with the background restriction — inaccurate unless OBS is in the foreground).
  If accuracy cannot be guaranteed, it is safer to remove Hz from the OBS display.

## 4. Measurement steps for users (draft wording for the UI help)
1. Bring Device HUB to the foreground (in the background the OS thins out the input)
2. Temporarily set DPI to 3200 or higher (restore it after measuring)
3. Draw fast, large circles on the mouse pad for 3 to 5 seconds
4. Look at the "peak" and the "estimated period" (for 8K, ~8000Hz / ~0.125ms)

## 5. Implementation status (2026-11-16)

- **P1 (display)** — implemented. "Measured polling rate" card on the Test tab: the big number is the peak
  (maximum rate of a moving span), with a sub-row showing the average (1 s) and the current span, an estimated period (median interval) row,
  a "movement X/s → limit ≈ X Hz" row, and a DPI/speed-shortage hint when it is almost 1 count.
  "Measure for 5 seconds" button: while measuring, the cursor is pinned inside the window with ClipCursor, and at the end the
  peak, median interval, and movement are shown as a fixed result.
- **P2 (countermeasures for missed events)** — implemented. After reading with `GetRawInputData` on `WM_INPUT`,
  the queue is drained with `GetRawInputBuffer` (Microsoft's official procedure for high frequencies,
  the buffer is reused and the lock is taken once per batch). Events are a ring of u64 ms timestamps,
  and `stats()` returns an aggregated snapshot under one lock.
- **P3 (OBS)** — done (the Hz display has been removed from the overlay).
- rawinput.rs API change: `hz/hist/min_gap_ms/gap_stats` → `stats() -> PollStats`
  (hz, hist, min_gap_ms, gap_stats, seg_hz, peak_hz, counts_per_sec, low_frac, segs).
  `Session.poll: Option<PollStats>` is the entry point for the UI. Verify with `cargo run -p devhub-obs --example rate`.
