//! devhub-obs — OBS source plugin.
//!
//! Placing `devhub-obs.dll` in `C:\ProgramData\obs-studio\plugins\devhub_obs\bin\64bit\`
//! makes "Device HUB Live / Keyboard / Mouse" appear in the add-source list.
//! (OBS 30.2+ does not scan %APPDATA%\plugins.)
//! obs.dll is resolved at runtime with GetProcAddress, so no import lib is needed.

pub mod config;
mod ffi;
pub mod live;
pub mod rawinput;
pub mod render;
mod usb_rate;

use core::ffi::c_void;
use std::collections::HashSet;
use std::ffi::CString;
use std::sync::atomic::AtomicPtr;
use std::sync::Mutex;

static MODULE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// Startup diagnostic log (%TEMP%\devhub_obs.log). It cannot go to the host's obs log, so
/// it is written directly to a file - for troubleshooting, always ON.
fn logline(s: &str) {
    use std::io::Write;
    let p = format!(r"{}\devhub_obs.log", std::env::temp_dir().display());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(f, "{:?} {s}", std::time::Instant::now());
    }
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("?").unwrap())
}

struct Src {
    mode: render::Mode,
    usb_missing: std::sync::atomic::AtomicBool,
    usb: Mutex<Option<usb_rate::Capture>>,
    usb_label: String,
    /// State of the latest snapshot — log the transition to an error to the diagnostic log only once.
    usb_status: Option<usb_rate::Status>,
    font: Option<fontdue::Font>,
    pix: Option<render::Pix>,
    tex: *mut c_void,
    last_seq: u64,
    /// Generation of the app settings (obs_overlay.json) applied last.
    last_cfg: u64,
    dirty: bool,
    /// For diagnosing the first render.
    first_render: Option<bool>,
    /// Property settings (written by update() from the UI thread).
    vis: Mutex<render::Vis>,
    /// Latest snapshot that serves as the interpolation target.
    live: live::Live,
    /// Interpolated values on display: route per key (mm, in samples order) / depth per button (0..1).
    /// Rendering (~60fps) is faster than data arrival (~15-25Hz), so
    /// drawing the raw values looks like a slideshow - smooth it with exponential interpolation.
    sm_route: Vec<f32>,
    sm_depth: Vec<f32>,
    last_tick: std::time::Instant,
    /// Handle holding the shared poller — dropping it decrements the refcount,
    /// and at 0 the HID worker stops. USB sources do not hold one.
    pool: Option<live::LiveRef>,
}

impl Src {
    fn new(mode: render::Mode) -> Self {
        Self {
            mode,
            usb: Mutex::default(),
            usb_missing: Default::default(),
            usb_label: String::new(),
            usb_status: None,
            font: render::font(),
            pix: None,
            tex: core::ptr::null_mut(),
            last_seq: u64::MAX,
            last_cfg: 0,
            dirty: false,
            first_render: None,
            vis: Mutex::new(render::Vis::default()),
            live: live::Live::default(),
            sm_route: Vec::new(),
            sm_depth: Vec::new(),
            last_tick: std::time::Instant::now(),
            pool: None,
        }
    }

    /// For live sources — hold the shared poller and wake it (first start or reuse).
    fn new_live(mode: render::Mode) -> Self {
        let mut s = Self::new(mode);
        s.pool = Some(live::shared());
        s
    }

    /// Latest poller snapshot (default if none is held).
    fn live_snap(&self) -> live::Live {
        self.pool
            .as_ref()
            .and_then(|p| p.live().lock().map(|l| l.clone()).ok())
            .unwrap_or_default()
    }

    /// Move the interpolated values toward the target. Returns true if still moving (redraw needed).
    fn advance(&mut self, dt: f32) -> bool {
        const TAU: f32 = 0.045; // s - at 60fps it tracks 90% in ~3 frames
        let k = 1.0 - (-dt / TAU).exp();
        let mut moving = false;
        match &self.live.kb {
            Some(kb) => {
                if self.sm_route.len() != kb.samples.len() {
                    // Catch up immediately on a configuration change (prevents a sweep-in on the first frame).
                    self.sm_route = kb.samples.iter().map(|s| s.route).collect();
                } else {
                    for (sm, t) in self
                        .sm_route
                        .iter_mut()
                        .zip(kb.samples.iter().map(|s| s.route))
                    {
                        let d = t - *sm;
                        if d.abs() > 0.004 {
                            *sm += d * k;
                            moving = true;
                        } else {
                            *sm = t;
                        }
                    }
                }
            }
            None => self.sm_route.clear(),
        }
        match &self.live.mouse {
            Some(m) => {
                if self.sm_depth.len() != m.depths.len() {
                    self.sm_depth = m.depths.iter().map(|b| b.depth).collect();
                } else {
                    for (sm, t) in self
                        .sm_depth
                        .iter_mut()
                        .zip(m.depths.iter().map(|b| b.depth))
                    {
                        let d = t - *sm;
                        if d.abs() > 0.003 {
                            *sm += d * k;
                            moving = true;
                        } else {
                            *sm = t;
                        }
                    }
                }
            }
            None => self.sm_depth.clear(),
        }
        moving
    }

