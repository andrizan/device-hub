//! Resolve the obs.dll functions at runtime (builds without obs.lib).
//!
//! `GetModuleHandleA("obs")` gets the host's libobs, and `GetProcAddress` picks up
//! only the symbols it needs. Symbols that are not found stay as Option.

#![allow(non_snake_case)]

use core::ffi::c_void;

type CStr8 = *const u8;

#[cfg(windows)]
extern "C" {
    fn GetModuleHandleA(name: CStr8) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: CStr8) -> *mut c_void;
}

// The plugin loader is Windows-only for now. On other systems nothing
// resolves, so `resolve` returns None and the plugin stays inert; the crate
// still builds, which the app needs for the overlay renderer.
#[cfg(not(windows))]
unsafe fn GetModuleHandleA(_name: CStr8) -> *mut c_void {
    core::ptr::null_mut()
}
#[cfg(not(windows))]
unsafe fn GetProcAddress(_m: *mut c_void, _name: CStr8) -> *mut c_void {
    core::ptr::null_mut()
}

fn sym(m: *mut c_void, name: &'static [u8]) -> *mut c_void {
    unsafe { GetProcAddress(m, name.as_ptr()) }
}

/// The leading part of `obs_source_info` (the ABI is fixed in this order).
/// `obs_register_source_s` is passed `size_of`, so trailing fields
/// are treated as "absent in older versions" - this is the official forward-compatibility mechanism.
#[repr(C)]
pub struct ObsSourceInfo {
    pub id: *const u8,
    pub typ: u32,
    pub output_flags: u32,
    pub get_name: Option<unsafe extern "C" fn(*mut c_void) -> *const u8>,
    pub create: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void>,
    pub destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    pub get_width: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
    pub get_height: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
    pub get_defaults: Option<unsafe extern "C" fn(*mut c_void)>,
    pub get_properties: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    pub update: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
    pub activate: Option<unsafe extern "C" fn(*mut c_void)>,
    pub deactivate: Option<unsafe extern "C" fn(*mut c_void)>,
    pub show: Option<unsafe extern "C" fn(*mut c_void)>,
    pub hide: Option<unsafe extern "C" fn(*mut c_void)>,
    pub video_tick: Option<unsafe extern "C" fn(*mut c_void, f32)>,
    pub video_render: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
}

pub const OBS_SOURCE_TYPE_INPUT: u32 = 0;
pub const OBS_SOURCE_VIDEO: u32 = 1;
/// obs.h: OBS_SOURCE_CUSTOM_DRAW = (1<<3). 1<<7 is DO_NOT_DUPLICATE, a different flag -
/// without this flag OBS calls video_render from inside the pass of the default "Draw" technique,
/// so our technique_begin gets nested and the drawing is discarded.
pub const OBS_SOURCE_CUSTOM_DRAW: u32 = 1 << 3;

/// gs_color_format: GS_RGBA (3rd in the enum).
pub const GS_RGBA: u32 = 3;
/// gs_texture_flags: GS_DYNAMIC = (1<<1). 1<<0 is GS_BUILD_MIPMAPS.
pub const GS_DYNAMIC: u32 = 1 << 1;
/// gs_blend_type.
pub const GS_BLEND_SRCALPHA: u32 = 4;
pub const GS_BLEND_INVSRCALPHA: u32 = 5;
/// obs_base_effect: OBS_EFFECT_DEFAULT ("Draw" technique + "image" parameter).
pub const OBS_EFFECT_DEFAULT: i32 = 0;
/// obs_combo_type: OBS_COMBO_TYPE_LIST.
pub const OBS_COMBO_LIST: i32 = 2;
/// obs_combo_format: OBS_COMBO_FORMAT_STRING.
pub const OBS_COMBO_FMT_STR: i32 = 3;
/// obs_group_type: OBS_GROUP_NORMAL.
pub const OBS_GROUP_NORMAL: i32 = 0;
/// obs_text_type: OBS_TEXT_INFO.
pub const OBS_TEXT_INFO: i32 = 0;

