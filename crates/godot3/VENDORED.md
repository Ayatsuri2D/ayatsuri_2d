# Vendored: gdnative-sys 0.11.3

`vendor/gdnative-sys/` is the `gdnative-sys` crate, version 0.11.3, as
published on crates.io by the godot-rust developers (MIT). The published
crate carries no licence file of its own, only `license = "MIT"` in its
manifest, so `vendor/gdnative-sys/LICENSE.md` is the MIT licence from the
repository, https://github.com/godot-rust/gdnative (archived in 2023 after
this release), added here so the notice travels with the copy and into
the addon's THIRD-PARTY.md. The Godot headers it carries under
`godot_headers/` are MIT too, with their own `LICENSE.md`.

The workspace's `[patch.crates-io]` points `gdnative-sys` at this copy, so
the other `gdnative` crates (pulled from crates.io at `=0.11.3`) link
against it. The copy is excluded from the workspace, so it keeps its own
lint settings and its build script is not held to the workspace's.

## Why

As published, the crate generates its Rust bindings at build time with
bindgen 0.63. On libclang 16 and newer, bindgen 0.63 reads every struct in
`gdnative_api_struct.gen.h` as opaque (`pub _address: u8`), and the crate's
own `api_wrapper.rs`, which reaches into those structs for the function
pointers, then fails with hundreds of errors of the form

    error[E0609]: no field `extensions` on type `godot_gdnative_core_api_struct`

Newer bindgen versions handle those headers again.

## The patch

Two lines differ from the published crate:

1. `Cargo.toml`: the `bindgen` build dependency is `0.72.1` instead of
   `0.63.0`. Its `runtime` feature (libclang loaded at build time) and
   `default-features = false` are as upstream had them.
2. `build.rs`: the call `.size_t_is_usize(true)` is removed. bindgen 0.72
   no longer has the method; `size_t` is `usize` by default there, which is
   what the flag asked for.

And one note at the top of `build.rs`: `#![allow(non_local_definitions)]`,
so a lint newer than the crate (tripped by its `miniserde::make_place!`
use) does not warn on every build. It changes nothing in the output.

Also removed from the copy: `Cargo.toml.orig` and `.cargo_vcs_info.json`,
the packaging leftovers crates.io adds, and `godot_headers/images/`, six
screenshots for a FAQ that nothing in the build reads.

## What building needs

bindgen loads libclang at build time, as upstream's crate always did, so a
machine building `ayatsuri-godot3` needs a libclang shared library on the
usual paths (or `LIBCLANG_PATH` set). The GitHub-hosted runners the
release workflow uses ship one. Nothing else in the workspace needs it.

## Updating

There is nothing upstream to update to. If a future bindgen stops building
the headers again, the fix belongs in this copy's `Cargo.toml` and
`build.rs`, and in this file.