    /// Snapshot for rendering with route/depth replaced by the interpolated values.
    fn smoothed_live(&self) -> live::Live {
        let mut l = self.live.clone();
        if let Some(kb) = &mut l.kb {
            for (smp, sm) in kb.samples.iter_mut().zip(&self.sm_route) {
                smp.route = *sm;
            }
        }
        if let Some(m) = &mut l.mouse {
            for (b, sm) in m.depths.iter_mut().zip(&self.sm_depth) {
                b.depth = *sm;
            }
        }
        l
    }
}

// ---------------- Source callbacks ----------------

unsafe extern "C" fn name_both(_d: *mut c_void) -> *const u8 {
    lt(c"devhub.src.both", c"Device HUB Live")
}
unsafe extern "C" fn name_kb(_d: *mut c_void) -> *const u8 {
    lt(c"devhub.src.kb", c"Device HUB Keyboard")
}
unsafe extern "C" fn name_mouse(_d: *mut c_void) -> *const u8 {
    lt(c"devhub.src.mouse", c"Device HUB Mouse")
}

unsafe extern "C" fn create_both(_s: *mut c_void, _src: *mut c_void) -> *mut c_void {
    logline("create (both)");
    Box::into_raw(Box::new(Src::new_live(render::Mode::Both))) as *mut c_void
}
unsafe extern "C" fn create_kb(_s: *mut c_void, _src: *mut c_void) -> *mut c_void {
    logline("create (kb)");
    Box::into_raw(Box::new(Src::new_live(render::Mode::Kb))) as *mut c_void
}
unsafe extern "C" fn create_mouse(_s: *mut c_void, _src: *mut c_void) -> *mut c_void {
    logline("create (mouse)");
    Box::into_raw(Box::new(Src::new_live(render::Mode::Mouse))) as *mut c_void
}

unsafe extern "C" fn destroy(d: *mut c_void) {
    if !d.is_null() {
        drop(Box::from_raw(d as *mut Src));
    }
}

/// The size changes with the shown keys, theme, and scale, so match the bitmap rendered last.
unsafe extern "C" fn get_width(d: *mut c_void) -> u32 {
    let s = &*(d as *mut Src);
    match &s.pix {
        Some(p) => p.w,
        None => render::size(&render_default_live(), s.mode, &s.cur_vis()).0,
    }
}
unsafe extern "C" fn get_height(d: *mut c_void) -> u32 {
    let s = &*(d as *mut Src);
    match &s.pix {
        Some(p) => p.h,
        None => render::size(&render_default_live(), s.mode, &s.cur_vis()).1,
    }
}

fn render_default_live() -> live::Live {
    live::Live::default()
}

impl Src {
    /// Render settings composed from the OBS properties + the app settings.
    /// `obs_all` ("Configure everything in OBS") do not look at the app settings.
    fn merge_vis(&self, cfg: &config::OverlayCfg) -> render::Vis {
        let v = self.vis.lock().map(|v| v.clone()).unwrap_or_default();
        if v.obs_all {
            v
        } else {
            v.with_cfg(cfg)
        }
    }

    fn cur_vis(&self) -> render::Vis {
        let (cfg, _) = config::current();
        self.merge_vis(&cfg)
    }
}

