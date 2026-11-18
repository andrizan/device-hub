//! Table of known devices.
//!
//! **When adding a new device, the first step is to add one line here**
//! (see `docs/ADD_DEVICE.md` for the full procedure).
//!
//! Take the VID/PID from a real-device enumeration or from the JS bundle of the web configurator
//! (the filters of `navigator.hid.requestDevice` / mouse.fit's `Config/env-models.json`).
//! One device may use separate PIDs for wired, dongle and 8K dongle,
//! so register one line for each (all pointing at the same model / driver).

use crate::model::{DeviceKind, DriverKind, LayoutKey, Transport};

/// One known device.
#[derive(Clone, Copy, Debug)]
pub struct KnownDevice {
    pub vid: u16,
    pub pid: u16,
    /// Narrows down the configuration interface (vendor-defined usage page). None if unknown.
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub kind: DeviceKind,
    pub vendor: &'static str,
    /// Model identifier (the key the driver uses to look up per-model differences).
    pub model: &'static str,
    /// Display name.
    pub name: &'static str,
    pub transport: Transport,
    pub driver: DriverKind,
    /// Whether it has been verified on real hardware (it is enumerated even if false; a marker for reflecting in the docs table).
    pub verified: bool,
}

const fn mf(
    pid: u16,
    model: &'static str,
    name: &'static str,
    transport: Transport,
    verified: bool,
) -> KnownDevice {
    KnownDevice {
        vid: 0x33E4,
        pid,
        usage_page: Some(0xFF02),
        usage: Some(0x02),
        kind: DeviceKind::Mouse,
        vendor: "G-Wolves",
        model,
        name,
        transport,
        driver: DriverKind::Mousefit,
        verified,
    }
}

