//! `cargo xtask godot`: the Godot 4 addon in `sdk/godot`.
//!
//! With no arguments it checks the addon's `.gdextension` file: the entry
//! symbol the godot crate exports, a library for every platform the pack
//! knows, each under `bin/`. `--pack <dir>...` takes unpacked SDK downloads
//! (as `cargo xtask sdk` makes them, one per platform), puts each
//! platform's `ayatsuri2d_godot` library under `bin/<label>/` in a staging
//! copy of the addon, adds the licences, and zips the `addons/` folder as
//! `target/godot/ayatsuri2d-godot-<version>.zip`, which a user unzips into
//! a project.

use std::path::{Path, PathBuf};

use super::{Result, third_party_notices_for, version};

const ADDON: &str = "sdk/godot/addons/ayatsuri2d";
const GDEXTENSION: &str = "sdk/godot/addons/ayatsuri2d/ayatsuri2d.gdextension";
/// What the godot crate's `#[gdextension]` exports; no other symbol works.
const ENTRY: &str = "gdext_rust_init";

/// Each platform's library: (SDK label, file in its lib/).
const PLATFORMS: &[(&str, &str)] = &[
    ("linux-x64", "libayatsuri2d_godot.so"),
    ("linux-arm64", "libayatsuri2d_godot.so"),
    ("windows-x64", "ayatsuri2d_godot.dll"),
    ("macos-universal", "libayatsuri2d_godot.dylib"),
];

pub fn godot(args: &[String]) -> Result<()> {
    check()?;
    let mut it = args.iter();
    let mut sdks: Vec<PathBuf> = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pack" => sdks.extend(it.by_ref().map(PathBuf::from)),
            other => return Err(format!("godot: unknown argument {other}").into()),
        }
    }
    if sdks.is_empty() { Ok(()) } else { pack(&sdks) }
}

/// The `.gdextension` names the right entry symbol and a library under
/// `bin/<label>/` for every platform the pack places.
fn check() -> Result<()> {
    let text = std::fs::read_to_string(GDEXTENSION)?;
    let libraries = libraries(&text);
    if !text.contains(&format!("entry_symbol = \"{ENTRY}\"")) {
        return Err(format!("{GDEXTENSION} must name entry_symbol = \"{ENTRY}\"").into());
    }
    for (label, file) in PLATFORMS {
        let want = format!("res://addons/ayatsuri2d/bin/{label}/{file}");
        if !libraries.iter().any(|l| l == &want) {
            return Err(format!("{GDEXTENSION} has no library at {want}").into());
        }
    }
    println!("{GDEXTENSION} names {ENTRY} and {} libraries", libraries.len());
    Ok(())
}

/// The paths under `[libraries]`.
fn libraries(text: &str) -> Vec<String> {
    let mut in_section = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[libraries]";
            continue;
        }
        if !in_section || line.starts_with(';') {
            continue;
        }
        if let Some((_, value)) = line.split_once('=') {
            out.push(value.trim().trim_matches('"').to_string());
        }
    }
    out
}

fn pack(sdks: &[PathBuf]) -> Result<()> {
    let version = version()?;
    let root = Path::new("target/godot");
    let stage = root.join("stage");
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    let addon = stage.join("addons/ayatsuri2d");
    super::unity::copy_tree(Path::new(ADDON), &addon)?;

    let mut placed = Vec::new();
    for sdk in sdks {
        let name = sdk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(&(label, file)) = PLATFORMS.iter().find(|(l, _)| name.ends_with(l)) else {
            println!("skipping {}: no Godot platform for it", sdk.display());
            continue;
        };
        let from = sdk.join("lib").join(file);
        if !from.exists() {
            return Err(format!("{} has no lib/{file}", sdk.display()).into());
        }
        let dir = addon.join("bin").join(label);
        std::fs::create_dir_all(&dir)?;
        std::fs::copy(&from, dir.join(file))?;
        placed.push(label);
    }
    if placed.is_empty() {
        return Err("no platform library found in the given SDK folders".into());
    }

    std::fs::copy("crates/capi/LICENSE", addon.join("LICENSE-APACHE"))?;
    std::fs::copy("crates/core/LICENSE", addon.join("LICENSE-MIT"))?;
    std::fs::write(addon.join("THIRD-PARTY.md"), third_party_notices_for(&["ayatsuri-godot"])?)?;

    let zip = root.join(format!("ayatsuri2d-godot-{version}.zip"));
    let zip_abs = std::env::current_dir()?.join(&zip);
    // zip from inside the stage, so the archive starts at addons/
    let stage_abs = std::env::current_dir()?.join(&stage);
    let status = std::process::Command::new("zip")
        .args(["-qr", &zip_abs.to_string_lossy(), "addons"])
        .current_dir(&stage_abs)
        .status()
        .map_err(|e| format!("zip: {e} (is zip installed?)"))?;
    if !status.success() {
        return Err("zip failed".into());
    }
    println!("{}: {}", zip.display(), placed.join(", "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_paths_are_read_from_their_section() {
        let text = "[configuration]\nentry_symbol = \"x\"\n\n[libraries]\n; a note\nlinux.x86_64 = \"res://a.so\"\nmacos = \"res://b.dylib\"\n";
        assert_eq!(libraries(text), ["res://a.so", "res://b.dylib"]);
    }

    #[test]
    fn the_committed_file_passes_the_check() {
        // run from the workspace root, as cargo does for xtask's tests
        if Path::new(GDEXTENSION).exists() {
            assert!(check().is_ok());
        }
    }
}