/// OBS settings string to a Rust String (UTF-8, up to the first max bytes).
unsafe fn cstr_text(p: *const u8, max: usize) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut buf = Vec::new();
    let mut e = p;
    while *e != 0 && buf.len() < max {
        buf.push(*e);
        e = e.add(1);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Look up the translation in the OBS plugin locale (`data/locale/<lang>.ini`).
/// Returns the fallback (English) if not loaded, the key is missing, or OBS is old.
/// The returned pointer is in OBS's lookup table, static, or the argument's CStr - all of which
/// stay alive while this expression is evaluated, and OBS copies desc and keeps its own, so it is safe.
/// (the fallback passed to get_name/description must be a 'static CStr)
unsafe fn lt(key: &std::ffi::CStr, fallback: &std::ffi::CStr) -> *const u8 {
    let mut out = fallback.as_ptr().cast();
    obs_module_get_string(key.as_ptr().cast(), &mut out);
    out
}

/// Pack/unpack OBS color data (int, 8 bits each of R,G,B,A from the low end).
fn pack_rgb(r: i64, g: i64, b: i64) -> i64 {
    r | (g << 8) | (b << 16) | (255i64 << 24)
}
fn unpack_rgb(v: i64) -> [u8; 4] {
    [
        (v & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        255,
    ]
}

/// Graphics thread - drop the texture (created lazily on the create side).
unsafe extern "C" fn deactivate(d: *mut c_void) {
    let s = &mut *(d as *mut Src);
    if !s.tex.is_null() {
        if let Some(api) = ffi::resolve() {
            (api.tex_destroy)(s.tex);
        }
        s.tex = core::ptr::null_mut();
    }
}

// ---- Properties (settings UI) ----

/// Defaults: title/labels ON, accent = app settings. `obs_cfg` is OFF - so existing scenes'
/// look does not change. The detailed settings match OverlayCfg's defaults (usable as is when turned ON).
unsafe extern "C" fn get_defaults(settings: *mut c_void) {
    let Some(api) = ffi::resolve() else { return };
    let dbool = |n: &std::ffi::CStr, v: bool| {
        (api.data_set_default_bool)(settings, n.as_ptr() as *const u8, v)
    };
    let dstr = |n: &std::ffi::CStr, v: &std::ffi::CStr| {
        (api.data_set_default_string)(settings, n.as_ptr() as *const u8, v.as_ptr() as *const u8)
    };
    dbool(c"title", true);
    dbool(c"labels", true);
    dstr(c"accent", c"app");
    dbool(c"obs_cfg", false);
    // ---- Defaults for "Configure everything in OBS" (same as OverlayCfg::default) ----
    dstr(c"theme", c"glass");
    dbool(c"numbers", false);
    dstr(c"mstyle", c"both");
    dstr(c"arrange", c"side");
    dstr(c"fill", c"bottom");
    dbool(c"num_pct", false);
    dbool(c"badges", true);
    dbool(c"tline", true);
    dbool(c"shadow", true);
    (api.data_set_default_double)(settings, c"oscale".as_ptr() as *const u8, 1.0);
    (api.data_set_default_double)(settings, c"ksize".as_ptr() as *const u8, 1.0);
    (api.data_set_default_double)(settings, c"radius".as_ptr() as *const u8, 1.0);
    (api.data_set_default_double)(settings, c"opad".as_ptr() as *const u8, 1.0);
    (api.data_set_default_int)(settings, c"palpha".as_ptr() as *const u8, 100);
    (api.data_set_default_int)(settings, c"glow".as_ptr() as *const u8, 100);
    dstr(c"ttext", c"");
    dbool(c"pcust", false);
    (api.data_set_default_int)(
        settings,
        c"acolor".as_ptr() as *const u8,
        pack_rgb(60, 190, 255),
    );
    (api.data_set_default_int)(
        settings,
        c"pcolor".as_ptr() as *const u8,
        pack_rgb(255, 150, 60),
    );
}

/// Load settings. Indices whose `sk{i}`/`sb{i}` is true go into the shown set
/// (all false = show all; if any is true, show only those).
unsafe extern "C" fn update(d: *mut c_void, settings: *mut c_void) {
    let Some(api) = ffi::resolve() else { return };
    let s = &mut *(d as *mut Src);
    let getb = |n: &std::ffi::CStr| (api.data_get_bool)(settings, n.as_ptr() as *const u8);
    let mut vis = render::Vis {
        title: getb(c"title"),
        labels: getb(c"labels"),
        obs_all: getb(c"obs_cfg"),
        ..render::Vis::default()
    };
    // When "Configure everything in OBS" is ON, read all the detailed items
    // (when OFF, discard the values and cur_vis uses the app settings).
    if vis.obs_all {
        use config::{Arrange, FillMode, MouseStyle, Theme};
        let gets = |n: &std::ffi::CStr, m| {
            cstr_text((api.data_get_string)(settings, n.as_ptr() as *const u8), m)
        };
        vis.theme = match gets(c"theme", 8).as_str() {
            "light" => Theme::Light,
            "clear" => Theme::Clear,
            "neon" => Theme::Neon,
            _ => Theme::Glass,
        };
        vis.numbers = getb(c"numbers");
        vis.mouse_style = match gets(c"mstyle", 8).as_str() {
            "shape" => MouseStyle::Shape,
            "bars" => MouseStyle::Bars,
            _ => MouseStyle::Both,
        };
        vis.arrange = match gets(c"arrange", 8).as_str() {
            "stack" => Arrange::Stack,
            "swap" => Arrange::Swap,
            _ => Arrange::Side,
        };
        vis.fill_mode = match gets(c"fill", 8).as_str() {
            "top" => FillMode::Top,
            "fade" => FillMode::Fade,
            _ => FillMode::Bottom,
        };
        vis.num_pct = getb(c"num_pct");
        vis.badges = getb(c"badges");
        vis.trigger_line = getb(c"tline");
        vis.shadow = getb(c"shadow");
        vis.title_text = gets(c"ttext", 64);
        if getb(c"pcust") {
            vis.pressed = Some(unpack_rgb((api.data_get_int)(
                settings,
                c"pcolor".as_ptr() as *const u8,
            )));
        }
        vis.scale = (api.data_get_double)(settings, c"oscale".as_ptr() as *const u8)
            .clamp(0.75, 3.0) as f32;
        vis.key_size =
            (api.data_get_double)(settings, c"ksize".as_ptr() as *const u8).clamp(0.7, 1.4) as f32;
        vis.radius =
            (api.data_get_double)(settings, c"radius".as_ptr() as *const u8).clamp(0., 2.) as f32;
        vis.pad =
            (api.data_get_double)(settings, c"opad".as_ptr() as *const u8).clamp(0.5, 2.) as f32;
        vis.panel_alpha = ((api.data_get_int)(settings, c"palpha".as_ptr() as *const u8)
            .clamp(0, 100) as f32)
            / 100.;
        vis.glow = ((api.data_get_int)(settings, c"glow".as_ptr() as *const u8).clamp(0, 200)
            as f32)
            / 100.;
    }
    let mut show_kb = HashSet::new();
    for i in 0..160 {
        let n = cs(&format!("sk{i}"));
        if (api.data_get_bool)(settings, n.as_ptr() as *const u8) {
            show_kb.insert(i);
        }
    }
    let mut show_btn = HashSet::new();
    for i in 0..24 {
        let n = cs(&format!("sb{i}"));
        if (api.data_get_bool)(settings, n.as_ptr() as *const u8) {
            show_btn.insert(i);
        }
    }
    vis.show_kb = show_kb;
    vis.show_btn = show_btn;
    let acc = cstr_text(
        (api.data_get_string)(settings, c"accent".as_ptr() as *const u8),
        16,
    );
    if acc == "custom" {
        vis.accent = unpack_rgb((api.data_get_int)(
            settings,
            c"acolor".as_ptr() as *const u8,
        ));
        vis.accent_set = true;
    } else if let Ok(i) = acc.parse::<usize>() {
        if let Some((_, c)) = render::ACCENTS.get(i) {
            vis.accent = *c;
            vis.accent_set = true;
        }
    }
    *s.vis.lock().unwrap_or_else(|p| p.into_inner()) = vis;
}

/// Properties UI. Keys/buttons are generated from the connected device's layout
/// (when no keyboard is connected, the known SU66 layout is used).
unsafe extern "C" fn get_properties(d: *mut c_void) -> *mut c_void {
    let Some(api) = ffi::resolve() else {
        return core::ptr::null_mut();
    };
    let s = &mut *(d as *mut Src);
    let snap = s.live_snap();
    let props = (api.props_create)();
    // Descriptions come from the OBS locale (untranslated ones fall back to English). The pointers
    // are owned by OBS or static, so the result of lt() can be passed as is.
    let addb = |p: *mut c_void, name: &str, desc: *const u8| {
        let n = cs(name);
        (api.props_add_bool)(p, n.as_ptr() as *const u8, desc);
    };
    addb(props, "title", lt(c"devhub.title", c"Show title"));
    addb(
        props,
        "labels",
        lt(c"devhub.labels", c"Show key/button names"),
    );
    // Accent color.
    {
        let n = cs("accent");
        let l = (api.props_add_list)(
            props,
            n.as_ptr() as *const u8,
            lt(c"devhub.accent", c"Accent color"),
            ffi::OBS_COMBO_LIST,
            ffi::OBS_COMBO_FMT_STR,
        );
        // Default: the color chosen on the "OBS overlay" screen of the Device HUB app.
        (api.prop_list_add_str)(
            l,
            lt(c"devhub.accent.app", c"Follow Device HUB app"),
            c"app".as_ptr() as *const u8,
        );
        for (i, (_, _)) in render::ACCENTS.iter().enumerate() {
            let (kk, fb): (CString, CString) =
                (cs(&format!("devhub.acc{i}")), cs(render::ACCENTS[i].0));
            (api.prop_list_add_str)(l, lt(&kk, &fb), cs(&i.to_string()).as_ptr() as *const u8);
        }
        (api.prop_list_add_str)(
            l,
            lt(c"devhub.accent.custom", c"Custom"),
            c"custom".as_ptr() as *const u8,
        );
        let n2 = cs("acolor");
        (api.props_add_color)(
            props,
            n2.as_ptr() as *const u8,
            lt(c"devhub.acolor", c"Custom accent color"),
        );
    }
    // ---- Mode where all settings are done in OBS only (no Device HUB app needed; for OSS distribution) ----
    addb(
        props,
        "obs_cfg",
        lt(
            c"devhub.obsall",
            c"Configure everything in OBS (ignore app settings)",
        ),
    );
    {
        let g = (api.props_create)();
        let addl =
            |name: &str, desc: *const u8, opts: &[(&std::ffi::CStr, &std::ffi::CStr, &str)]| {
                let n = cs(name);
                let l = (api.props_add_list)(
                    g,
                    n.as_ptr() as *const u8,
                    desc,
                    ffi::OBS_COMBO_LIST,
                    ffi::OBS_COMBO_FMT_STR,
                );
                for (key, fb, val) in opts {
                    (api.prop_list_add_str)(l, lt(key, fb), cs(val).as_ptr() as *const u8);
                }
            };
        let addgb = |name: &str, desc: *const u8| {
            let n = cs(name);
            (api.props_add_bool)(g, n.as_ptr() as *const u8, desc);
        };
        let addfs = |name: &str, desc: *const u8, min: f64, max: f64, step: f64| {
            let n = cs(name);
            (api.props_add_float_slider)(g, n.as_ptr() as *const u8, desc, min, max, step);
        };
        let addis = |name: &str, desc: *const u8, min: i32, max: i32, step: i32| {
            let n = cs(name);
            (api.props_add_int_slider)(g, n.as_ptr() as *const u8, desc, min, max, step);
        };
        addl(
            "theme",
            lt(c"devhub.theme", c"Theme"),
            &[
                (c"devhub.theme.glass", c"Glass", "glass"),
                (c"devhub.theme.light", c"Light", "light"),
                (c"devhub.theme.clear", c"Clear", "clear"),
                (c"devhub.theme.neon", c"Neon", "neon"),
            ],
        );
        if s.mode != render::Mode::Kb {
            addl(
                "mstyle",
                lt(c"devhub.mstyle", c"Mouse style"),
                &[
                    (c"devhub.mstyle.both", c"Shape + bars", "both"),
                    (c"devhub.mstyle.shape", c"Shape only", "shape"),
                    (c"devhub.mstyle.bars", c"Bars only", "bars"),
                ],
            );
        }
        if s.mode == render::Mode::Both {
            addl(
                "arrange",
                lt(c"devhub.arrange", c"Arrange"),
                &[
                    (c"devhub.arrange.side", c"Side by side", "side"),
                    (c"devhub.arrange.stack", c"Stacked", "stack"),
                    (c"devhub.arrange.swap", c"Swapped", "swap"),
                ],
            );
        }
        addl(
            "fill",
            lt(c"devhub.fill", c"Fill direction"),
            &[
                (c"devhub.fill.bottom", c"Bottom-up", "bottom"),
                (c"devhub.fill.top", c"Top-down", "top"),
                (c"devhub.fill.fade", c"Fade", "fade"),
            ],
        );
        addgb("numbers", lt(c"devhub.numbers", c"Show depth values"));
        addgb("num_pct", lt(c"devhub.num_pct", c"Show values as %"));
        addgb("badges", lt(c"devhub.badges", c"Show AP/RT badges"));
        addgb("tline", lt(c"devhub.tline", c"Show trigger line"));
        addgb("shadow", lt(c"devhub.shadow", c"Show shadows"));
        addgb("pcust", lt(c"devhub.pcust", c"Custom pressed color"));
        {
            let n = cs("pcolor");
            (api.props_add_color)(
                g,
                n.as_ptr() as *const u8,
                lt(c"devhub.pcolor", c"Pressed color"),
            );
        }
        addfs(
            "oscale",
            lt(c"devhub.oscale", c"Render scale"),
            0.75,
            3.0,
            0.25,
        );
        addfs("ksize", lt(c"devhub.ksize", c"Key size"), 0.7, 1.4, 0.05);
        addfs(
            "radius",
            lt(c"devhub.radius", c"Corner radius"),
            0.0,
            2.0,
            0.1,
        );
        addfs("opad", lt(c"devhub.opad", c"Panel padding"), 0.5, 2.0, 0.1);
        addis(
            "palpha",
            lt(c"devhub.palpha", c"Panel opacity %"),
            0,
            100,
            5,
        );
        addis("glow", lt(c"devhub.glow", c"Press glow %"), 0, 200, 10);
        {
            let n = cs("ttext");
            (api.props_add_text)(
                g,
                n.as_ptr() as *const u8,
                lt(c"devhub.ttext", c"Title text (empty = device name)"),
                0,
            );
        }
        let n = cs("detail");
        (api.props_add_group)(
            props,
            n.as_ptr() as *const u8,
            lt(
                c"devhub.detail",
                c"Appearance (enabled by the toggle above)",
            ),
            ffi::OBS_GROUP_NORMAL,
            g,
        );
    }
    if s.mode != render::Mode::Mouse {
        // Per-key display selection (only checked keys are shown; none checked = show all).
        let fallback = devhub_core::registry::su66max_layout();
        let layout: &[devhub_core::model::LayoutKey] = snap
            .kb
            .as_ref()
            .map(|k| k.layout.as_slice())
            .unwrap_or(&fallback);
        let g = (api.props_create)();
        for (i, k) in layout.iter().enumerate() {
            let n = cs(&format!("sk{i}"));
            let dd = cs(&k.label);
            (api.props_add_bool)(g, n.as_ptr() as *const u8, dd.as_ptr() as *const u8);
        }
        let n = cs("keys");
        (api.props_add_group)(
            props,
            n.as_ptr() as *const u8,
            lt(c"devhub.keys", c"Keys to show (unchecked = all)"),
            ffi::OBS_GROUP_NORMAL,
            g,
        );
    }
    if s.mode != render::Mode::Kb {
        match &snap.mouse {
            Some(m) if !m.names.is_empty() => {
                let g = (api.props_create)();
                for (i, name) in m.names.iter().enumerate() {
                    let (n, dd) = (cs(&format!("sb{i}")), cs(name));
                    (api.props_add_bool)(g, n.as_ptr() as *const u8, dd.as_ptr() as *const u8);
                }
                let n = cs("btns");
                (api.props_add_group)(
                    props,
                    n.as_ptr() as *const u8,
                    lt(c"devhub.btns", c"Buttons to show (unchecked = all)"),
                    ffi::OBS_GROUP_NORMAL,
                    g,
                );
            }
            _ => {
                let n = cs("btn_note");
                (api.props_add_text)(
                    props,
                    n.as_ptr() as *const u8,
                    lt(
                        c"devhub.btn.note",
                        c"No mouse connected — buttons appear once connected",
                    ),
                    ffi::OBS_TEXT_INFO,
                );
            }
        }
    }
    props
}

unsafe extern "C" fn video_render(d: *mut c_void, _e: *mut c_void) {
    let Some(api) = ffi::resolve() else {
        logline("video_render: resolve() failed");
        return;
    };
    let s = &mut *(d as *mut Src);
    update_live_pixels(s, api);
    draw_texture(s, api);
}

unsafe fn update_live_pixels(s: &mut Src, api: &ffi::Api) {
    // If there is a new data generation, keep it as the interpolation target.
    let (cfg, cfg_gen) = config::current();
    let cfg_changed = cfg_gen != s.last_cfg;
    let new_data = {
        match s.pool.as_ref().map(|p| p.live().lock()) {
            Some(Ok(l)) => {
                if l.seq != s.last_seq {
                    s.live = l.clone();
                    s.last_seq = l.seq;
                    true
                } else {
                    false
                }
            }
            Some(Err(_)) | None => {
                s.last_seq = u64::MAX;
                false
            }
        }
    };
    // Advance the interpolation every frame and re-render if it is moving - polling (~15-25Hz)
    // is slower than rendering (~60fps), so this removes the slideshow effect.
    let now = std::time::Instant::now();
    let dt = now
        .duration_since(s.last_tick)
        .as_secs_f32()
        .clamp(0.0, 0.1);
    s.last_tick = now;
    let moving = s.advance(dt);
    if new_data || cfg_changed || moving || s.pix.is_none() {
        let vis = s.merge_vis(&cfg);
        let disp = s.smoothed_live();
        let pix = render::draw(&disp, s.font.as_ref(), s.mode, &vis);
        // Recreate the texture if the size changed.
        if s.pix.as_ref().is_some_and(|o| o.w != pix.w || o.h != pix.h) && !s.tex.is_null() {
            (api.tex_destroy)(s.tex);
            s.tex = core::ptr::null_mut();
        }
        s.pix = Some(pix);
        s.last_cfg = cfg_gen;
        s.dirty = true;
    }
}

unsafe fn draw_texture(s: &mut Src, api: &ffi::Api) {
    let Some(pix) = &s.pix else {
        s.first_render.get_or_insert_with(|| {
            logline("video_render: pix empty");
            true
        });
        return;
    };
    if s.tex.is_null() {
        s.tex = (api.tex_create)(
            pix.w,
            pix.h,
            ffi::GS_RGBA,
            1,
            core::ptr::null(),
            ffi::GS_DYNAMIC,
        );
        if s.tex.is_null() {
            logline("video_render: tex_create failed");
            return;
        }
        logline("video_render: tex created");
        s.dirty = true;
    }
    if s.dirty {
        (api.tex_set_image)(s.tex, pix.buf.as_ptr(), pix.w * 4, false);
        s.dirty = false;
    }
    (api.blend_push)();
    (api.blend_function)(ffi::GS_BLEND_SRCALPHA, ffi::GS_BLEND_INVSRCALPHA);
    let effect = (api.base_effect)(ffi::OBS_EFFECT_DEFAULT);
    // Canonical libobs pattern: obs_source_draw inside the pass of gs_effect_loop.
    if let (Some(eff_loop), Some(draw)) = (api.effect_loop, api.source_draw) {
        if !effect.is_null() {
            let mut first = true;
            while eff_loop(effect, c"Draw".as_ptr() as *const u8) {
                draw(s.tex, 0, 0, 0, 0, false);
                if first {
                    first = false;
                    if s.first_render.is_none() {
                        s.first_render = Some(true);
                        logline("video_render: effect_loop+source_draw used");
                    }
                }
            }
        }
        (api.blend_pop)();
        return;
    }
    if !effect.is_null() {
        let tech = (api.effect_technique)(effect, c"Draw".as_ptr() as *const u8);
        let param = (api.effect_param)(effect, c"image".as_ptr() as *const u8);
        if s.first_render.is_none() {
            logline(&format!(
                "video_render: effect={effect:?} tech={tech:?} param={param:?} tex={:?}",
                s.tex
            ));
        }
        if tech.is_null() || param.is_null() {
            (api.blend_pop)();
            return;
        }
        (api.effect_set_texture)(param, s.tex);
        let passes = (api.technique_begin)(tech);
        for p in 0..passes {
            if (api.technique_begin_pass)(tech, p) {
                (api.draw_sprite)(s.tex, 0, pix.w, pix.h);
                (api.technique_end_pass)(tech);
            }
        }
        (api.technique_end)(tech);
        if s.first_render.is_none() {
            s.first_render = Some(true);
            logline(&format!(
                "video_render: drew {}x{} passes={passes}",
                pix.w, pix.h
            ));
        }
    } else {
        s.first_render.get_or_insert_with(|| {
            logline("video_render: effect null");
            true
        });
    }
    (api.blend_pop)();
}

unsafe extern "C" fn usb_name(_: *mut c_void) -> *const u8 {
    lt(c"devhub.usb.source", c"Device HUB USB Polling Rate")
}

unsafe extern "C" fn usb_create(settings: *mut c_void, _: *mut c_void) -> *mut c_void {
    let d = Box::into_raw(Box::new(Src::new(render::Mode::Mouse))).cast();
    usb_update(d, settings);
    d
}

unsafe extern "C" fn usb_update(d: *mut c_void, settings: *mut c_void) {
    let Some(api) = ffi::resolve() else { return };
    let get = |n: &std::ffi::CStr| (api.data_get_int)(settings, n.as_ptr().cast());
    let source = &*d.cast::<Src>();
    let mut capture = source.usb.lock().unwrap_or_else(|e| e.into_inner());
    use std::sync::atomic::Ordering;
    source.usb_missing.store(false, Ordering::Relaxed);
    if !(api.data_get_bool)(settings, c"capture".as_ptr().cast()) {
        capture.take();
        return;
    }
    let report = get(c"report");
    let mut hub = get(c"hub").clamp(1, 255) as u16;
    let mut device = get(c"device").clamp(1, 127) as u8;
    // If a product is picked in the "Target device" list, override with its current hub number and
    // USB address (follows replug and port changes too).
    // If not connected, wait for re-selection.
    let pick = {
        let s = (api.data_get_string)(settings, c"pick".as_ptr().cast());
        if s.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(s.cast()) }
                .to_string_lossy()
                .into_owned()
        }
    };
    if !pick.is_empty() {
        if let Some(d) = devhub_polling::discover::devices()
            .into_iter()
            .find(|d| d.hub != 0 && d.instance_id.eq_ignore_ascii_case(&pick))
        {
            hub = d.hub;
            device = d.address;
        } else {
            capture.take();
            source.usb_missing.store(true, Ordering::Relaxed);
            return;
        }
    }
    let target = usb_rate::Target {
        hub,
        device,
        endpoint: 0x80 | get(c"endpoint").clamp(1, 15) as u8,
        report: u8::try_from(report).ok(),
    };
    // A capture that ended in a terminal error is recreated when properties are applied (once the hub
    // is free the user can re-apply to recover). A live capture on the same target is kept.
    let dead = capture
        .as_ref()
        .is_some_and(|c| c.snapshot().status.is_error());
    if dead || capture.as_ref().is_none_or(|c| c.target != target) {
        capture.take(); // Release the exclusive hub handle before reopening it.
        *capture = Some(usb_rate::Capture::start(target));
    }
}

