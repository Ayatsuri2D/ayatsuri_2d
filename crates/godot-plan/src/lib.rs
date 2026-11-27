//! What the Godot addons share and Godot never sees: the C ABI runtime as
//! safe Rust (`runtime`), and the decisions behind the drawing (`plan`):
//! how a model unit becomes a pixel, which blend a drawable wants, what a
//! frame's draw order means for the canvas items already made, and what
//! a `model3.json` bundle names.
//!
//! The Godot 4 addon (`ayatsuri-godot`, a GDExtension) and the Godot 3
//! addon (`ayatsuri-godot3`, a GDNative library) both build on this crate,
//! so the two draw the same model the same way and the logic is tested
//! once, here, without an engine.

pub mod plan;
pub mod runtime;
