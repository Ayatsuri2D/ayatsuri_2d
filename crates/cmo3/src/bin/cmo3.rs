//! Command-line front end for poking at `.cmo3` files.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cmo3::caff::{self, TAG_MAIN_XML};
use cmo3::model::Value;
use cmo3::version::Cmo3TargetVersion;
use cmo3::{Cmo3Model, Result, Typed};

const USAGE: &str = "\
usage: cmo3 <command> [args]

  dump <file.cmo3>                 header, file table, version, root overview, pool size
  xml <file.cmo3>                  stored main.xml, decompressed, to stdout
  render <file.cmo3>               main.xml re-emitted from the parsed graph, to stdout
  extract <file.cmo3> <dir>        unpack every entry (main.xml plaintext + PNGs) into <dir>
  roundtrip <file.cmo3> [out]      parse + re-emit; report whether main.xml is byte-identical
  convert <in.cmo3> <out.cmo3>     resave through the graph (overwrites <out> without asking)
  images <file.cmo3>               list CImageResource entries (path, size, dimensions)
  tags <file.cmo3>                 count element tags in main.xml
  set-target <in> <out> <no>       set CModelSource.targetVersionNo and resave
  schema <files...> [--rust out.rs] infer the class schema from files; optionally emit typed_gen.rs
  params <file.cmo3>               list parameters through the typed layer
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let arg = |i: usize| -> Result<&str> {
        args.get(i).map(String::as_str).ok_or_else(|| cmo3::Error::new(USAGE))
    };
    match cmd {
        "dump" => dump(Path::new(arg(1)?)),
        "xml" => {
            let bytes = std::fs::read(arg(1)?)?;
            let entry = caff::read_first_entry_by_tag(&bytes, TAG_MAIN_XML)?
                .ok_or_else(|| cmo3::Error::new("no main_xml entry"))?;
            use std::io::Write;
            std::io::stdout().write_all(&entry.content)?;
            Ok(())
        }
        "render" => {
            let model = Cmo3Model::read_file(arg(1)?)?;
            use std::io::Write;
            std::io::stdout().write_all(&model.render_main_xml()?)?;
            Ok(())
        }
        "extract" => extract(Path::new(arg(1)?), Path::new(arg(2)?)),
        "roundtrip" => roundtrip(Path::new(arg(1)?), args.get(2).map(PathBuf::from)),
        "convert" => {
            let model = Cmo3Model::read_file(arg(1)?)?;
            model.write_file(arg(2)?)?;
            println!("wrote {}", arg(2)?);
            Ok(())
        }
        "images" => images(Path::new(arg(1)?)),
        "tags" => tags(Path::new(arg(1)?)),
        "schema" => {
            let mut files: Vec<&str> = Vec::new();
            let mut rust_out: Option<&str> = None;
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--rust" {
                    rust_out = args.get(i + 1).map(String::as_str);
                    i += 2;
                } else {
                    files.push(&args[i]);
                    i += 1;
                }
            }
            if files.is_empty() {
                return Err(cmo3::Error::new(USAGE));
            }
            let mut schema = cmo3::schema::Schema::default();
            for f in &files {
                let model = Cmo3Model::read_file(f)?;
                schema.absorb(&model.graph);
                eprintln!("absorbed {f}: {} classes so far", schema.classes.len());
            }
            match rust_out {
                Some(path) => {
                    std::fs::write(path, cmo3::schema::emit_rust(&schema))?;
                    println!("wrote {path} ({} classes)", schema.classes.len());
                }
                None => print!("{}", schema.describe()),
            }
            Ok(())
        }
        "params" => {
            use cmo3::typed_gen::CModelSource;
            let model = Cmo3Model::read_file(arg(1)?)?;
            let root = model
                .root()
                .and_then(|r| CModelSource::try_from_ref(&r))
                .ok_or_else(|| cmo3::Error::new("root is not a CModelSource"))?;
            let set = root
                .parameter_source_set()
                .ok_or_else(|| cmo3::Error::new("no parameterSourceSet"))?;
            for p in set.sources() {
                println!(
                    "{:<24} {:<20} [{} .. {}] default={} repeat={} type={}",
                    p.id().map(|i| i.idstr()).unwrap_or_default(),
                    p.name().unwrap_or_default(),
                    p.min_value().unwrap_or(f32::NAN),
                    p.max_value().unwrap_or(f32::NAN),
                    p.default_value().unwrap_or(f32::NAN),
                    p.is_repeat().unwrap_or(false),
                    p.param_type().and_then(|t| t.constant()).unwrap_or_default(),
                );
            }
            Ok(())
        }
        "set-target" => {
            let mut model = Cmo3Model::read_file(arg(1)?)?;
            let no: i32 =
                arg(3)?.parse().map_err(|_| cmo3::Error::new("versionNo must be an integer"))?;
            model.set_target_version_no(no)?;
            model.write_file(arg(2)?)?;
            println!("targetVersionNo={no} written to {}", arg(2)?);
            Ok(())
        }
        _ => {
            eprint!("{USAGE}");
            Err(cmo3::Error::new("unknown or missing command"))
        }
    }
}

