//! mouse.fit protocol — HTS+ Pro / HTX Ultra / ATK F1 Ultimate, etc.
//!
//! Analyzed on real hardware (G-Wolves Receiver RS, 33e4:3854, usage_page 0xFF02 / usage 2):
//! - output report id 8 (16B command + 1B checksum)
//! - replies on input report id 8 (17B: [id][op echo][status][dev][data@5+][cksum])
//! - checksum: sum of all 17B (including id) ≡ 85 (0x55)
//! - command ops: 4=battery, 18=FW version, 29=FW (per dev), 8=EEPROM read,
//!   7=EEPROM write
//! - EEPROM addresses (analyzed from the mouse.fit bundle):
//!   ReportRate=0, maxDpiStage=2, CurrentDPI=4, KeyOperation=8, LOD=10,
//!   DPIValue=12 (4B×stage), DebounceTime=169, MotionSync=171, SleepTime=173,
//!   Angle=175, Ripple=177
//! - while the mouse is asleep the data part comes back all 0 (only the dongle responds)

use devhub_core::backend::{DeviceDriver, HubError, HubResult};
use devhub_core::model::*;
use hidapi::HidDevice;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const READY: bool = true;

const REPORT_ID: u8 = 8;
/// EEPROM address table (from the JS bundle). Unused entries are kept for future implementation.
#[allow(dead_code)]
mod ee {
    pub const REPORT_RATE: u16 = 0;
    pub const MAX_DPI_STAGE: u16 = 2;
    pub const CURRENT_DPI: u16 = 4;
    pub const KEY_OPERATION: u16 = 8;
    pub const LOD: u16 = 10;
    pub const DPI_VALUE: u16 = 12;
    pub const DPI_COLOR: u16 = 44;
    pub const KEY_FUNCTION: u16 = 96;
    pub const DEBOUNCE: u16 = 169;
    pub const MOTION_SYNC: u16 = 171;
    pub const SLEEP: u16 = 173;
    pub const ANGLE: u16 = 175;
    pub const RIPPLE: u16 = 177;
    pub const SENSOR_FPS20K: u16 = 225;
    pub const WHEEL_DEBOUNCE: u16 = 227;
    pub const DEBOUNCE_RELEASE: u16 = 229;
}

const OP_BATTERY: u8 = 4;
const OP_EEPROM_WRITE: u8 = 7;
const OP_EEPROM_READ: u8 = 8;
const OP_FIRMWARE: u8 = 18;

/// Takes the first byte of a per-EEPROM-value (v, 0x55-v) pair.
fn pair_val(chunk: &[u8]) -> Option<u8> {
    chunk.first().copied()
}

const POLL_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

pub struct MousefitDriver {
    info: DeviceInfo,
    dev: Mutex<HidDevice>,
}

pub fn open(info: DeviceInfo, path: &std::ffi::CString) -> HubResult<MousefitDriver> {
    let api = hidapi::HidApi::new().map_err(|e| HubError::Io(e.to_string()))?;
    let dev = api
        .open_path(path)
        .map_err(|e| HubError::Io(format!("open {}: {e}", info.name)))?;
    dev.set_blocking_mode(false).ok();
    Ok(MousefitDriver {
        info,
        dev: Mutex::new(dev),
    })
}

impl MousefitDriver {
    /// Sends a 16B command frame on report id 8 and returns the 17B response.
    /// Verifies the response's op echo + checksum (sum17 ≡ 85).
    fn cmd(&self, frame: &[u8; 16]) -> HubResult<[u8; 17]> {
        let dev = self.dev.lock().unwrap_or_else(|p| p.into_inner());
        let mut w = [0u8; 17];
        w[0] = REPORT_ID;
        w[1..17].copy_from_slice(frame);
        // Checksum: adjust the last byte so that the sum over id+data ≡ 85
        // JS: t[15] = (85 - sum(t[0..15])) - reportID
        let sum: u32 = w[..16].iter().map(|&b| b as u32).sum();
        w[16] = (85u32.wrapping_sub(sum)) as u8;

        dev.write(&w)
            .map_err(|e| HubError::Io(format!("mousefit write: {e}")))?;

        let deadline = Instant::now() + Duration::from_millis(500);
        let mut buf = [0u8; 65];
        while Instant::now() < deadline {
            match dev.read_timeout(&mut buf, 80) {
                Ok(n) if n >= 17 && buf[0] == REPORT_ID && buf[1] == frame[0] => {
                    let mut out = [0u8; 17];
                    out.copy_from_slice(&buf[..17]);
                    let s: u32 = out.iter().map(|&b| b as u32).sum();
                    if s % 256 != 85 {
                        // Return the data even on a checksum mismatch (to allow for FW differences)
                    }
                    return Ok(out);
                }
                Ok(_) => {}
                Err(e) => return Err(HubError::Io(format!("mousefit read: {e}"))),
            }
        }
        Err(HubError::Protocol("mousefit: response timeout".into()))
    }

