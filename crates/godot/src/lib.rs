//! Ayatsuri2D for Godot 4: a GDExtension that registers one node class,
//! `AyatsuriModel`, which loads and plays `.aom` models and Cubism `.moc3`
//! bundles on the same C ABI runtime the Unity package uses.
//!
//! ```text
//! node.rs      the AyatsuriModel class: properties, functions, the frame
//! painter.rs   RenderingServer canvas items, one per drawable
//! plan.rs      the decisions, with no Godot type: units, blends, order,
//!              what a model3.json names
//! runtime.rs   the ay2d_* calls as safe Rust
//! ```
//!
//! Godot 3 is not supported: GDNative is deprecated and the Rust bindings
//! for it ended in 2023. The C ABI (`ayatsuri.h`) remains for anyone who
//! wants to bind it by hand.

use godot::prelude::{ExtensionLibrary, gdextension};

mod node;
mod painter;
mod plan;
mod runtime;

pub use node::AyatsuriModel;

struct Ayatsuri2DExtension;

// SAFETY: the godot crate's contract for the entry point: this is the one
// ExtensionLibrary in the library, and the proc macro generates the
// `gdext_rust_init` symbol the .gdextension file names.
#[gdextension]
unsafe impl ExtensionLibrary for Ayatsuri2DExtension {}