/// The table. Add new devices here.
///
/// mouse.fit PIDs come from `PIDWired` / `PIDWireless` / `PIDWireless4K8K` in
/// `https://www.mouse.fit/Config/env-models.json` (fetched 2026-11). usage_page/usage are
/// assumed to be the same as on the real HTS Plus Pro (0xFF02 / 0x02) — **other models need confirmation on real hardware**.
pub const KNOWN: &[KnownDevice] = &[
    // ---------------------------------------------------------------- keyboards
    KnownDevice {
        vid: 0x1CA6,
        pid: 0x3009, // SU66 Max — confirmed by real-device enumeration (also listed as 12297 in the filters of hub.xsyd.top)
        usage_page: Some(0xFFB0),
        usage: Some(0x01),
        kind: DeviceKind::Keyboard,
        vendor: "XSYD",
        model: "su66max",
        name: "SU66 Max",
        transport: Transport::Wired,
        driver: DriverKind::Xsyd,
        verified: true,
    },
    // ---------------------------------------------------------------- mice (mouse.fit)
    // HTS Plus Pro (PAW3955 version JM_052_3955). Wired also uses the report-8/EEPROM path — verified on real hardware.
    mf(
        0x5219,
        "hts-plus-pro",
        "HTS Plus Pro",
        Transport::Wired,
        true,
    ),
    // 8K dongle shared by the 3955/3950 series (report-8/EEPROM path, verified on real hardware).
    // The inner model cannot be queried, so per-model differences are looked up by model.
    // The display name is the paired model's name (shown as the mouse inside, not the receiver's own product string).
    mf(
        0x3854,
        "gw-dongle-3854",
        "HTS Plus Pro",
        Transport::Wireless24G,
        true,
    ),
    // HTX Ultra = "HTXU" in mouse.fit. There are 3 variants depending on the sensor.
    mf(0x5608, "htx-ultra", "HTX Ultra", Transport::Wired, false),
    mf(
        0x5617,
        "htx-ultra",
        "HTX Ultra",
        Transport::Wireless24G,
        false,
    ),
    mf(
        0x5618,
        "htx-ultra-3950",
        "HTX Ultra (3950)",
        Transport::Wired,
        false,
    ),
    mf(
        0x5619,
        "htx-ultra-3955",
        "HTX Ultra (3955)",
        Transport::Wired,
        false,
    ),
    // ---------------------------------------------------------------- USB DAC (peq.szwalkplay.com)
    // TP55 — real-device enumeration: 3302:4383 if=0 up=000C u=01 "TP55".
    // Settings go through report 0x4B on the consumer control IF (CB-series S1 protocol).
    // Beware: [1,255] switches to the bootloader (0666:0888 "USB20").
    KnownDevice {
        vid: 0x3302,
        pid: 0x4383,
        usage_page: Some(0x000C),
        usage: Some(0x01),
        kind: DeviceKind::Other,
        vendor: "WalkPlay",
        model: "tp55",
        name: "TP55",
        transport: Transport::Wired,
        driver: DriverKind::Walkplay,
        verified: false,
    },
    // ATK F1 V3 ULTIMATE+ — real-device enumeration 373b:12a7. config = ff02/u2 (report-8/EEPROM).
    // Product string "ATK F1 V3 ULTIMATE+". CID/MID=1,62 (matches the official DB).
    KnownDevice {
        vid: 0x373B,
        pid: 0x12A7,
        usage_page: Some(0xFF02),
        usage: Some(0x02),
        kind: DeviceKind::Mouse,
        vendor: "ATK",
        model: "atk-f1v3u",
        name: "ATK F1 V3 ULTIMATE+",
        transport: Transport::Wired,
        driver: DriverKind::Atk,
        verified: true,
    },
    // ATK 8K dongle "Wireless mouse 8k dongle-L" (373b:1278).
    // Relays the same 16B/report-8 protocol — the inner model is determined by cmd16 CID/MID.
    // While the mouse is asleep, relayed commands time out (the dongle's own cmd29 etc. still respond).
    KnownDevice {
        vid: 0x373B,
        pid: 0x1278,
        usage_page: Some(0xFF02),
        usage: Some(0x02),
        kind: DeviceKind::Mouse,
        vendor: "ATK",
        model: "atk-dongle",
        name: "ATK wireless dongle",
        transport: Transport::Wireless24G,
        driver: DriverKind::Atk,
        verified: true,
    },
    // ---------------------------------------------------------------- mice (lamzu.net)
    // LAMZU Maya X V2 (LM20) wired — real-device enumeration 37b0:0044.
    // Settings = 64B feature report on if2 FFFF/00, notifications = input report on if1 FFA0/01.
    KnownDevice {
        vid: 0x37B0,
        pid: 0x0044,
        usage_page: Some(0xFFFF),
        usage: Some(0x00),
        kind: DeviceKind::Mouse,
        vendor: "LAMZU",
        model: "maya-x-v2",
        name: "LAMZU MAYA X V2",
        transport: Transport::Wired,
        driver: DriverKind::Lamzu,
        verified: true,
    },
    // LAMZU Orbital 8K dongle (37b0:0048) — relays mouse settings with the same FFFF:00 feature /
    // FFA0:01 notification layout (confirmed by real-device enumeration).
    KnownDevice {
        vid: 0x37B0,
        pid: 0x0048,
        usage_page: Some(0xFFFF),
        usage: Some(0x00),
        kind: DeviceKind::Mouse,
        vendor: "LAMZU",
        model: "lamzu-dongle",
        // The initial value is the receiver name — once the driver determines the paired mouse with GetDevicePID,
        // it sets info.name/model to the product name (e.g. LAMZU Maya X V2 / maya-x-v2).
        name: "LAMZU 8K receiver",
        transport: Transport::Wireless24G,
        driver: DriverKind::Lamzu,
        verified: true,
    },
    // Same LM20 generation (official env-models.json). The protocol is shared with MAYA X V2 — not confirmed on real hardware.
    KnownDevice {
        vid: 0x37B0,
        pid: 0x0050,
        usage_page: Some(0xFFFF),
        usage: Some(0x00),
        kind: DeviceKind::Mouse,
        vendor: "LAMZU",
        model: "maya-v2",
        name: "LAMZU MAYA V2",
        transport: Transport::Wired,
        driver: DriverKind::Lamzu,
        verified: false,
    },
    // Same LM20 generation (official env-models.json). The protocol is shared with MAYA X V2 — not confirmed on real hardware.
    KnownDevice {
        vid: 0x37B0,
        pid: 0x0052,
        usage_page: Some(0xFFFF),
        usage: Some(0x00),
        kind: DeviceKind::Mouse,
        vendor: "LAMZU",
        model: "maya-m",
        name: "LAMZU MAYA M",
        transport: Transport::Wired,
        driver: DriverKind::Lamzu,
        verified: false,
    },
    // Same LM20 generation (official env-models.json). The protocol is shared with MAYA X V2 — not confirmed on real hardware.
    KnownDevice {
        vid: 0x37B0,
        pid: 0x0056,
        usage_page: Some(0xFFFF),
        usage: Some(0x00),
        kind: DeviceKind::Mouse,
        vendor: "LAMZU",
        model: "orcus-v2",
        name: "LAMZU ORCUS V2",
        transport: Transport::Wired,
        driver: DriverKind::Lamzu,
        verified: false,
    },
    // Same LM20 generation (official env-models.json). The protocol is shared with MAYA X V2 — not confirmed on real hardware.
    KnownDevice {
        vid: 0x37B0,
        pid: 0x0046,
        usage_page: Some(0xFFFF),
        usage: Some(0x00),
        kind: DeviceKind::Mouse,
        vendor: "LAMZU",
        model: "atlantis-mini-v2",
        name: "LAMZU ATLANTIS MINI V2",
        transport: Transport::Wired,
        driver: DriverKind::Lamzu,
        verified: false,
    },
    // 8K receiver for ATLANTIS MINI V2 (official _8KDongle=0054). The paired mouse is determined with GetDevicePID.
    KnownDevice {
        vid: 0x37B0,
        pid: 0x0054,
        usage_page: Some(0xFFFF),
        usage: Some(0x00),
        kind: DeviceKind::Mouse,
        vendor: "LAMZU",
        model: "lamzu-dongle",
        name: "LAMZU 8K receiver",
        transport: Transport::Wireless24G,
        driver: DriverKind::Lamzu,
        verified: false,
    },
    // ---------------------------------------------------------------- RongYuan (VID 3151)
    // These PIDs are shared by many products from the same OEM, so an entry
    // here must not claim every device that carries one. Two checks keep
    // another vendor's board from being taken for these:
    // - `SHARED_IDS` below: the entry only matches when the HID product
    //   string names the product.
    // - the driver asks the device for its id (`0x8F`) before it reads or
    //   writes anything, and refuses an id it does not know.
    //
    // Fantech MAXFIT81, cable. Enumerated here as 3151:4010 if2 FFFF:02,
    // id 694, firmware 5.08, yc500 command family.
    KnownDevice {
        vid: 0x3151,
        pid: 0x4010,
        usage_page: Some(0xFFFF),
        usage: Some(0x02),
        kind: DeviceKind::Keyboard,
        vendor: "Fantech",
        model: "maxfit81",
        name: "MAXFIT81",
        transport: Transport::Wired,
        driver: DriverKind::Ry,
        verified: true,
    },
    // A RongYuan keyboard's 2.4G receiver. Its product string is the generic
    // "2.4G Wireless Keyboard" whatever keyboard it belongs to, so it is
    // listed under a generic name and the driver names it once the keyboard
    // behind it has answered. Only its status reply was read here because the
    // keyboard was on the cable.
    KnownDevice {
        vid: 0x3151,
        pid: 0x4011,
        usage_page: Some(0xFFFF),
        usage: Some(0x02),
        kind: DeviceKind::Keyboard,
        vendor: "RongYuan",
        model: "ry-keyboard-receiver",
        name: "2.4G keyboard receiver",
        transport: Transport::Wireless24G,
        driver: DriverKind::Ry,
        verified: false,
    },
    // Fantech Tanto PRO WG13P through its 2.4G receiver (3151:402d). The mouse
    // answers id 3109 (the "Tanto PRO S" platform, PAW3950) through the relay.
    KnownDevice {
        vid: 0x3151,
        pid: 0x402D,
        usage_page: Some(0xFFFF),
        usage: Some(0x02),
        kind: DeviceKind::Mouse,
        vendor: "Fantech",
        model: "ry-mouse-receiver",
        name: "Tanto PRO WG13P",
        transport: Transport::Wireless24G,
        driver: DriverKind::Ry,
        verified: true,
    },
    // The same mouse on its cable (3151:4026, a PID many RongYuan mice
    // share). It answers the same commands directly, without the relay.
    KnownDevice {
        vid: 0x3151,
        pid: 0x4026,
        usage_page: Some(0xFFFF),
        usage: Some(0x02),
        kind: DeviceKind::Mouse,
        vendor: "Fantech",
        model: "tanto-pro",
        name: "Tanto PRO WG13P",
        transport: Transport::Wired,
        driver: DriverKind::Ry,
        verified: true,
    },
];