unsafe extern "C" fn usb_defaults(settings: *mut c_void) {
    let Some(api) = ffi::resolve() else { return };
    for (n, value) in [
        (c"hub", 1),
        (c"device", 1),
        (c"endpoint", 1),
        (c"report", -1),
    ] {
        (api.data_set_default_int)(settings, n.as_ptr().cast(), value);
    }
    (api.data_set_default_bool)(settings, c"capture".as_ptr().cast(), false);
}

unsafe extern "C" fn usb_props(_: *mut c_void) -> *mut c_void {
    let Some(api) = ffi::resolve() else {
        return std::ptr::null_mut();
    };
    let p = (api.props_create)();
    // List to pick the target device by product name — the value is the PnP instance ID.
    // If unselected (""), the manually entered values below are used. Mice are listed first.
    let list = (api.props_add_list)(
        p,
        c"pick".as_ptr().cast(),
        lt(c"devhub.usb.pick", c"Target device (product)"),
        ffi::OBS_COMBO_LIST,
        ffi::OBS_COMBO_FMT_STR,
    );
    (api.prop_list_add_str)(
        list,
        lt(c"devhub.usb.pick.none", c"(manual hub/address below)"),
        c"".as_ptr().cast(),
    );
    let mut devs = devhub_polling::discover::devices();
    devs.retain(|d| d.hub != 0);
    devs.sort_by_key(|d| !d.is_mouse);
    for d in &devs {
        let label = format!(
            "{} ({:04x}:{:04x}) USBPcap{} — {}",
            d.name, d.vid, d.pid, d.hub, d.location
        );
        (api.prop_list_add_str)(
            list,
            cs(&label).as_ptr().cast(),
            cs(&d.instance_id).as_ptr().cast(),
        );
    }
    for (n, key, label, min, max) in [
        (c"hub", c"devhub.usb.hub", c"USBPcap hub number", 1, 255),
        (
            c"device",
            c"devhub.usb.device",
            c"USB device address",
            1,
            127,
        ),
        (
            c"endpoint",
            c"devhub.usb.ep",
            c"Mouse IN endpoint number (0x81 = 1)",
            1,
            15,
        ),
        (
            c"report",
            c"devhub.usb.report",
            c"Report ID (-1 = all)",
            -1,
            255,
        ),
    ] {
        (api.props_add_int)(p, n.as_ptr().cast(), lt(key, label), min, max, 1);
    }
    (api.props_add_bool)(
        p,
        c"capture".as_ptr().cast(),
        lt(c"devhub.usb.capture", c"Measure USB input"),
    );
    p
}

