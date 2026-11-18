# legacy/ — drivers from the contract v1 era (not compiled)

On 2026-11-14 the devhub-core contract was rebuilt as v2 (Capabilities + DeviceState + Change),
so the code that depended on v1 types was moved here. **Don't throw it away; use it as source material for porting.**

| File | Contents | Value |
|---|---|---|
| `mousefit.rs` | mouse.fit family (HTS Plus Pro 33e4:5219) report 8 / 16B commands / EEPROM read and write | Read and write verified on a real device. Framing, checksum (`0x55` sum), complement pairs and the DPI 4B record can be reused as is |
| `xsyd.rs` | XSYD (SU66 Max 1ca6:3009) 64B frames / group-sub commands / key code table | Read verified on a real device (device info, performance, lighting). 2 layers |
| `probe.rs`, `wtest.rs` | Real-device tests that use the v1 API | Rewrite for the new API, then move back to `examples/` |

Porting targets: `crates/devhub-hid/src/mousefit/` and `crates/devhub-hid/src/xsyd/` (see docs/BACKEND_BRIEF.md).