/// A Bluetooth Classic device, matched by its whole paired name: RFCOMM has
/// no VID/PID to key on, and the vendors' own apps tell these models apart by
/// name or by an advert that is gone once the device is connected.
#[derive(Clone, Copy, Debug)]
pub struct KnownBtDevice {
    /// Paired name, compared whole and case-insensitively.
    pub name: &'static str,
    pub kind: DeviceKind,
    pub vendor: &'static str,
    pub model: &'static str,
    pub driver: DriverKind,
    pub verified: bool,
}

/// Bluetooth devices. `DriverKind::Unsupported` entries are listed so the UI
/// can show them as detected; they have no settings protocol implemented.
pub const KNOWN_BT: &[KnownBtDevice] = &[
    // EarFun Air Pro 4+ (model code TW700, Qualcomm): GAIA over SPP. Written
    // from published protocol notes; no unit was available to test.
    KnownBtDevice {
        name: "EarFun Air Pro 4+",
        kind: DeviceKind::Audio,
        vendor: "EarFun",
        model: "earfun-tw700",
        driver: DriverKind::Earfun,
        verified: false,
    },
    // Same hardware and settings; EarFun's app only adds AI features on top.
    KnownBtDevice {
        name: "EarFun Air Pro 4+AI",
        kind: DeviceKind::Audio,
        vendor: "EarFun",
        model: "earfun-tw700",
        driver: DriverKind::Earfun,
        verified: false,
    },
    // EarFun Wave Pro (HP200, BES chip). Its SPP channel opens but answers
    // neither the GAIA nor the EF firmware query, and its own protocol is not
    // documented anywhere we could find.
    KnownBtDevice {
        name: "EarFun Wave Pro",
        kind: DeviceKind::Audio,
        vendor: "EarFun",
        model: "earfun-hp200",
        driver: DriverKind::Unsupported,
        verified: false,
    },
    KnownBtDevice {
        name: "HUAWEI FreeBuds SE 3",
        kind: DeviceKind::Audio,
        vendor: "HUAWEI",
        model: "freebuds-se3",
        driver: DriverKind::Unsupported,
        verified: false,
    },
];