unsafe extern "C" fn usb_width(_: *mut c_void) -> u32 {
    render::RATE_WIDTH
}
unsafe extern "C" fn usb_height(_: *mut c_void) -> u32 {
    render::RATE_HEIGHT
}

unsafe extern "C" fn usb_render(d: *mut c_void, _: *mut c_void) {
    let Some(api) = ffi::resolve() else { return };
    let s = &mut *d.cast::<Src>();
    if s.pix.is_none() || s.last_tick.elapsed() >= std::time::Duration::from_millis(250) {
        s.last_tick = std::time::Instant::now();
        let cap = s.usb.lock().unwrap_or_else(|e| e.into_inner());
        let (label, status) = cap.as_ref().map_or_else(
            || {
                (
                    if s.usb_missing.load(std::sync::atomic::Ordering::Relaxed) {
                        "USB device unavailable"
                    } else {
                        "USB capture off"
                    }
                    .to_string(),
                    None,
                )
            },
            |c| {
                let snap = c.snapshot();
                (usb_rate::label(&snap), Some(snap.status))
            },
        );
        // Log the transition to an error state to the diagnostic log (repeats of the same state are not written).
        if status.is_some_and(|st| st.is_error()) && s.usb_status != status {
            logline(&format!(
                "USBPcap{}: {label} ({:?})",
                cap.as_ref().map_or(0, |c| c.target.hub),
                status.unwrap()
            ));
        }
        s.usb_status = status;
        if label != s.usb_label {
            s.pix = Some(render::usb_rate_label(&label));
            s.usb_label = label;
            s.dirty = true;
        }
    }
    draw_texture(s, api);
}