fn dump(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path)?;
    let archive = caff::read(&bytes)?;
    println!("file: {} ({} bytes)", path.display(), bytes.len());
    println!(
        "CAFF ident={} archiveVersion={:?} formatVersion={:?} key={} (0x{:08X}) preview={:?}",
        archive.format_identifier,
        archive.archive_version,
        archive.format_version,
        archive.obfuscate_key,
        archive.obfuscate_key as u32,
        archive.preview.image_type
    );
    println!(
        "{:>4} {:<28} {:<10} {:>10} {:>10} {:<5} compression",
        "#", "path", "tag", "offset", "stored", "obf"
    );
    for (i, (p, t, off, size, obf, comp)) in caff::list_table(&bytes)?.into_iter().enumerate() {
        println!("{i:>4} {p:<28} {t:<10} {off:>10} {size:>10} {obf:<5} {comp:?}");
    }
    let model = Cmo3Model::read_archive(archive)?;
    let main_len = model.stored_main_xml().map(<[u8]>::len).unwrap_or(0);
    println!();
    println!("main.xml: {main_len} bytes decompressed");
    println!(
        "fileFormatVersion: {} ({:?})",
        model.file_format_version().unwrap_or("?"),
        Cmo3Model::get_version(&bytes)
    );
    let target = model.target_version_no();
    println!(
        "targetVersionNo: {:?} -> {}",
        target,
        match target {
            Some(Cmo3TargetVersion::LATEST_VERSION_NO) => "SDK (N/A) / latest".to_string(),
            other => Cmo3TargetVersion::from_version_no(other)
                .map(|t| t.to_string())
                .unwrap_or_else(|| "unknown".into()),
        }
    );
    println!("versions: {:?}", model.graph.versions());
    println!("imports: {}", model.graph.imports().len());
    println!("shared pool: {} entries", model.graph.shared.len());
    if let Some(root) = model.root() {
        let r = root.borrow();
        println!("root: <{}> attrs={:?}", r.tag, r.attrs);
        for f in &r.fields {
            println!("  {:<32} {}", f.name.as_deref().unwrap_or("(item)"), describe(&f.value));
        }
    }
    if !model.graph.diagnostics.is_empty() {
        println!("diagnostics ({}):", model.graph.diagnostics.len());
        for d in model.graph.diagnostics.iter().take(20) {
            println!("  {d}");
        }
    }
    Ok(())
}

fn describe(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Object(o) => {
            let o = o.borrow();
            match o.enum_constant() {
                Some(c) => format!("<{}> {c}", o.tag),
                None => format!("<{}> {} attrs, {} fields", o.tag, o.attrs.len(), o.fields.len()),
            }
        }
        Value::List { kind, items } => format!("{} [{}]", kind.tag(), items.len()),
        Value::Set(items) => format!("linked_set [{}]", items.len()),
        Value::Map { kind, entries, .. } => format!("{} {{{}}}", kind.tag(), entries.len()),
        Value::Str(s) => format!("\"{s}\""),
        Value::Shared(s) => format!("shared {}", describe(&s.borrow())),
        Value::File { path, text } => format!("file {:?}", path.as_deref().or(text.as_deref())),
        Value::Raw(e) => format!("raw <{}>", e.name),
        Value::DanglingRef { tag, id } => format!("dangling <{tag} xs.ref={id}>"),
        other => {
            let text = cmo3::model::scalar_text(other, cmo3::xml::FloatStyle::Java);
            if text.len() > 60 {
                format!("{} ({} chars)", other.tag(), text.len())
            } else {
                format!("{} {text}", other.tag())
            }
        }
    }
}