/// The known Bluetooth device with this paired name.
pub fn match_bt(name: &str) -> Option<&'static KnownBtDevice> {
    KNOWN_BT
        .iter()
        .find(|k| k.name.eq_ignore_ascii_case(name.trim()))
}

/// USB ids that unrelated products share. An entry in `KNOWN` with one of
/// these ids only stands for the product whose HID product string contains
/// the text here (compared without case); any other device on the same id is
/// left alone rather than shown under this product's name.
const SHARED_IDS: &[(u16, u16, &str)] = &[
    (0x3151, 0x4010, "MAXFIT81"),
    (0x3151, 0x4026, "Tanto PRO"),
    (0x3151, 0x402D, "Tanto PRO"),
];

/// The known device matching an enumeration result (VID/PID/usage). A `0xFFFF` placeholder never matches.
///
/// `product` is the HID product string, used for ids in `SHARED_IDS`.
pub fn match_device(
    vid: u16,
    pid: u16,
    usage_page: u16,
    usage: u16,
    product: Option<&str>,
) -> Option<&'static KnownDevice> {
    let named = SHARED_IDS
        .iter()
        .find(|s| (s.0, s.1) == (vid, pid))
        .is_none_or(|s| {
            product.is_some_and(|p| p.to_ascii_lowercase().contains(&s.2.to_ascii_lowercase()))
        });
    KNOWN.iter().find(|k| {
        k.vid == vid
            && k.pid == pid
            && k.vid != 0xFFFF
            && named
            && k.usage_page.is_none_or(|p| p == usage_page)
            && k.usage.is_none_or(|u| u == usage)
    })
}

// ------------------------------------------------------------------ layouts

