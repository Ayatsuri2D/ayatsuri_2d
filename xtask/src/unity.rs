//! `cargo xtask unity`: the Unity package in `sdk/unity`.
//!
//! With no arguments it checks the package against the C header: every
//! `ay2d_` function has a declaration in `Native.cs`, and the ABI version
//! matches. `--pack <dir>...` takes unpacked SDK downloads (as `cargo xtask
//! sdk` makes them, one per platform), puts each platform's library under
//! `Plugins/`, writes the `.meta` files Unity needs for a package it cannot
//! write into, and makes `dist/release/ayatsuri2d-unity-<version>.tgz`, the
//! tarball Unity's Package Manager installs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{HEADER, Result, run, third_party_notices, version};

const PACKAGE: &str = "sdk/unity";
const NATIVE: &str = "sdk/unity/Runtime/Native.cs";

pub fn unity(args: &[String]) -> Result<()> {
    check()?;
    let mut it = args.iter();
    let mut sdks: Vec<PathBuf> = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pack" => {
                sdks.extend(it.by_ref().map(PathBuf::from));
            }
            other => return Err(format!("unity: unknown argument {other}").into()),
        }
    }
    if sdks.is_empty() { Ok(()) } else { pack(&sdks) }
}

/// The header's functions all declared, the ABI version the same.
fn check() -> Result<()> {
    let header = std::fs::read_to_string(HEADER)?;
    let native = std::fs::read_to_string(NATIVE)?;
    let declared = |name: &str| native.contains(&format!(" {name}("));
    let missing: Vec<String> = functions(&header).into_iter().filter(|f| !declared(f)).collect();
    if !missing.is_empty() {
        return Err(format!("{NATIVE} lacks {}: declare them", missing.join(", ")).into());
    }
    let abi = header
        .lines()
        .find_map(|l| l.strip_prefix("#define AY2D_ABI_VERSION "))
        .map(str::trim)
        .ok_or("no AY2D_ABI_VERSION in the header")?;
    if !native.contains(&format!("public const uint AbiVersion = {abi};")) {
        return Err(format!("{NATIVE} does not say AbiVersion = {abi}").into());
    }
    println!("{NATIVE} declares every function in {HEADER}");
    Ok(())
}

/// Every `ay2d_` function the header declares.
fn functions(header: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in header.lines() {
        let line = line.trim_start();
        if line.starts_with('#') || line.starts_with("typedef") || line.starts_with('*') {
            continue;
        }
        let Some(open) = line.find('(') else { continue };
        let Some(start) = line[..open].rfind(|c: char| !(c.is_alphanumeric() || c == '_')) else {
            continue;
        };
        let name = &line[start + 1..open];
        if name.starts_with("ay2d_") {
            out.insert(name.to_string());
        }
    }
    out
}

/// Where each platform's library goes in the package, and how Unity
/// should treat it: (SDK label, file in its lib/, folder in the package).
const PLUGINS: &[(&str, &str, &str)] = &[
    ("windows-x64", "ayatsuri_capi.dll", "Plugins/Windows/x86_64"),
    ("linux-x64", "libayatsuri_capi.so", "Plugins/Linux/x86_64"),
    ("macos-universal", "libayatsuri_capi.dylib", "Plugins/macOS"),
    ("android-arm64", "libayatsuri_capi.so", "Plugins/Android/arm64-v8a"),
    ("ios-arm64", "libayatsuri_capi.a", "Plugins/iOS"),
];

fn pack(sdks: &[PathBuf]) -> Result<()> {
    let version = version()?;
    let root = Path::new("target/unity");
    let stage = root.join("package");
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    copy_tree(Path::new(PACKAGE), &stage)?;

    // the version the release carries
    let manifest = stage.join("package.json");
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&manifest)?)?;
    json["version"] = serde_json::Value::String(version.clone());
    std::fs::write(&manifest, serde_json::to_string_pretty(&json)? + "\n")?;

    let mut placed = Vec::new();
    for sdk in sdks {
        let name = sdk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(&(label, file, dir)) = PLUGINS.iter().find(|(l, _, _)| name.ends_with(l)) else {
            println!("skipping {}: no Unity platform for it", sdk.display());
            continue;
        };
        let from = sdk.join("lib").join(file);
        if !from.exists() {
            return Err(format!("{} has no lib/{file}", sdk.display()).into());
        }
        std::fs::create_dir_all(stage.join(dir))?;
        std::fs::copy(&from, stage.join(dir).join(file))?;
        placed.push(label);
    }
    if placed.is_empty() {
        return Err("no platform library found in the given SDK folders".into());
    }

    std::fs::copy("crates/capi/LICENSE", stage.join("LICENSE"))?;
    std::fs::copy("crates/core/LICENSE", stage.join("LICENSE-MIT"))?;
    std::fs::write(stage.join("THIRD-PARTY.md"), third_party_notices()?)?;
    metas(&stage, &stage)?;

    let out = Path::new("dist/release");
    std::fs::create_dir_all(out)?;
    let tgz = out.join(format!("ayatsuri2d-unity-{version}.tgz"));
    let tgz_abs = std::env::current_dir()?.join(&tgz);
    run("tar", &["-czf", &tgz_abs.to_string_lossy(), "-C", &root.to_string_lossy(), "package"])?;
    println!("{}: {}", tgz.display(), placed.join(", "));
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let s = name.to_string_lossy();
        // metas a local Unity wrote into the checkout are not ours
        if s.ends_with(".meta") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(&name))?;
        } else {
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

/// A `.meta` beside every file and folder: an installed package is
/// read-only, and Unity ignores what has none. The GUID comes from the
/// path, so it stays the same from release to release and references to
/// the package's assets keep working. `Samples~` is skipped, as Unity
/// skips it; it copies a sample into the project when asked.
fn metas(base: &Path, dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let rel = path.strip_prefix(base)?.to_string_lossy().replace('\\', "/");
        if rel.ends_with('~') {
            continue;
        }
        let guid = guid(&rel);
        let body = if path.is_dir() {
            metas(base, &path)?;
            format!(
                "fileFormatVersion: 2\nguid: {guid}\nfolderAsset: yes\nDefaultImporter:\n{TAIL}"
            )
        } else {
            importer(&rel, &guid)
        };
        let mut meta = path.clone().into_os_string();
        meta.push(".meta");
        std::fs::write(meta, body)?;
    }
    Ok(())
}