fn extract(path: &Path, dir: &Path) -> Result<()> {
    let archive = caff::read(&std::fs::read(path)?)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".into());
    let out_dir = dir.join(stem);
    std::fs::create_dir_all(&out_dir)?;
    for entry in &archive.entries {
        let name =
            if entry.path.is_empty() { format!("{}.bin", entry.tag) } else { entry.path.clone() };
        let name = name.replace(['/', '\\'], "_");
        std::fs::write(out_dir.join(&name), &entry.content)?;
        println!("{:<28} {:>10} bytes", name, entry.content.len());
    }
    println!("extracted {} entries to {}", archive.entries.len(), out_dir.display());
    Ok(())
}

fn roundtrip(path: &Path, out: Option<PathBuf>) -> Result<()> {
    let model = Cmo3Model::read_file(path)?;
    let original = model.stored_main_xml().unwrap_or(&[]);
    let rendered = model.render_main_xml()?;
    if original == rendered.as_slice() {
        println!("main.xml round-trips byte-identical ({} bytes)", original.len());
    } else {
        let first_diff = original
            .iter()
            .zip(rendered.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(original.len().min(rendered.len()));
        println!(
            "main.xml DIFFERS: original {} bytes, rendered {} bytes, first difference at offset {first_diff}",
            original.len(),
            rendered.len()
        );
        let ctx = |b: &[u8]| {
            let s = first_diff.saturating_sub(80);
            let e = (first_diff + 80).min(b.len());
            String::from_utf8_lossy(&b[s..e]).replace("\r\n", "\\r\\n\n")
        };
        println!("--- original\n{}\n--- rendered\n{}", ctx(original), ctx(&rendered));
    }
    if !model.graph.diagnostics.is_empty() {
        println!("{} diagnostics (run `dump` to list)", model.graph.diagnostics.len());
    }
    if let Some(out) = out {
        model.write_file(&out)?;
        let back = Cmo3Model::read_file(&out)?;
        println!("wrote {} ({} entries re-read OK)", out.display(), back.archive.entries.len());
    }
    Ok(())
}

fn images(path: &Path) -> Result<()> {
    let model = Cmo3Model::read_file(path)?;
    let resources = model.image_resources();
    println!("{} CImageResource(s)", resources.len());
    for r in &resources {
        let o = r.borrow();
        let p = Cmo3Model::image_file_buf_path(r).unwrap_or_default();
        let stored = model.archive.by_path(&p).map(|e| e.content.len()).unwrap_or(0);
        println!(
            "{:<28} {:>5}x{:<5} type={:<4} imageFileBuf_size={:<9} stored={}",
            p,
            o.attr("width").unwrap_or("?"),
            o.attr("height").unwrap_or("?"),
            o.attr("type").unwrap_or("?"),
            o.attr("imageFileBuf_size").unwrap_or("?"),
            stored
        );
    }
    Ok(())
}

fn tags(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path)?;
    let entry = caff::read_first_entry_by_tag(&bytes, TAG_MAIN_XML)?
        .ok_or_else(|| cmo3::Error::new("no main_xml entry"))?;
    let doc = cmo3::xml::parse(&entry.content)?;
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    fn walk(el: &cmo3::xml::Element, counts: &mut std::collections::BTreeMap<String, usize>) {
        *counts.entry(el.name.clone()).or_default() += 1;
        for c in el.child_elements() {
            walk(c, counts);
        }
    }
    walk(&doc.root, &mut counts);
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (tag, n) in rows {
        println!("{n:>8} {tag}");
    }
    Ok(())
}
