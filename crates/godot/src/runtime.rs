//! The C ABI runtime as safe Rust: one owned handle, Rust strings and
//! slices in and out, and the raw pointers of `ay2d_drawable` turned into
//! borrowed views. Every `unsafe` block in this crate that touches the
//! runtime lives here, so the node and the painter are plain safe code.
//!
//! The calls are the same `ay2d_*` functions the Unity package loads from
//! the shared library, called directly: the Godot addon and the Unity
//! package share one runtime behaviour (threading, motions, expressions,
//! physics and fades) because they share the code, not a copy of it.

use std::ffi::{CStr, CString, c_char};
use std::ptr::NonNull;

use ayatsuri_capi as c;
pub use ayatsuri_capi::{Canvas, Status};

/// A loaded model. Freed with the value.
pub struct Runtime {
    model: NonNull<c::Model>,
}

/// One mesh to draw, as borrowed slices: `positions` and `uvs` hold xy
/// pairs, `indices` a triangle list into them, `masks` mesh indices.
#[derive(Debug, Clone, Copy)]
pub struct Drawable<'a> {
    pub mesh: u32,
    pub texture: u32,
    pub flags: u32,
    pub opacity: f32,
    pub positions: &'a [f32],
    pub uvs: &'a [f32],
    pub indices: &'a [u16],
    pub masks: &'a [u32],
}

/// An event a motion fired during the last update.
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    pub motion: u32,
    pub time: f32,
    pub name: String,
}

/// Why the last runtime call on this thread failed.
pub fn last_error() -> String {
    string_at(c::ay2d_last_error()).unwrap_or_default()
}

