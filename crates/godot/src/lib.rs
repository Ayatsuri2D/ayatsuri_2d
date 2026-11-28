//! Ayatsuri2D for Godot 4: a GDExtension that registers one node class,
//! `AyatsuriModel`, which loads and plays `.aom` models and Cubism `.moc3`
//! bundles on the same C ABI runtime the Unity package uses.
//!
//! ```text
//! node.rs      the AyatsuriModel class: properties, functions, the frame
//! painter.rs   RenderingServer canvas items, one per drawable
//! plan         (ayatsuri-godot-plan) the decisions, with no Godot type:
//!              units, blends, order, what a model3.json names
//! runtime      (ayatsuri-godot-plan) the ay2d_* calls as safe Rust
//! ```
//!
//! Godot 3 has its own addon, `ayatsuri-godot3`, a GDNative library over
//! the same two shared modules.

use godot::prelude::{ExtensionLibrary, gdextension};

use ayatsuri_godot_plan::{breath, plan, runtime};

mod node;
mod painter;

pub use node::AyatsuriModel;

struct Ayatsuri2DExtension;

// SAFETY: the godot crate's contract for the entry point: this is the one
// ExtensionLibrary in the library, and the proc macro generates the
// `gdext_rust_init` symbol the .gdextension file names.
#[gdextension]
unsafe impl ExtensionLibrary for Ayatsuri2DExtension {}