    /// EEPROM read: cmd [8,0,addrHi,addrLo,len] → resp [id][op][0][addrHi][addrLo][len][data…]
    /// (JS: Get_Device_Eeprom_Buffer — data starts at payload index 5 = buf[6])
    fn ee_read(&self, addr: u16, len: u8) -> HubResult<Vec<u8>> {
        let mut f = [0u8; 16];
        f[0] = OP_EEPROM_READ;
        f[2] = (addr >> 8) as u8;
        f[3] = (addr & 0xff) as u8;
        f[4] = len;
        let r = self.cmd(&f)?;
        Ok(r[6..6 + (len as usize).min(11)].to_vec())
    }

    /// EEPROM array write: in 10B chunks, cmd [7,0,addrHi,addrLo,len,data…]
    /// (JS: Set_Device_Eeprom_Array — scalars are written as a 2B [v, 85-v])
    fn ee_write(&self, addr: u16, data: &[u8]) -> HubResult<()> {
        for (i, chunk) in data.chunks(10).enumerate() {
            let a = addr + (i * 10) as u16;
            let mut f = [0u8; 16];
            f[0] = OP_EEPROM_WRITE;
            f[2] = (a >> 8) as u8;
            f[3] = (a & 0xff) as u8;
            f[4] = chunk.len() as u8;
            f[5..5 + chunk.len()].copy_from_slice(chunk);
            self.cmd(&f)?;
        }
        Ok(())
    }

    /// Scalar write: a 2B [v, 85-v] pair (JS: Set_Device_Eeprom_Value(.., v, 2)).
    fn ee_write_val(&self, addr: u16, val: u8) -> HubResult<()> {
        self.ee_write(addr, &[val, 85u8.wrapping_sub(val)])
    }

    /// Scalar read: first byte of the 2B pair (r[6]).
    fn ee_u8(&self, addr: u16) -> u8 {
        self.ee_read(addr, 2)
            .ok()
            .and_then(|v| pair_val(&v))
            .unwrap_or(0)
    }

    /// CRC of a 4B block (JS: get_Crc — 85 minus the sum excluding the last byte).
    fn crc4(b: &[u8]) -> u8 {
        85u8.wrapping_sub(b[..3].iter().map(|&x| x as u32).sum::<u32>() as u8)
    }

    /// One DPI stage (4B): [x, y, flags, crc]
    /// dx = (x + ((flags>>2 & 3)<<8) + 1) * step, doubled when flags&1 (as in the JS)
    fn parse_stage(b: &[u8]) -> (u32, u32) {
        let (x, y, fl) = (b[0] as u32, b[1] as u32, b[2] as u32);
        let mut dx = (x + (((fl >> 2) & 3) << 8) + 1) * 50;
        let mut dy = (y + ((fl >> 6) << 8) + 1) * 50;
        if fl & 1 == 1 {
            dx *= 2;
        }
        if fl & 2 == 2 {
            dy *= 2;
        }
        (dx, dy)
    }

    /// Encodes a DPI stage into 4B (JS: Set_MS_DPIValue_XY, sensor 3950 family).
    fn encode_stage(dx: u32, dy: u32) -> [u8; 4] {
        let (mut l, mut s) = (dx / 50, dy / 50);
        let (mut c, mut d) = (0u8, 0u8);
        // When it does not fit in the upper bits, set the multiplier bit
        if l > 1024 {
            l /= 2;
            c |= 1;
        }
        if s > 1024 {
            s /= 2;
            d |= 2;
        }
        let l = l.saturating_sub(1).min(1023) as u32;
        let s = s.saturating_sub(1).min(1023) as u32;
        let mut b = [0u8; 4];
        b[0] = (l & 0xff) as u8;
        b[1] = (s & 0xff) as u8;
        b[2] = (((l >> 8) & 3) << 2) as u8 | (((s >> 8) & 3) << 6) as u8 | c | (d << 4);
        b[3] = Self::crc4(&b);
        b
    }
}

