//! Ayatsuri2D for Godot 3: a GDNative library that registers one
//! NativeScript class, `AyatsuriModel` (on a `Node2D`), which loads and
//! plays `.aom` models and Cubism `.moc3` bundles on the same C ABI runtime
//! the Unity package and the Godot 4 addon use.
//!
//! ```text
//! node.rs      the AyatsuriModel class: properties, methods, the frame
//! painter.rs   VisualServer canvas items, one per drawable; mask sheets
//! mask.rs      masks without canvas groups: sheet size, the shader
//! plan         (ayatsuri-godot-plan) the decisions, with no Godot type:
//!              units, blends, order, what a model3.json names
//! runtime      (ayatsuri-godot-plan) the ay2d_* calls as safe Rust
//! ```
//!
//! The binding is the `gdnative` crate (0.11.3, the last release; see
//! VENDORED.md for the one patched crate). Its bindings are Godot 3.5.1's
//! GDNative API, which Godot 3.6 carries unchanged, so one library serves
//! both.

use std::ffi::c_void;

use gdnative::api::Engine;
use gdnative::init::godot_gdnative_terminate;
use gdnative::init::{InitHandle, InitLevel, auto_register, godot_gdnative_init};
use gdnative::log::godot_warn;

use ayatsuri_godot_plan::{breath, plan, runtime};

mod mask;
mod node;
mod painter;

pub use node::AyatsuriModel;

/// Register the class. A tool class, so the model shows in the editor as
/// the Godot 4 node does.
fn init(handle: InitHandle) {
    handle.add_tool_class::<AyatsuriModel>();
}

godot_gdnative_init!();
godot_gdnative_terminate!();

/// The NativeScript entry point, written out instead of through
/// `godot_nativescript_init!`: the macro warns on every engine that is not
/// 3.5.x, and this addon runs on 3.6 as well. The body is the macro's,
/// with a version note of its own.
///
/// # Safety
///
/// Called by Godot, once, with its NativeScript handle, after
/// `godot_gdnative_init` bound the API.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn godot_nativescript_init(handle: *mut c_void) {
    if !gdnative::private::is_api_bound() {
        return;
    }
    gdnative::private::report_panics("nativescript_init", || {
        // SAFETY: `handle` is the one Godot passed to this entry point,
        // valid for the call; InitHandle::new asks for nothing more.
        unsafe {
            auto_register(InitHandle::new(handle, InitLevel::AUTO));
            init(InitHandle::new(handle, InitLevel::USER));
        }
        version_note();
    });
}

/// Say so once when the engine is not one the binding was made for.
fn version_note() {
    let info = Engine::godot_singleton().get_version_info();
    let field = |k: &str| info.get(k).and_then(|v| v.to::<i64>()).unwrap_or(0);
    let (major, minor) = (field("major"), field("minor"));
    if major != 3 || !(5..=6).contains(&minor) {
        godot_warn!(
            "Ayatsuri2D: this addon is built for Godot 3.5 and 3.6 (found {major}.{minor}); other versions are untested"
        );
    }
}