/// Physical layout of su66max (derived from the real matrix: 15col×5row + split space).
///
/// `index` = the protocol's matrix number `(matrix_row << 4) | matrix_col`
/// (matrix_row is 1..=5, col is 0..=14 — confirmed from a real-device dump).
/// The visual x/y/w are an estimated placement of a standard 65% board. SU66MAX_MATRIX maps index→(row,col).
pub fn su66max_layout() -> Vec<LayoutKey> {
    fn k(row: u8, col: u8, x: f32, y: f32, w: f32, label: &str) -> LayoutKey {
        LayoutKey {
            id: ((row as u16) << 4) | col as u16,
            x,
            y,
            w,
            h: 1.0,
            label: label.into(),
        }
    }
    vec![
        // Row 0: Esc + number row + Backspace (no right-column key)
        k(1, 0, 0.0, 0.0, 1.0, "Esc"),
        k(1, 1, 1.0, 0.0, 1.0, "1"),
        k(1, 2, 2.0, 0.0, 1.0, "2"),
        k(1, 3, 3.0, 0.0, 1.0, "3"),
        k(1, 4, 4.0, 0.0, 1.0, "4"),
        k(1, 5, 5.0, 0.0, 1.0, "5"),
        k(1, 6, 6.0, 0.0, 1.0, "6"),
        k(1, 7, 7.0, 0.0, 1.0, "7"),
        k(1, 8, 8.0, 0.0, 1.0, "8"),
        k(1, 9, 9.0, 0.0, 1.0, "9"),
        k(1, 10, 10.0, 0.0, 1.0, "0"),
        k(1, 11, 11.0, 0.0, 1.0, "-"),
        k(1, 12, 12.0, 0.0, 1.0, "="),
        k(1, 13, 13.0, 0.0, 2.0, "Backspace"),
        // Row 1: Tab + QWERTY
        k(2, 0, 0.0, 1.0, 1.5, "Tab"),
        k(2, 1, 1.5, 1.0, 1.0, "Q"),
        k(2, 2, 2.5, 1.0, 1.0, "W"),
        k(2, 3, 3.5, 1.0, 1.0, "E"),
        k(2, 4, 4.5, 1.0, 1.0, "R"),
        k(2, 5, 5.5, 1.0, 1.0, "T"),
        k(2, 6, 6.5, 1.0, 1.0, "Y"),
        k(2, 7, 7.5, 1.0, 1.0, "U"),
        k(2, 8, 8.5, 1.0, 1.0, "I"),
        k(2, 9, 9.5, 1.0, 1.0, "O"),
        k(2, 10, 10.5, 1.0, 1.0, "P"),
        k(2, 11, 11.5, 1.0, 1.0, "["),
        k(2, 12, 12.5, 1.0, 1.0, "]"),
        k(2, 13, 13.5, 1.0, 1.5, "\\"),
        // Row 2: Caps + ASDF + Enter
        k(3, 0, 0.0, 2.0, 1.75, "Caps"),
        k(3, 1, 1.75, 2.0, 1.0, "A"),
        k(3, 2, 2.75, 2.0, 1.0, "S"),
        k(3, 3, 3.75, 2.0, 1.0, "D"),
        k(3, 4, 4.75, 2.0, 1.0, "F"),
        k(3, 5, 5.75, 2.0, 1.0, "G"),
        k(3, 6, 6.75, 2.0, 1.0, "H"),
        k(3, 7, 7.75, 2.0, 1.0, "J"),
        k(3, 8, 8.75, 2.0, 1.0, "K"),
        k(3, 9, 9.75, 2.0, 1.0, "L"),
        k(3, 10, 10.75, 2.0, 1.0, ";"),
        k(3, 11, 11.75, 2.0, 1.0, "'"),
        k(3, 13, 12.75, 2.0, 2.25, "Enter"),
        // Row 3: LShift + ZXCV + RShift + ↑ + Del — GH60 15u row with arrows
        // (the same 1.75 Shift / 1.25 RShift / 1u right column as the real board).
        k(4, 0, 0.0, 3.0, 1.75, "LShift"),
        k(4, 2, 1.75, 3.0, 1.0, "Z"),
        k(4, 3, 2.75, 3.0, 1.0, "X"),
        k(4, 4, 3.75, 3.0, 1.0, "C"),
        k(4, 5, 4.75, 3.0, 1.0, "V"),
        k(4, 6, 5.75, 3.0, 1.0, "B"),
        k(4, 7, 6.75, 3.0, 1.0, "N"),
        k(4, 8, 7.75, 3.0, 1.0, "M"),
        k(4, 9, 8.75, 3.0, 1.0, ","),
        k(4, 10, 9.75, 3.0, 1.0, "."),
        k(4, 11, 10.75, 3.0, 1.0, "/"),
        k(4, 12, 11.75, 3.0, 1.25, "RShift"),
        k(4, 13, 13.0, 3.0, 1.0, "↑"),
        k(4, 14, 14.0, 3.0, 1.0, "Del"),
        // Row 4: modifiers + split Space (real-board spec 2.25+1.75+2.25) + RAlt/Fn + ←↓→
        k(5, 0, 0.0, 4.0, 1.25, "LCtrl"),
        k(5, 1, 1.25, 4.0, 1.25, "LWin"),
        k(5, 2, 2.5, 4.0, 1.25, "LAlt"),
        k(5, 4, 3.75, 4.0, 2.25, "Space"),
        k(5, 6, 6.0, 4.0, 1.75, "Space"),
        k(5, 8, 7.75, 4.0, 2.25, "Space"),
        k(5, 9, 10.0, 4.0, 1.0, "RAlt"),
        k(5, 10, 11.0, 4.0, 1.0, "Fn"),
        k(5, 12, 12.0, 4.0, 1.0, "←"),
        k(5, 13, 13.0, 4.0, 1.0, "↓"),
        k(5, 14, 14.0, 4.0, 1.0, "→"),
    ]
}