unsafe extern "C" fn usb_destroy(d: *mut c_void) {
    let mut s = Box::from_raw(d.cast::<Src>());
    if let Some(api) = ffi::resolve() {
        if !s.tex.is_null() {
            (api.enter_graphics)();
            (api.tex_destroy)(s.tex);
            (api.leave_graphics)();
            s.tex = std::ptr::null_mut();
        }
    }
}

fn usb_source_info() -> ffi::ObsSourceInfo {
    ffi::ObsSourceInfo {
        destroy: Some(usb_destroy),
        get_width: Some(usb_width),
        get_height: Some(usb_height),
        get_defaults: Some(usb_defaults),
        get_properties: Some(usb_props),
        update: Some(usb_update),
        video_render: Some(usb_render),
        deactivate: None,
        ..source_info(c"devhub_usb_rate", usb_name, usb_create)
    }
}

/// `obs_register_source_s` copies the struct, so building it locally is fine
/// (it holds raw pointers, so it can't be a static).
fn source_info(
    id: &'static std::ffi::CStr,
    get_name: unsafe extern "C" fn(*mut c_void) -> *const u8,
    create: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
) -> ffi::ObsSourceInfo {
    ffi::ObsSourceInfo {
        id: id.as_ptr() as *const u8,
        typ: ffi::OBS_SOURCE_TYPE_INPUT,
        output_flags: ffi::OBS_SOURCE_VIDEO | ffi::OBS_SOURCE_CUSTOM_DRAW,
        get_name: Some(get_name),
        create: Some(create),
        destroy: Some(destroy),
        get_width: Some(get_width),
        get_height: Some(get_height),
        get_defaults: Some(get_defaults),
        get_properties: Some(get_properties),
        update: Some(update),
        activate: None,
        deactivate: Some(deactivate),
        show: None,
        hide: None,
        video_tick: None,
        video_render: Some(video_render),
    }
}