const TAIL: &str =
    "  externalObjects: {}\n  userData: \n  assetBundleName: \n  assetBundleVariant: \n";

fn importer(rel: &str, guid: &str) -> String {
    let head = format!("fileFormatVersion: 2\nguid: {guid}\n");
    let ext = rel.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if rel.starts_with("Plugins/") {
        return format!("{head}{}", plugin(rel));
    }
    match ext {
        "cs" => format!(
            "{head}MonoImporter:\n  externalObjects: {{}}\n  serializedVersion: 2\n  defaultReferences: []\n  executionOrder: 0\n  icon: {{instanceID: 0}}\n  userData: \n  assetBundleName: \n  assetBundleVariant: \n"
        ),
        "asmdef" => format!("{head}AssemblyDefinitionImporter:\n{TAIL}"),
        "shader" => format!(
            "{head}ShaderImporter:\n  externalObjects: {{}}\n  defaultTextures: []\n  nonModifiableTextures: []\n  userData: \n  assetBundleName: \n  assetBundleVariant: \n"
        ),
        "json" if rel == "package.json" => format!("{head}PackageManifestImporter:\n{TAIL}"),
        "json" | "md" | "txt" => format!("{head}TextScriptImporter:\n{TAIL}"),
        _ => format!("{head}DefaultImporter:\n{TAIL}"),
    }
}

/// A player platform in a plugin's meta: (key, value, CPU).
type Player = (&'static str, &'static str, &'static str);

/// Which platforms a library loads on, so each build takes only its own.
fn plugin(rel: &str) -> String {
    // the editor's OS and CPU (none: not in the editor), then each player
    let (editor, player): (Option<(&str, &str)>, Player) = if rel.contains("/Windows/") {
        (Some(("Windows", "x86_64")), ("Standalone", "Win64", "x86_64"))
    } else if rel.contains("/Linux/") {
        (Some(("Linux", "x86_64")), ("Standalone", "Linux64", "x86_64"))
    } else if rel.contains("/macOS/") {
        (Some(("OSX", "AnyCPU")), ("Standalone", "OSXUniversal", "AnyCPU"))
    } else if rel.contains("/Android/") {
        (None, ("Android", "Android", "ARM64"))
    } else {
        (None, ("iPhone", "iOS", "AnyCPU"))
    };
    let mut s = String::from(
        "PluginImporter:\n  externalObjects: {}\n  serializedVersion: 2\n  iconMap: {}\n  executionOrder: {}\n  defineConstraints: []\n  isPreloaded: 0\n  isOverridable: 0\n  isExplicitlyReferenced: 0\n  validateReferences: 1\n  platformData:\n  - first:\n      Any: \n    second:\n      enabled: 0\n      settings: {}\n",
    );
    match editor {
        Some((os, cpu)) => s.push_str(&format!(
            "  - first:\n      Editor: Editor\n    second:\n      enabled: 1\n      settings:\n        CPU: {cpu}\n        DefaultValueInitialized: true\n        OS: {os}\n"
        )),
        None => s.push_str(
            "  - first:\n      Editor: Editor\n    second:\n      enabled: 0\n      settings:\n        DefaultValueInitialized: true\n",
        ),
    }
    let (key, value, cpu) = player;
    s.push_str(&format!(
        "  - first:\n      {key}: {value}\n    second:\n      enabled: 1\n      settings:\n        CPU: {cpu}\n"
    ));
    s.push_str("  userData: \n  assetBundleName: \n  assetBundleVariant: \n");
    s
}

/// 32 hex digits from a path: two FNV-1a hashes with different starts.
fn guid(rel: &str) -> String {
    let key = format!("org.ayatsuri-2d.runtime/{rel}");
    let fnv = |mut h: u64| {
        for b in key.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    };
    format!("{:016x}{:016x}", fnv(0xcbf2_9ce4_8422_2325), fnv(0x6c62_272e_07bb_0142))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_header_function_is_found() {
        let h = "uint32_t ay2d_abi_version(void);\nenum ay2d_status ay2d_canvas(const struct ay2d_model *m,\n  struct x *out);\nstruct ay2d_model *ay2d_load(const uint8_t *bytes, size_t len);\n#define AY2D_ABI_VERSION 1\n";
        let f: Vec<_> = functions(h).into_iter().collect();
        assert_eq!(f, ["ay2d_abi_version", "ay2d_canvas", "ay2d_load"]);
    }

    #[test]
    fn guids_are_stable_and_distinct() {
        assert_eq!(guid("Runtime/Native.cs"), guid("Runtime/Native.cs"));
        assert_ne!(guid("Runtime/Native.cs"), guid("Runtime/ModelMesh.cs"));
        assert_eq!(guid("Runtime").len(), 32);
    }
}