/// Resolved API table. video_render is called from the graphics thread.
pub struct Api {
    pub register_source_s: unsafe extern "C" fn(*const ObsSourceInfo, usize),
    pub version_string: Option<unsafe extern "C" fn() -> *const u8>,
    pub base_effect: unsafe extern "C" fn(i32) -> *mut c_void,
    pub effect_technique: unsafe extern "C" fn(*mut c_void, *const u8) -> *mut c_void,
    pub effect_param: unsafe extern "C" fn(*mut c_void, *const u8) -> *mut c_void,
    pub effect_set_texture: unsafe extern "C" fn(*mut c_void, *mut c_void),
    pub technique_begin: unsafe extern "C" fn(*mut c_void) -> usize,
    pub technique_begin_pass: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    pub technique_end_pass: unsafe extern "C" fn(*mut c_void),
    pub technique_end: unsafe extern "C" fn(*mut c_void),
    pub tex_create: TexCreate,
    pub tex_set_image: unsafe extern "C" fn(*mut c_void, *const u8, u32, bool),
    pub tex_destroy: unsafe extern "C" fn(*mut c_void),
    pub draw_sprite: unsafe extern "C" fn(*mut c_void, u32, u32, u32),
    /// `obs_source_draw(image, x, y, cx, cy, flip)` - an official helper that
    /// has OBS handle the effect/technique correctly.
    pub source_draw: Option<unsafe extern "C" fn(*mut c_void, i32, i32, u32, u32, bool)>,
    /// For diagnostics (used to isolate whether gs_clear can color-clear the whole RT).
    #[allow(dead_code)]
    pub clear: Option<unsafe extern "C" fn(u32, *const f32, f32, u8)>,
    /// `gs_effect_loop(effect, "Draw")` - the canonical loop that wraps
    /// technique begin/pass. obs_source_draw must be called inside it.
    pub effect_loop: Option<unsafe extern "C" fn(*mut c_void, *const u8) -> bool>,
    pub blend_push: unsafe extern "C" fn(),
    pub blend_pop: unsafe extern "C" fn(),
    pub blend_function: unsafe extern "C" fn(u32, u32),
    // ---- Properties / obs_data (source settings UI) ----
    pub props_create: unsafe extern "C" fn() -> *mut c_void,
    pub props_add_bool: unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> *mut c_void,
    pub props_add_list:
        unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32, i32) -> *mut c_void,
    pub props_add_text: unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32) -> *mut c_void,
    pub enter_graphics: unsafe extern "C" fn(),
    pub leave_graphics: unsafe extern "C" fn(),
    pub props_add_int: unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32, i32, i32) -> *mut c_void,
    pub props_add_int_slider:
        unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32, i32, i32) -> *mut c_void,
    pub props_add_float_slider:
        unsafe extern "C" fn(*mut c_void, *const u8, *const u8, f64, f64, f64) -> *mut c_void,
    pub props_add_color: unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> *mut c_void,
    pub props_add_group:
        unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32, *mut c_void) -> *mut c_void,
    pub prop_list_add_str: unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> usize,
    /// `obs_module_get_locale_string(module, key)` — look up the translation from the OBS locale (en-US default +
    /// overridden by the UI-language ini). NULL when unregistered/not found.
    pub load_locale: unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> *mut c_void,
    pub lookup_string: unsafe extern "C" fn(*mut c_void, *const u8, *mut *const u8) -> bool,
    pub free_lookup: unsafe extern "C" fn(*mut c_void),
    pub data_get_bool: unsafe extern "C" fn(*mut c_void, *const u8) -> bool,
    pub data_get_int: unsafe extern "C" fn(*mut c_void, *const u8) -> i64,
    pub data_get_double: unsafe extern "C" fn(*mut c_void, *const u8) -> f64,
    pub data_get_string: unsafe extern "C" fn(*mut c_void, *const u8) -> *const u8,
    pub data_set_default_bool: unsafe extern "C" fn(*mut c_void, *const u8, bool),
    pub data_set_default_int: unsafe extern "C" fn(*mut c_void, *const u8, i64),
    pub data_set_default_double: unsafe extern "C" fn(*mut c_void, *const u8, f64),
    pub data_set_default_string: unsafe extern "C" fn(*mut c_void, *const u8, *const u8),
}