// ---------------- Module entry ----------------

#[no_mangle]
pub extern "C" fn obs_module_set_pointer(m: *mut c_void) {
    MODULE.store(m, std::sync::atomic::Ordering::Relaxed);
}

#[no_mangle]
pub extern "C" fn obs_current_module() -> *mut c_void {
    MODULE.load(std::sync::atomic::Ordering::Relaxed)
}

static LOOKUP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// OBS export: looks up a string in the loaded locale.
///
/// # Safety
/// `key` must be a NUL-terminated string and `out` a writable pointer, as OBS passes them.
#[no_mangle]
pub unsafe extern "C" fn obs_module_get_string(key: *const u8, out: *mut *const u8) -> bool {
    let lookup = LOOKUP.load(std::sync::atomic::Ordering::Acquire);
    !lookup.is_null() && ffi::resolve().is_some_and(|api| (api.lookup_string)(lookup, key, out))
}

/// OBS export: loads the locale files for `locale`.
///
/// # Safety
/// `locale` must be a NUL-terminated string, as OBS passes it.
#[no_mangle]
pub unsafe extern "C" fn obs_module_set_locale(locale: *const u8) {
    if let Some(api) = ffi::resolve() {
        let next = (api.load_locale)(obs_current_module(), c"en-US".as_ptr().cast(), locale);
        let old = LOOKUP.swap(next, std::sync::atomic::Ordering::AcqRel);
        if !old.is_null() {
            (api.free_lookup)(old);
        }
    }
}