/// id of su66max_layout → (matrix_row, matrix_col).
/// The position in the key-code stream is `row * 30 + col` (30B rows × up to 8 rows).
pub fn su66max_matrix_pos(id: u16) -> (u8, u8) {
    ((id >> 4) as u8, (id & 0x0f) as u8)
}

/// Fantech MAXFIT81 (RongYuan id 694): 81 keys, one of them the knob's push.
///
/// `id` is the firmware's keymap slot, `column * 6 + row`, read back from the
/// board's own default keymap (each slot still held its factory key). The
/// knob's two turn directions are slots of their own and are listed as the
/// two narrow keys beside it, so they can be remapped like any key.
///
/// Positions are a drawing of a 16u-wide 75% board. The right-hand column
/// (Del / PgUp / PgDn) is placed by convention; the matrix does not say which
/// row each of those three sits on.
pub fn maxfit81_layout() -> Vec<LayoutKey> {
    fn k(id: u16, x: f32, y: f32, w: f32, label: &str) -> LayoutKey {
        LayoutKey {
            id,
            x,
            y,
            w,
            h: 1.0,
            label: label.into(),
        }
    }
    let mut keys = vec![
        k(0, 0.0, 0.0, 1.0, "Esc"),
        k(91, 14.0, 0.0, 0.5, "−"),
        k(85, 14.5, 0.0, 1.0, "Knob"),
        k(90, 15.5, 0.0, 0.5, "+"),
    ];
    // F1..F12 in three groups of four; slots step by one column (6).
    for i in 0..12u16 {
        let x = 1.25 + i as f32 + 0.25 * (i / 4) as f32;
        keys.push(k(12 + i * 6, x, 0.0, 1.0, &format!("F{}", i + 1)));
    }
    let y1 = 1.25;
    keys.push(k(1, 0.0, y1, 1.0, "`"));
    for (i, label) in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "="]
        .iter()
        .enumerate()
    {
        keys.push(k(7 + i as u16 * 6, 1.0 + i as f32, y1, 1.0, label));
    }
    keys.push(k(79, 13.0, y1, 2.0, "Backspace"));
    keys.push(k(86, 15.0, y1, 1.0, "Del"));

    let y2 = 2.25;
    keys.push(k(2, 0.0, y2, 1.5, "Tab"));
    for (i, label) in ["Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "[", "]"]
        .iter()
        .enumerate()
    {
        keys.push(k(8 + i as u16 * 6, 1.5 + i as f32, y2, 1.0, label));
    }
    keys.push(k(80, 13.5, y2, 1.5, "\\"));
    keys.push(k(87, 15.0, y2, 1.0, "PgUp"));

    let y3 = 3.25;
    keys.push(k(3, 0.0, y3, 1.75, "Caps"));
    for (i, label) in ["A", "S", "D", "F", "G", "H", "J", "K", "L", ";", "'"]
        .iter()
        .enumerate()
    {
        keys.push(k(9 + i as u16 * 6, 1.75 + i as f32, y3, 1.0, label));
    }
    keys.push(k(81, 12.75, y3, 2.25, "Enter"));
    keys.push(k(88, 15.0, y3, 1.0, "PgDn"));

    let y4 = 4.25;
    keys.push(k(4, 0.0, y4, 2.25, "LShift"));
    for (i, label) in ["Z", "X", "C", "V", "B", "N", "M", ",", ".", "/"]
        .iter()
        .enumerate()
    {
        keys.push(k(16 + i as u16 * 6, 2.25 + i as f32, y4, 1.0, label));
    }
    keys.push(k(76, 12.25, y4, 1.75, "RShift"));
    keys.push(k(82, 14.0, y4, 1.0, "↑"));

    let y5 = 5.25;
    keys.extend([
        k(5, 0.0, y5, 1.25, "LCtrl"),
        k(17, 1.25, y5, 1.25, "LWin"),
        k(23, 2.5, y5, 1.25, "LAlt"),
        k(41, 3.75, y5, 6.25, "Space"),
        k(59, 10.0, y5, 1.0, "RAlt"),
        k(65, 11.0, y5, 1.0, "Fn"),
        k(71, 12.0, y5, 1.0, "RCtrl"),
        k(77, 13.0, y5, 1.0, "←"),
        k(83, 14.0, y5, 1.0, "↓"),
        k(89, 15.0, y5, 1.0, "→"),
    ]);
    keys
}

