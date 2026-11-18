# SU66 Max fixtures

`probe1.txt` — hex dump of every response to the commands sent to the real device
(VID:PID `1ca6:3009`, usagePage `FFB0`) on 2026-11-15. `.req.bin` / `.resp.bin` are some of
them saved as 64B binaries.

| name | request | response highlights |
|---|---|---|
| `perf` | `[4,1,1,0]` performance read (row1,col0) | mode=0, nP=nR=2.0, rtF=0.5, rtP=rtR=0.3, dZ=0.2/0.2, axis=0, calib=1, axisV2=0x3561, coef=0x0c80, rangeMax=0.1 |
| `higher_none` | `[6,1,1,0,0]` advanced key read | mode=0 (NONE, not set) |
| `macro_space` | `[2,14,0]` macro area | r[3]=16 slots, r[4,5]=960 max events |
| `poll_read` | `[2,5,1]` current polling rate | r[3]=0 (8KHz) |
| `route_r1` | `[4,3,1,1]` route row 1 | all 0 (nothing pressed). `live_travel` collects this over rows 1..5 |

Note: at probe time every key was mode=0 (normal), no advanced keys were set and the macros were empty.
`axisRangeMax` (perf r[25,26]) returned 0.1mm — that is not the full stroke, so
`XsydInfo::travel` uses the default 4.0mm (values under 0.5mm are not adopted).