/// OBS export: frees the loaded locale.
///
/// # Safety
/// Called by OBS while unloading; no `obs_module_get_string` call may run at the same time.
#[no_mangle]
pub unsafe extern "C" fn obs_module_free_locale() {
    let old = LOOKUP.swap(std::ptr::null_mut(), std::sync::atomic::Ordering::AcqRel);
    if let Some(api) = ffi::resolve() {
        if !old.is_null() {
            (api.free_lookup)(old);
        }
    }
}

#[no_mangle]
pub extern "C" fn obs_module_ver() -> u32 {
    match ffi::resolve() {
        Some(api) => ffi::host_semver(api),
        None => 31 << 24,
    }
}

#[no_mangle]
pub extern "C" fn obs_module_name() -> *const u8 {
    c"devhub-obs".as_ptr() as *const u8
}

#[no_mangle]
pub extern "C" fn obs_module_description() -> *const u8 {
    unsafe {
        lt(
            c"devhub.desc",
            c"Device HUB: live magnetic key/button depth display",
        )
    }
}

#[no_mangle]
pub extern "C" fn obs_module_author() -> *const u8 {
    c"Device HUB".as_ptr() as *const u8
}

#[no_mangle]
pub extern "C" fn obs_module_load() -> bool {
    let Some(api) = ffi::resolve() else {
        return false;
    };
    let infos = [
        usb_source_info(),
        source_info(c"devhub_live", name_both, create_both),
        source_info(c"devhub_live_kb", name_kb, create_kb),
        source_info(c"devhub_live_mouse", name_mouse, create_mouse),
    ];
    unsafe {
        for info in &infos {
            (api.register_source_s)(info, core::mem::size_of::<ffi::ObsSourceInfo>());
        }
    }
    true
}

#[no_mangle]
pub extern "C" fn obs_module_unload() {
    // Stop the shared poller if one remains (normally already stopped once all sources are destroyed).
    live::shutdown();
}