#[cfg(test)]
mod ry_tests {
    use super::*;

    #[test]
    fn maxfit81_has_83_distinct_slots() {
        // 81 keys (the knob's push among them) plus the two turn directions.
        let layout = maxfit81_layout();
        assert_eq!(layout.len(), 83);
        let mut ids: Vec<u16> = layout.iter().map(|k| k.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 83);
        assert!(ids.iter().all(|id| *id < 128));
        // Spot checks against the keymap read from the board.
        let slot = |label: &str| layout.iter().find(|k| k.label == label).unwrap().id;
        assert_eq!(slot("F12"), 78);
        assert_eq!(slot("0"), 61);
        assert_eq!(slot("]"), 74);
        assert_eq!(slot("'"), 69);
        assert_eq!(slot("/"), 70);
    }

    #[test]
    fn tanto_pro_is_known_on_cable_and_receiver() {
        let tanto = Some("Fantech Tanto PRO WG13P");
        let model =
            |pid| match_device(0x3151, pid, 0xFFFF, 0x02, tanto).map(|k| (k.model, k.transport));
        assert_eq!(model(0x4026), Some(("tanto-pro", Transport::Wired)));
        assert_eq!(
            model(0x402D),
            Some(("ry-mouse-receiver", Transport::Wireless24G))
        );
        // The keyboard-input collection of the same device is not a match.
        assert!(match_device(0x3151, 0x4026, 0x0001, 0x02, tanto).is_none());
    }

    #[test]
    fn shared_ids_do_not_claim_other_products() {
        let on = |pid, product| match_device(0x3151, pid, 0xFFFF, 0x02, product).is_some();
        // Other mice and keyboards ship with the very same ids.
        assert!(!on(0x4026, Some("AJ159 APEX")));
        assert!(!on(0x402D, Some("2.4G Wireless Mouse")));
        assert!(!on(0x4010, Some("Akko 5075B Plus")));
        // No product string to go by: not claimed either.
        assert!(!on(0x4026, None));
        assert!(on(0x4010, Some("MAXFIT81 Mechanical Keyboard")));
        assert!(on(0x4026, Some("fantech tanto pro wg13p")));
        // A keyboard receiver names no product, so it is listed under a
        // generic name; the driver identifies the keyboard behind it.
        let rx = match_device(0x3151, 0x4011, 0xFFFF, 0x02, Some("2.4G Wireless Keyboard"));
        assert_eq!(rx.unwrap().name, "2.4G keyboard receiver");
        // Ids outside the shared list are unaffected by the product string.
        assert!(match_device(0x1CA6, 0x3009, 0xFFB0, 0x01, None).is_some());
    }

    #[test]
    fn bluetooth_names_match_whole() {
        assert_eq!(match_bt("EarFun Air Pro 4+").unwrap().model, "earfun-tw700");
        assert_eq!(
            match_bt(" earfun air pro 4+ai ").unwrap().model,
            "earfun-tw700"
        );
        // The plain Air Pro 4 is a different model and is not registered.
        assert!(match_bt("EarFun Air Pro 4").is_none());
        assert_eq!(
            match_bt("EarFun Wave Pro").unwrap().driver,
            DriverKind::Unsupported
        );
    }
}