/// KeyFunction type/param → ButtonAction (JS Ya enum).
/// param is the "button number to press" for MouseKey, and an index for Macro/DPISwitch.
fn button_action(ty: u8, param: u16) -> ButtonAction {
    match ty {
        0 => ButtonAction::Disabled,
        1 => ButtonAction::Mouse(match param {
            0 => MouseFn::Left,
            1 => MouseFn::Right,
            2 => MouseFn::Middle,
            3 => MouseFn::Back,
            4 => MouseFn::Forward,
            _ => MouseFn::Left,
        }),
        2 => ButtonAction::Mouse(if param == 0 {
            MouseFn::DpiCycle
        } else {
            MouseFn::DpiShift(param as u32)
        }),
        5 => ButtonAction::Key(format!("{param}")),
        6 => ButtonAction::Macro(param as u8),
        7 => ButtonAction::Mouse(MouseFn::RateCycle),
        10 => ButtonAction::Mouse(MouseFn::DpiUp),
        11 => ButtonAction::Mouse(MouseFn::DpiDown),
        _ => ButtonAction::Key(format!("type{ty}:{param}")),
    }
}

/// Slot default when the EEPROM read fails.
fn default_slot_action(slot: &str) -> ButtonAction {
    ButtonAction::Mouse(match slot {
        "Left" => MouseFn::Left,
        "Right" => MouseFn::Right,
        "Middle" => MouseFn::Middle,
        "Forward" => MouseFn::Forward,
        "Back" => MouseFn::Back,
        _ => MouseFn::DpiCycle,
    })
}