/// gs_texture_create signature.
pub type TexCreate = unsafe extern "C" fn(u32, u32, u32, u32, *const *const u8, u32) -> *mut c_void;

static API: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();

/// Resolve the API from the host's obs.dll. None if a required symbol is missing.
/// Call this only when loaded inside OBS (assumes obs.dll is present).
pub fn resolve() -> Option<&'static Api> {
    if let Some(a) = API.get() {
        return a.as_ref();
    }
    unsafe {
        // The host module name is normally "obs" (bin/64bit/obs.dll).
        let m = [c"obs", c"obs64", c"libobs"]
            .iter()
            .map(|n| GetModuleHandleA(n.as_ptr() as *const u8))
            .find(|h| !h.is_null())?;
        macro_rules! need {
            ($n:literal, $t:ty) => {{
                let p = sym(m, concat!($n, "\0").as_bytes());
                if p.is_null() {
                    return None;
                }
                core::mem::transmute::<*mut c_void, $t>(p)
            }};
        }
        macro_rules! want {
            ($n:literal, $t:ty) => {{
                let p = sym(m, concat!($n, "\0").as_bytes());
                if p.is_null() {
                    None
                } else {
                    Some(core::mem::transmute::<*mut c_void, $t>(p))
                }
            }};
        }
        let api = Api {
            register_source_s: need!(
                "obs_register_source_s",
                unsafe extern "C" fn(*const ObsSourceInfo, usize)
            ),
            version_string: want!(
                "obs_get_version_string",
                unsafe extern "C" fn() -> *const u8
            ),
            base_effect: need!(
                "obs_get_base_effect",
                unsafe extern "C" fn(i32) -> *mut c_void
            ),
            effect_technique: need!(
                "gs_effect_get_technique",
                unsafe extern "C" fn(*mut c_void, *const u8) -> *mut c_void
            ),
            effect_param: need!(
                "gs_effect_get_param_by_name",
                unsafe extern "C" fn(*mut c_void, *const u8) -> *mut c_void
            ),
            effect_set_texture: need!(
                "gs_effect_set_texture",
                unsafe extern "C" fn(*mut c_void, *mut c_void)
            ),
            technique_begin: need!(
                "gs_technique_begin",
                unsafe extern "C" fn(*mut c_void) -> usize
            ),
            technique_begin_pass: need!(
                "gs_technique_begin_pass",
                unsafe extern "C" fn(*mut c_void, usize) -> bool
            ),
            technique_end_pass: need!("gs_technique_end_pass", unsafe extern "C" fn(*mut c_void)),
            technique_end: need!("gs_technique_end", unsafe extern "C" fn(*mut c_void)),
            tex_create: need!("gs_texture_create", TexCreate),
            tex_set_image: need!(
                "gs_texture_set_image",
                unsafe extern "C" fn(*mut c_void, *const u8, u32, bool)
            ),
            tex_destroy: need!("gs_texture_destroy", unsafe extern "C" fn(*mut c_void)),
            draw_sprite: need!(
                "gs_draw_sprite",
                unsafe extern "C" fn(*mut c_void, u32, u32, u32)
            ),
            source_draw: want!(
                "obs_source_draw",
                unsafe extern "C" fn(*mut c_void, i32, i32, u32, u32, bool)
            ),
            clear: want!("gs_clear", unsafe extern "C" fn(u32, *const f32, f32, u8)),
            effect_loop: want!(
                "gs_effect_loop",
                unsafe extern "C" fn(*mut c_void, *const u8) -> bool
            ),
            blend_push: need!("gs_blend_state_push", unsafe extern "C" fn()),
            blend_pop: need!("gs_blend_state_pop", unsafe extern "C" fn()),
            blend_function: need!("gs_blend_function", unsafe extern "C" fn(u32, u32)),
            props_create: need!(
                "obs_properties_create",
                unsafe extern "C" fn() -> *mut c_void
            ),
            props_add_bool: need!(
                "obs_properties_add_bool",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> *mut c_void
            ),
            props_add_list: need!(
                "obs_properties_add_list",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32, i32) -> *mut c_void
            ),
            props_add_text: need!(
                "obs_properties_add_text",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32) -> *mut c_void
            ),
            enter_graphics: need!("obs_enter_graphics", unsafe extern "C" fn()),
            leave_graphics: need!("obs_leave_graphics", unsafe extern "C" fn()),
            props_add_int: need!("obs_properties_add_int", unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32, i32, i32) -> *mut c_void),
            props_add_int_slider: need!(
                "obs_properties_add_int_slider",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8, i32, i32, i32)
                    -> *mut c_void
            ),
            props_add_float_slider: need!(
                "obs_properties_add_float_slider",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8, f64, f64, f64)
                    -> *mut c_void
            ),
            props_add_color: need!(
                "obs_properties_add_color",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> *mut c_void
            ),
            props_add_group: need!(
                "obs_properties_add_group",
                unsafe extern "C" fn(
                    *mut c_void,
                    *const u8,
                    *const u8,
                    i32,
                    *mut c_void,
                ) -> *mut c_void
            ),
            prop_list_add_str: need!(
                "obs_property_list_add_string",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> usize
            ),
            load_locale: need!("obs_module_load_locale", unsafe extern "C" fn(*mut c_void, *const u8, *const u8) -> *mut c_void),
            lookup_string: need!("text_lookup_getstr", unsafe extern "C" fn(*mut c_void, *const u8, *mut *const u8) -> bool),
            free_lookup: need!("text_lookup_destroy", unsafe extern "C" fn(*mut c_void)),
            data_get_bool: need!(
                "obs_data_get_bool",
                unsafe extern "C" fn(*mut c_void, *const u8) -> bool
            ),
            data_get_int: need!(
                "obs_data_get_int",
                unsafe extern "C" fn(*mut c_void, *const u8) -> i64
            ),
            data_get_double: need!(
                "obs_data_get_double",
                unsafe extern "C" fn(*mut c_void, *const u8) -> f64
            ),
            data_get_string: need!(
                "obs_data_get_string",
                unsafe extern "C" fn(*mut c_void, *const u8) -> *const u8
            ),
            data_set_default_bool: need!(
                "obs_data_set_default_bool",
                unsafe extern "C" fn(*mut c_void, *const u8, bool)
            ),
            data_set_default_int: need!(
                "obs_data_set_default_int",
                unsafe extern "C" fn(*mut c_void, *const u8, i64)
            ),
            data_set_default_double: need!(
                "obs_data_set_default_double",
                unsafe extern "C" fn(*mut c_void, *const u8, f64)
            ),
            data_set_default_string: need!(
                "obs_data_set_default_string",
                unsafe extern "C" fn(*mut c_void, *const u8, *const u8)
            ),
        };
        API.get_or_init(|| Some(api)).as_ref()
    }
}

/// For `obs_module_ver`: packs the host's version string "31.0.3" into a semver and returns it.
/// It returns the host's version as is, so the module loads on any OBS generation.
pub fn host_semver(api: &Api) -> u32 {
    let Some(vs) = api.version_string else {
        return 31 << 24;
    };
    let s = unsafe {
        let mut e = vs();
        let mut v = Vec::new();
        while *e != 0 && v.len() < 32 {
            v.push(*e);
            e = e.add(1);
        }
        String::from_utf8_lossy(&v).into_owned()
    };
    let mut it = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty());
    let mut part = |def: u32| -> u32 { it.next().and_then(|p| p.parse().ok()).unwrap_or(def) };
    let (maj, min, pat) = (part(31), part(0), part(0));
    (maj << 24) | (min << 16) | pat
}
