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
//!
//! The packing is shared with `godot3`, which does the same for the Godot
//! 3 addon.

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

/// What one addon's pack needs to know.
pub struct Addon {
    /// The addon folder in the repo, `sdk/<engine>/addons/ayatsuri2d`.
    pub dir: &'static str,
    /// Each platform's library: (SDK label, file in the SDK's lib/).
    pub platforms: &'static [(&'static str, &'static str)],
    /// The workspace package whose dependencies THIRD-PARTY.md lists.
    pub package: &'static str,
    /// The zip's name before the version: `ayatsuri2d-godot`.
    pub zip_stem: &'static str,
}

const GODOT4: Addon = Addon {
    dir: ADDON,
    platforms: PLATFORMS,
    package: "ayatsuri-godot",
    zip_stem: "ayatsuri2d-godot",
};

pub fn godot(args: &[String]) -> Result<()> {
    check()?;
    let sdks = pack_args("godot", args)?;
    if sdks.is_empty() { Ok(()) } else { pack_addon(&GODOT4, &sdks) }
}

/// The SDK folders after `--pack`, or none.
pub fn pack_args(task: &str, args: &[String]) -> Result<Vec<PathBuf>> {
    let mut it = args.iter();
    let mut sdks: Vec<PathBuf> = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pack" => sdks.extend(it.by_ref().map(PathBuf::from)),
            other => return Err(format!("{task}: unknown argument {other}").into()),
        }
    }
    Ok(sdks)
}

/// The `.gdextension` names the right entry symbol and a library under
/// `bin/<label>/` for every platform the pack places.
fn check() -> Result<()> {
    let text = std::fs::read_to_string(GDEXTENSION)?;
    let libraries = section_values(&text, "libraries");
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

/// The values under `[section]` of an ini-like file (a `.gdextension`, a
/// `.gdnlib`), quotes stripped, `;` comments skipped.
pub fn section_values(text: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == format!("[{section}]");
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

/// Stage an addon with each SDK's library under `bin/<label>/`, add the
/// licences, and zip it as `target/godot/<zip_stem>-<version>.zip`. Only
/// this addon's stage and zip are replaced; another addon's zip in the
/// same folder stays.
pub fn pack_addon(addon: &Addon, sdks: &[PathBuf]) -> Result<()> {
    let version = version()?;
    let root = Path::new("target/godot");
    let stage = root.join(format!("stage-{}", addon.zip_stem));
    if stage.exists() {
        std::fs::remove_dir_all(&stage)?;
    }
    std::fs::create_dir_all(root)?;
    let staged = stage.join("addons/ayatsuri2d");
    super::unity::copy_tree(Path::new(addon.dir), &staged)?;

    let mut placed = Vec::new();
    for sdk in sdks {
        let name = sdk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(&(label, file)) = addon.platforms.iter().find(|(l, _)| name.ends_with(l)) else {
            println!("skipping {}: no Godot platform for it", sdk.display());
            continue;
        };
        let from = sdk.join("lib").join(file);
        if !from.exists() {
            return Err(format!("{} has no lib/{file}", sdk.display()).into());
        }
        let dir = staged.join("bin").join(label);
        std::fs::create_dir_all(&dir)?;
        std::fs::copy(&from, dir.join(file))?;
        placed.push(label);
    }
    if placed.is_empty() {
        return Err("no platform library found in the given SDK folders".into());
    }

    std::fs::copy("crates/capi/LICENSE", staged.join("LICENSE-APACHE"))?;
    std::fs::copy("crates/core/LICENSE", staged.join("LICENSE-MIT"))?;
    std::fs::write(staged.join("THIRD-PARTY.md"), third_party_notices_for(&[addon.package])?)?;

    let zip = root.join(format!("{}-{version}.zip", addon.zip_stem));
    if zip.exists() {
        std::fs::remove_file(&zip)?;
    }
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
        assert_eq!(section_values(text, "libraries"), ["res://a.so", "res://b.dylib"]);
        assert_eq!(section_values(text, "configuration"), ["x"]);
        assert!(section_values(text, "entry").is_empty());
    }

    #[test]
    fn pack_takes_folders_after_the_flag() {
        let args = ["--pack".to_string(), "a".to_string(), "b".to_string()];
        let sdks = pack_args("godot", &args).unwrap_or_default();
        assert_eq!(sdks, [PathBuf::from("a"), PathBuf::from("b")]);
        assert!(pack_args("godot", &["--zip".to_string()]).is_err());
    }

    #[test]
    fn the_committed_file_passes_the_check() {
        // run from the workspace root, as cargo does for xtask's tests
        if Path::new(GDEXTENSION).exists() {
            assert!(check().is_ok());
        }
    }
}