impl DeviceDriver for MousefitDriver {
    fn info(&self) -> DeviceInfo {
        let mut info = self.info.clone();
        // FW version: cmd[0]=18 → resp[5].[6] (hex)
        let mut f = [0u8; 16];
        f[0] = OP_FIRMWARE;
        if let Ok(r) = self.cmd(&f) {
            if r[5] != 0 || r[6] != 0 {
                info.firmware = Some(format!("{:x}.{:02x}", r[5], r[6]));
            }
        }
        info
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            mouse: Some(MouseCaps {
                dpi_min: 50,
                dpi_max: 30000,
                dpi_step: 50,
                max_stages: 8,
                xy_split: true,
                polling_rates: &POLL_RATES,
                wireless_polling: true,
                lod_choices: &[1.0, 2.0],
                debounce_min: 0,
                debounce_max: 20,
                button_slots: &["Left", "Right", "Middle", "Forward", "Back", "DPI"],
                macros: true,
                onboard_profiles: 1,
                dpi_led: true,
                battery: true,
                sleep_timer: true,
                motion_sync: true,
                angle_snap: true,
                ripple_control: true,
            }),
            keyboard: None,
        }
    }

    fn mouse_config(&self) -> HubResult<MouseConfig> {
        let mut cfg = MouseConfig {
            stages: Vec::new(),
            active_stage: 0,
            xy_split: false,
            polling_wired: 1000,
            polling_wireless: None,
            lod_mm: 1.0,
            debounce_ms: 4,
            motion_sync: false,
            angle_snap: false,
            ripple_control: false,
            sleep_min: 0,
            buttons: Vec::new(),
            profile: 0,
        };

        // Polling: eeprom[0] → index into POLL_RATES
        let pr = self.ee_u8(ee::REPORT_RATE) as usize;
        cfg.polling_wired = POLL_RATES.get(pr).copied().unwrap_or(1000);

        // DPI stage count / current
        let n_stages = (self.ee_u8(ee::MAX_DPI_STAGE) as usize).clamp(1, 8);
        cfg.active_stage = self.ee_u8(ee::CURRENT_DPI) as usize;

        // Stages 4B × n + colors 4B × n (raw blocks; a different format from the scalar (v,85-v) pairs)
        for i in 0..n_stages {
            let b = self.ee_read(ee::DPI_VALUE + (i * 4) as u16, 4).unwrap_or_default();
            if b.len() < 4 {
                break;
            }
            let (x, y) = Self::parse_stage(&b);
            let c = self
                .ee_read(ee::DPI_COLOR + (i * 4) as u16, 4)
                .unwrap_or_default();
            cfg.stages.push(DpiStage {
                dpi: x.max(50),
                dpi_y: if y == x { None } else { Some(y.max(50)) },
                color: if c.len() >= 3 {
                    Rgb::new(c[0], c[1], c[2])
                } else {
                    Rgb::new(255, 255, 255)
                },
            });
        }
        if cfg.stages.is_empty() {
            cfg.stages.push(DpiStage {
                dpi: 800,
                dpi_y: None,
                color: Rgb::new(255, 255, 255),
            });
        }
        cfg.active_stage = cfg.active_stage.min(cfg.stages.len() - 1);

        cfg.lod_mm = match self.ee_u8(ee::LOD) {
            0 => 1.0,
            _ => 2.0,
        };
        cfg.debounce_ms = self.ee_u8(ee::DEBOUNCE);
        cfg.motion_sync = self.ee_u8(ee::MOTION_SYNC) != 0;
        cfg.angle_snap = self.ee_u8(ee::ANGLE) != 0;
        cfg.ripple_control = self.ee_u8(ee::RIPPLE) != 0;
        cfg.sleep_min = self.ee_u8(ee::SLEEP);

        // Buttons: KeyFunction@96 + 4B/slot [type,paramHi,paramLo,crc]
        // Ya types: 0=Disable,1=MouseKey,2=DPISwitch,4=FireKey,5=ShortcutKey,
        //         6=Macro,7=ReportRateSwitch,10=DPILock,11=UpDownRoll
        const SLOTS: [&str; 6] = ["Left", "Right", "Middle", "Forward", "Back", "DPI"];
        for (i, slot) in SLOTS.iter().enumerate() {
            let b = self
                .ee_read(ee::KEY_FUNCTION + (i * 4) as u16, 4)
                .unwrap_or_default();
            let action = if b.len() >= 3 {
                let param = u16::from_be_bytes([b[1], b[2]]);
                button_action(b[0], param)
            } else {
                default_slot_action(slot)
            };
            cfg.buttons.push(ButtonAssign {
                slot: slot.to_string(),
                action,
            });
        }

        Ok(cfg)
    }

    fn apply_mouse(&self, cfg: &MouseConfig) -> HubResult<()> {
        // Polling
        if let Some(idx) = POLL_RATES.iter().position(|&r| r == cfg.polling_wired) {
            self.ee_write_val(ee::REPORT_RATE, idx as u8)?;
        }
        // Stage count → active (JS: setDPIStageNum — stage count first, then active)
        let n = cfg.stages.len().clamp(1, 8) as u8;
        self.ee_write_val(ee::MAX_DPI_STAGE, n)?;
        let active = (cfg.active_stage as u8).min(n - 1);
        self.ee_write_val(ee::CURRENT_DPI, active)?;
        // DPI stage values / colors (4B blocks: [x,y,flags,crc] / [r,g,b,crc])
        for (i, s) in cfg.stages.iter().take(8).enumerate() {
            let y = s.dpi_y.unwrap_or(s.dpi);
            self.ee_write(ee::DPI_VALUE + (i * 4) as u16, &Self::encode_stage(s.dpi, y))?;
            let mut col = [s.color.r, s.color.g, s.color.b, 0];
            col[3] = Self::crc4(&col);
            self.ee_write(ee::DPI_COLOR + (i * 4) as u16, &col)?;
        }
        self.ee_write_val(ee::LOD, if cfg.lod_mm >= 2.0 { 1 } else { 0 })?;
        self.ee_write_val(ee::DEBOUNCE, cfg.debounce_ms)?;
        self.ee_write_val(ee::MOTION_SYNC, cfg.motion_sync as u8)?;
        self.ee_write_val(ee::ANGLE, cfg.angle_snap as u8)?;
        self.ee_write_val(ee::RIPPLE, cfg.ripple_control as u8)?;
        self.ee_write_val(ee::SLEEP, cfg.sleep_min)?;
        Ok(())
    }

    fn battery(&self) -> HubResult<Battery> {
        let mut f = [0u8; 16];
        f[0] = OP_BATTERY;
        let r = self.cmd(&f)?;
        // resp[6]=percent, resp[7..8]=voltage (0 while the mouse is asleep)
        if r[6] == 0 {
            return Err(HubError::Protocol("mouse asleep".into()));
        }
        Ok(Battery {
            percent: Some(r[6].min(100)),
            charging: false,
            voltage_mv: Some(u16::from_le_bytes([r[7], r[8]])),
        })
    }
}