/// A NUL-terminated string the runtime handed out, or `None` for null.
fn string_at(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the ABI's contract: every string the runtime returns is
    // NUL-terminated and lives as long as the model (or the library, for
    // the version and the error text), and we copy it out at once.
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// A Rust string as the NUL-terminated argument the runtime takes. A NUL
/// inside the text is dropped rather than ending it early.
fn c_string(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// An empty drawable slot for the runtime to fill.
const EMPTY: c::Drawable = c::Drawable {
    mesh: 0,
    texture: 0,
    flags: 0,
    opacity: 0.0,
    vertex_count: 0,
    index_count: 0,
    mask_count: 0,
    positions: std::ptr::null(),
    uvs: std::ptr::null(),
    indices: std::ptr::null(),
    masks: std::ptr::null(),
};

/// A borrowed view over one `ay2d_drawable`.
///
/// The caller promises the drawable came from `ay2d_drawables` or
/// `ay2d_mesh_geometry` on a model that is still alive and has not been
/// updated twice since: the ABI keeps the views valid while the next
/// update runs, until the one after it starts.
fn view<'a>(d: &c::Drawable) -> Drawable<'a> {
    fn slice<'a, T>(p: *const T, n: usize) -> &'a [T] {
        if p.is_null() || n == 0 {
            return &[];
        }
        // SAFETY: the ABI's contract (see `view`): `n` readable elements at
        // `p`, alive for the lifetime the caller chose.
        unsafe { std::slice::from_raw_parts(p, n) }
    }
    let n = d.vertex_count as usize;
    Drawable {
        mesh: d.mesh,
        texture: d.texture,
        flags: d.flags,
        opacity: d.opacity,
        positions: slice(d.positions, n.saturating_mul(2)),
        uvs: slice(d.uvs, n.saturating_mul(2)),
        indices: slice(d.indices, d.index_count as usize),
        masks: slice(d.masks, d.mask_count as usize),
    }
}

impl Runtime {
    /// Load an `.aom` or a bare `.moc3` from its bytes.
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let p = c::ay2d_load(bytes.as_ptr(), bytes.len());
        NonNull::new(p).map(|model| Self { model }).ok_or_else(last_error)
    }

    fn ptr(&self) -> *const c::Model {
        self.model.as_ptr()
    }

    fn ptr_mut(&self) -> *mut c::Model {
        self.model.as_ptr()
    }

    /// The runtime's own version string.
    pub fn version() -> String {
        string_at(c::ay2d_version()).unwrap_or_default()
    }

    /// The ABI the runtime was built with, and the one this crate expects.
    pub fn abi_matches() -> bool {
        c::ay2d_abi_version() == c::AY2D_ABI_VERSION
    }

    // ---- a moc3 bundle's JSON files ----

    pub fn add_physics3(&self, json: &[u8]) -> Result<(), String> {
        match c::ay2d_add_physics3(self.ptr_mut(), json.as_ptr(), json.len()) {
            Status::Ok => Ok(()),
            _ => Err(last_error()),
        }
    }

    pub fn add_motion3(&self, name: &str, json: &[u8]) -> Result<u32, String> {
        let name = c_string(name);
        let i = c::ay2d_add_motion3(self.ptr_mut(), name.as_ptr(), json.as_ptr(), json.len());
        u32::try_from(i).map_err(|_| last_error())
    }

    pub fn add_expression3(&self, name: &str, json: &[u8]) -> Result<u32, String> {
        let name = c_string(name);
        let i = c::ay2d_add_expression3(self.ptr_mut(), name.as_ptr(), json.as_ptr(), json.len());
        u32::try_from(i).map_err(|_| last_error())
    }

    // ---- facts ----

    pub fn canvas(&self) -> Canvas {
        let mut out = Canvas::default();
        c::ay2d_canvas(self.ptr(), &mut out);
        out
    }

    pub fn parameter_count(&self) -> u32 {
        c::ay2d_parameter_count(self.ptr())
    }

    pub fn part_count(&self) -> u32 {
        c::ay2d_part_count(self.ptr())
    }

    pub fn mesh_count(&self) -> u32 {
        c::ay2d_mesh_count(self.ptr())
    }

    pub fn texture_count(&self) -> u32 {
        c::ay2d_texture_count(self.ptr())
    }

    pub fn parameter_id(&self, i: u32) -> Option<String> {
        string_at(c::ay2d_parameter_id(self.ptr(), i))
    }

    pub fn parameter_name(&self, i: u32) -> Option<String> {
        string_at(c::ay2d_parameter_name(self.ptr(), i))
    }

    pub fn parameter_index(&self, id: &str) -> i32 {
        c::ay2d_parameter_index(self.ptr(), c_string(id).as_ptr())
    }

    pub fn part_id(&self, i: u32) -> Option<String> {
        string_at(c::ay2d_part_id(self.ptr(), i))
    }

    /// Every parameter's minimum, maximum and default, by index.
    pub fn parameter_ranges(&self) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let n = self.parameter_count() as usize;
        let (mut min, mut max, mut def) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        c::ay2d_parameter_ranges(
            self.ptr(),
            min.as_mut_ptr(),
            max.as_mut_ptr(),
            def.as_mut_ptr(),
            n,
        );
        (min, max, def)
    }

    /// Mesh `i`'s blend as moc3 5.3 encodes it (see `ay2d_mesh_blend`).
    pub fn mesh_blend(&self, i: u32) -> u32 {
        c::ay2d_mesh_blend(self.ptr(), i)
    }

    /// Texture page `i` as PNG bytes.
    pub fn texture(&self, i: u32) -> Option<&[u8]> {
        let mut bytes: *const u8 = std::ptr::null();
        let mut len: usize = 0;
        if c::ay2d_texture(self.ptr(), i, &mut bytes, &mut len) != Status::Ok || bytes.is_null() {
            return None;
        }
        // SAFETY: the ABI's contract: `len` readable bytes at `bytes`, alive
        // as long as the model, which `self` keeps alive for the borrow.
        Some(unsafe { std::slice::from_raw_parts(bytes, len) })
    }

    // ---- the pose ----

    /// Every parameter's value after the last update, into `out`.
    pub fn get_parameters(&self, out: &mut [f32]) -> Status {
        c::ay2d_get_parameters(self.ptr(), out.as_mut_ptr(), out.len())
    }

    /// Set `indices[i]` to `values[i]` for the next update.
    pub fn set_parameters(&self, indices: &[u32], values: &[f32]) -> Status {
        let n = indices.len().min(values.len());
        c::ay2d_set_parameters(self.ptr_mut(), indices.as_ptr(), values.as_ptr(), n)
    }

    pub fn set_part_opacities(&self, indices: &[u32], values: &[f32]) -> Status {
        let n = indices.len().min(values.len());
        c::ay2d_set_part_opacities(self.ptr_mut(), indices.as_ptr(), values.as_ptr(), n)
    }

    /// Advance the model by `dt` seconds and evaluate it.
    pub fn update(&self, dt: f32) -> Status {
        c::ay2d_update(self.ptr_mut(), dt)
    }

    // ---- what to draw ----

    /// The visible meshes after the last update, back to front. `buf` is
    /// the caller's scratch space, kept between frames so nothing is
    /// allocated once it has grown to the model's size.
    pub fn drawables<'a>(&'a self, buf: &'a mut Vec<c::Drawable>) -> Vec<Drawable<'a>> {
        let n = c::ay2d_drawables(self.ptr(), std::ptr::null_mut(), 0) as usize;
        if buf.len() < n {
            buf.resize(n, EMPTY);
        }
        let got = c::ay2d_drawables(self.ptr(), buf.as_mut_ptr(), buf.len()) as usize;
        buf.iter().take(got.min(n)).map(view).collect()
    }

    /// A mesh's geometry after the last update, visible or not.
    pub fn mesh_geometry(&self, mesh: u32) -> Option<Drawable<'_>> {
        let mut out = EMPTY;
        match c::ay2d_mesh_geometry(self.ptr(), mesh, &mut out) {
            Status::Ok => Some(view(&out)),
            _ => None,
        }
    }

    /// What changed for every mesh in the last update, into `out`
    /// (`AY2D_CHANGED_*` bits by mesh index).
    pub fn mesh_changes(&self, out: &mut Vec<u32>) -> Status {
        out.clear();
        out.resize(self.mesh_count() as usize, 0);
        c::ay2d_mesh_changes(self.ptr(), out.as_mut_ptr(), out.len())
    }

    // ---- motions ----

    pub fn motion_count(&self) -> u32 {
        c::ay2d_motion_count(self.ptr())
    }

    pub fn motion_id(&self, i: u32) -> Option<String> {
        string_at(c::ay2d_motion_id(self.ptr(), i))
    }

    pub fn motion_index(&self, name: &str) -> i32 {
        c::ay2d_motion_index(self.ptr(), c_string(name).as_ptr())
    }

    pub fn motion_play(&self, i: u32, looping: bool, fade_in: f32) -> Status {
        c::ay2d_motion_play(self.ptr_mut(), i, i32::from(looping), fade_in)
    }

    pub fn motion_stop(&self, fade_out: f32) -> Status {
        c::ay2d_motion_stop(self.ptr_mut(), fade_out)
    }

    pub fn motion_playing(&self) -> i32 {
        c::ay2d_motion_playing(self.ptr())
    }

    pub fn motion_finished(&self) -> bool {
        c::ay2d_motion_finished(self.ptr()) != 0
    }

    pub fn motion_time(&self) -> f32 {
        c::ay2d_motion_time(self.ptr())
    }

    pub fn motion_seek(&self, t: f32) -> Status {
        c::ay2d_motion_seek(self.ptr_mut(), t)
    }

    pub fn motion_set_speed(&self, speed: f32) -> Status {
        c::ay2d_motion_set_speed(self.ptr_mut(), speed)
    }

    /// The events motions fired during the last update.
    pub fn motion_events(&self) -> Vec<Fired> {
        let n = c::ay2d_motion_events(self.ptr(), std::ptr::null_mut(), 0) as usize;
        if n == 0 {
            return Vec::new();
        }
        let mut buf = vec![c::Event { motion: 0, time: 0.0, name: std::ptr::null() }; n];
        let got = c::ay2d_motion_events(self.ptr(), buf.as_mut_ptr(), buf.len()) as usize;
        buf.iter()
            .take(got.min(n))
            .map(|e| Fired {
                motion: e.motion,
                time: e.time,
                name: string_at(e.name).unwrap_or_default(),
            })
            .collect()
    }

    // ---- expressions ----

    pub fn expression_count(&self) -> u32 {
        c::ay2d_expression_count(self.ptr())
    }

    pub fn expression_id(&self, i: u32) -> Option<String> {
        string_at(c::ay2d_expression_id(self.ptr(), i))
    }

    pub fn expression_index(&self, name: &str) -> i32 {
        c::ay2d_expression_index(self.ptr(), c_string(name).as_ptr())
    }

    pub fn expression_set(&self, i: u32, fade_in: f32) -> Status {
        c::ay2d_expression_set(self.ptr_mut(), i, fade_in)
    }

    pub fn expression_clear(&self, fade_out: f32) -> Status {
        c::ay2d_expression_clear(self.ptr_mut(), fade_out)
    }

    pub fn expression_shown(&self) -> i32 {
        c::ay2d_expression_shown(self.ptr())
    }

    // ---- physics ----

    pub fn has_physics(&self) -> bool {
        c::ay2d_has_physics(self.ptr()) != 0
    }

    pub fn set_physics(&self, on: bool) -> Status {
        c::ay2d_set_physics(self.ptr_mut(), i32::from(on))
    }

    pub fn set_wind(&self, x: f32, y: f32) -> Status {
        c::ay2d_set_wind(self.ptr_mut(), x, y)
    }

    pub fn reset_physics(&self) -> Status {
        c::ay2d_reset_physics(self.ptr_mut())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        c::ay2d_free(self.model.as_ptr());
    }
}
