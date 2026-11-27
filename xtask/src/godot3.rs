//! `cargo xtask godot3`: the Godot 3 addon in `sdk/godot3`.
//!
//! With no arguments it checks the addon's `.gdnlib` file (the `godot_`
//! symbol prefix the gdnative crate's entry points carry, a library for
//! every platform the pack knows, each under `bin/`) and its `.gdns` (the
//! class name the library registers, the `.gdnlib` it points at). `--pack
//! <dir>...` takes unpacked SDK downloads, puts each platform's
//! `ayatsuri2d_godot3` library under `bin/<label>/` in a staging copy of
//! the addon, adds the licences, and zips the `addons/` folder as
//! `target/godot/ayatsuri2d-godot3-<version>.zip`.

use std::path::Path;

use super::Result;
use super::godot::{Addon, pack_addon, pack_args, section_values};

const ADDON: &str = "sdk/godot3/addons/ayatsuri2d";
const GDNLIB: &str = "sdk/godot3/addons/ayatsuri2d/ayatsuri2d.gdnlib";
const GDNS: &str = "sdk/godot3/addons/ayatsuri2d/ayatsuri_model.gdns";
/// What the gdnative crate's entry points are called: `godot_gdnative_init`
/// and the two beside it.
const SYMBOL_PREFIX: &str = "godot_";
/// The one class `ayatsuri-godot3` registers.
const CLASS: &str = "AyatsuriModel";

/// Each platform's library: (SDK label, file in its lib/).
const PLATFORMS: &[(&str, &str)] = &[
    ("linux-x64", "libayatsuri2d_godot3.so"),
    ("linux-arm64", "libayatsuri2d_godot3.so"),
    ("windows-x64", "ayatsuri2d_godot3.dll"),
    ("macos-universal", "libayatsuri2d_godot3.dylib"),
];

const GODOT3: Addon = Addon {
    dir: ADDON,
    platforms: PLATFORMS,
    package: "ayatsuri-godot3",
    zip_stem: "ayatsuri2d-godot3",
};

pub fn godot3(args: &[String]) -> Result<()> {
    check()?;
    let sdks = pack_args("godot3", args)?;
    if sdks.is_empty() { Ok(()) } else { pack_addon(&GODOT3, &sdks) }
}

/// The `.gdnlib` names the entry prefix and a library under `bin/<label>/`
/// for every platform the pack places; the `.gdns` names the class and
/// the `.gdnlib`.
fn check() -> Result<()> {
    let text = std::fs::read_to_string(GDNLIB)?;
    check_gdnlib(&text)?;
    let script = std::fs::read_to_string(GDNS)?;
    check_gdns(&script)?;
    let entries = section_values(&text, "entry");
    println!("{GDNLIB} names {SYMBOL_PREFIX}* and {} libraries; {GDNS} is {CLASS}", entries.len());
    Ok(())
}

fn check_gdnlib(text: &str) -> Result<()> {
    let general = section_values(text, "general");
    if !text.contains(&format!("symbol_prefix=\"{SYMBOL_PREFIX}\"")) || general.is_empty() {
        return Err(
            format!("{GDNLIB} must set symbol_prefix=\"{SYMBOL_PREFIX}\" under [general]").into()
        );
    }
    let entries = section_values(text, "entry");
    for (label, file) in PLATFORMS {
        let want = format!("res://addons/ayatsuri2d/bin/{label}/{file}");
        if !entries.iter().any(|l| l == &want) {
            return Err(format!("{GDNLIB} has no entry at {want}").into());
        }
    }
    Ok(())
}

fn check_gdns(script: &str) -> Result<()> {
    if !script.contains("type=\"NativeScript\"") {
        return Err(format!("{GDNS} is not a NativeScript resource").into());
    }
    if !script.contains(&format!("class_name = \"{CLASS}\"")) {
        return Err(format!("{GDNS} must name class_name = \"{CLASS}\"").into());
    }
    let lib = format!(
        "path=\"res://addons/ayatsuri2d/{}\"",
        Path::new(GDNLIB).file_name().and_then(|n| n.to_str()).unwrap_or_default()
    );
    if !script.contains(&lib) {
        return Err(format!("{GDNS} must point at {GDNLIB}").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_LIB: &str = "[general]\n\nsingleton=false\nsymbol_prefix=\"godot_\"\n\n[entry]\n\nX11.arm64=\"res://addons/ayatsuri2d/bin/linux-arm64/libayatsuri2d_godot3.so\"\nX11.x86_64=\"res://addons/ayatsuri2d/bin/linux-x64/libayatsuri2d_godot3.so\"\nWindows.64=\"res://addons/ayatsuri2d/bin/windows-x64/ayatsuri2d_godot3.dll\"\nOSX.64=\"res://addons/ayatsuri2d/bin/macos-universal/libayatsuri2d_godot3.dylib\"\n";

    #[test]
    fn a_gdnlib_needs_the_prefix_and_every_platform() {
        assert!(check_gdnlib(GOOD_LIB).is_ok());
        assert!(check_gdnlib(&GOOD_LIB.replace("godot_", "ay_")).is_err());
        assert!(check_gdnlib(&GOOD_LIB.replace("OSX.64", ";OSX.64")).is_err());
    }

    #[test]
    fn a_gdns_names_the_class_and_the_library() {
        let good = "[gd_resource type=\"NativeScript\" load_steps=2 format=2]\n\n[ext_resource path=\"res://addons/ayatsuri2d/ayatsuri2d.gdnlib\" type=\"GDNativeLibrary\" id=1]\n\n[resource]\nclass_name = \"AyatsuriModel\"\nlibrary = ExtResource( 1 )\n";
        assert!(check_gdns(good).is_ok());
        assert!(check_gdns(&good.replace("AyatsuriModel", "Model")).is_err());
        assert!(check_gdns(&good.replace("ayatsuri2d.gdnlib", "other.gdnlib")).is_err());
        assert!(check_gdns(&good.replace("NativeScript", "GDScript")).is_err());
    }

    #[test]
    fn the_committed_files_pass_the_check() {
        // run from the workspace root, as cargo does for xtask's tests
        if Path::new(GDNLIB).exists() {
            assert!(check().is_ok());
        }
    }
}
