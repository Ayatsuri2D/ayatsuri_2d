// The embedded export: everything a Cubism-compatible runtime loads, laid
// out the way those runtimes expect. The .aom keeps all of this inside one
// file; this is the spread-out form for other people's players.
//
//     <dir>/<name>.moc3
//     <dir>/<name>.model3.json
//     <dir>/<name>.physics3.json      when the model has physics
//     <dir>/<name>.cdi3.json          display names and parameter folders
//     <dir>/<name>.textures/texture_00.png ...
//     <dir>/motions/<clip>.motion3.json

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::session::Session;

pub struct Options {
    pub name: String,
    pub version: ayatsuri_import::ExportVersion,
    pub physics: bool,
    pub motions: bool,
    pub expressions: bool,
    /// Write each texture page with its longer side this many pixels (up
    /// to 49152), resized on the way out; None writes them as they are.
    /// The model's own pages, and so the editor, keep their size.
    pub texture_size: Option<u32>,
}

/// The largest texture side the export writes.
pub const MAX_EXPORT_TEXTURE: u32 = 49152;

/// A page as export writes it: the clear pixels take the colour of the
/// nearest art (alpha stays 0), then, with `side`, its longer side `side`
/// pixels, its shape kept. Runtimes filter the texture without
/// premultiplying, so clear pixels left black darken every mesh's edge
/// into a line. UVs are fractions of the page, so they need no change.
fn page_out(png: &[u8], side: Option<u32>) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(png).map_err(|e| e.to_string())?.into_rgba8();
    crate::atlas::bleed(&mut img);
    let (w, h) = (img.width().max(1), img.height().max(1));
    if let Some(side) = side.map(|s| s.clamp(16, MAX_EXPORT_TEXTURE))
        && w.max(h) != side
    {
        let k = side as f64 / w.max(h) as f64;
        let (nw, nh) =
            (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1));
        img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::CatmullRom);
        // resampling leaves new clear pixels at the edges, black again
        crate::atlas::bleed(&mut img);
    }
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// File names keep to what every filesystem accepts.
fn safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() { "model".into() } else { cleaned }
}

/// The models already in a folder: its `*.model3.json` files.
pub fn existing_models(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.to_string_lossy().ends_with(".model3.json"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// A path a model3.json names, if it stays inside `dir` (no absolute paths,
/// no `..`): an export folder's files are the only ones ever removed.
fn inside(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    let safe = rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    (safe && !rel.as_os_str().is_empty()).then(|| dir.join(rel))
}

/// Make a folder ready for a fresh export over an earlier one: for every
/// model3.json in it, remove the files it lists (moc3, textures, physics,
/// display info, pose, user data, every motion and expression) and the
/// model3.json itself, then any folder those leave empty. Files it does not
/// list stay: VTube Studio's settings, items, icons, the user's own files,
/// and motion sounds, which a fresh export does not write. Returns how many
/// files went.
pub fn clear_previous(dir: &Path) -> Result<usize, String> {
    let mut removed = 0;
    let mut folders: Vec<PathBuf> = Vec::new();
    for model in existing_models(dir) {
        let text =
            std::fs::read_to_string(&model).map_err(|e| format!("{}: {e}", model.display()))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", model.display()))?;
        let none = serde_json::Value::Null;
        let refs = json.get("FileReferences").unwrap_or(&none);
        let mut listed: Vec<String> = Vec::new();
        for key in ["Moc", "Physics", "DisplayInfo", "Pose", "UserData"] {
            if let Some(f) = refs.get(key).and_then(|v| v.as_str()) {
                listed.push(f.to_string());
            }
        }
        for t in refs.get("Textures").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(f) = t.as_str() {
                listed.push(f.to_string());
            }
        }
        for group in
            refs.get("Motions").and_then(|v| v.as_object()).into_iter().flat_map(|o| o.values())
        {
            for m in group.as_array().into_iter().flatten() {
                if let Some(f) = m.get("File").and_then(|v| v.as_str()) {
                    listed.push(f.to_string());
                }
            }
        }
        for e in refs.get("Expressions").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(f) = e.get("File").and_then(|v| v.as_str()) {
                listed.push(f.to_string());
            }
        }
        for rel in listed {
            let Some(path) = inside(dir, &rel) else { continue };
            if !path.is_file() {
                continue;
            }
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("could not remove {} of the previous export: {e}", path.display());
            } else {
                removed += 1;
                if let Some(parent) = path.parent()
                    && parent != dir
                    && !folders.iter().any(|f| f == parent)
                {
                    folders.push(parent.to_path_buf());
                }
            }
        }
        std::fs::remove_file(&model).map_err(|e| format!("{}: {e}", model.display()))?;
        removed += 1;
    }
    // folders the old export made, deepest first, once they are empty
    folders.sort_by_key(|f| std::cmp::Reverse(f.components().count()));
    for f in folders {
        // a folder that still holds the user's own files stays
        if let Err(e) = std::fs::remove_dir(&f) {
            log::debug!("kept {}: {e}", f.display());
        }
    }
    Ok(removed)
}

/// The files an export writes, worked out from the session (`prepare`) and
/// written apart from it (`Job::run`), so the slow part, the texture pages,
/// can run on its own thread while the editor keeps drawing.
pub struct Job {
    files: Vec<(PathBuf, Out)>,
}

enum Out {
    Bytes(Vec<u8>),
    /// A texture page as stored, and the size it goes out at.
    Page(Vec<u8>, Option<u32>),
}

impl Job {
    fn add(&mut self, path: PathBuf, bytes: &[u8]) {
        self.files.push((path, Out::Bytes(bytes.to_vec())));
    }

    /// Write every file, in order (the model3.json last, so an app watching
    /// the folder finds the rest already there). The paths written.
    pub fn run(self) -> Result<Vec<PathBuf>, String> {
        // the pages are the slow part: each on a thread of its own
        let pages: Vec<Option<Result<Vec<u8>, String>>> = std::thread::scope(|sc| {
            let running: Vec<_> = self
                .files
                .iter()
                .map(|(_, out)| match out {
                    Out::Page(png, side) => Some(sc.spawn(move || page_out(png, *side))),
                    Out::Bytes(_) => None,
                })
                .collect();
            running
                .into_iter()
                .map(|h| h.map(|h| h.join().unwrap_or_else(|_| Err("page failed".into()))))
                .collect()
        });
        let mut written = Vec::new();
        for ((path, out), page) in self.files.iter().zip(pages) {
            let bytes = match (out, page) {
                (Out::Bytes(b), _) => b.clone(),
                (Out::Page(..), Some(p)) => p?,
                (Out::Page(..), None) => continue,
            };
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            written.push(path.clone());
        }
        Ok(written)
    }
}

/// Prepare and write in one go (the editor runs the two apart).
#[cfg(test)]
pub fn export(s: &Session, dir: &Path, opt: &Options) -> Result<Vec<PathBuf>, String> {
    prepare(s, dir, opt)?.run()
}

/// Everything an export needs from the session, as files still to write.
pub fn prepare(s: &Session, dir: &Path, opt: &Options) -> Result<Job, String> {
    let name = safe(&opt.name);
    // inherited clips go out as hidden helper masks, on copies
    // pages no mesh draws from (kept for undo while the model is open)
    // stay behind
    let mut base = s.puppet.model().clone();
    // a mesh drawn flipped would be culled by Cubism's runtimes: it goes
    // out double-sided, as it shows in the editor
    for i in ayatsuri_core::edit::culled_meshes(&base) {
        if let Some(m) = base.art_meshes.get_mut(i) {
            m.flags |= ayatsuri_format::model::flags::DOUBLE_SIDED;
        }
    }
    let (base_pngs, _) = crate::session::prune_pages(&mut base, s.texture_pngs.clone());
    let expanded = crate::inherit_export::expand(&base, &base_pngs)?;
    if let Some(e) = &expanded {
        log::info!(
            "export: {} inherited clip(s) carried by {} helper mask(s)",
            e.meshes,
            e.helpers
        );
    }
    let model = expanded.as_ref().map_or(&base, |e| &e.model);
    let pngs = expanded.as_ref().map_or(&base_pngs, |e| &e.pngs);
    let mut job = Job { files: Vec::new() };

    let moc = ayatsuri_import::to_moc3(model, opt.version).map_err(|e| e.to_string())?;
    job.add(dir.join(format!("{name}.moc3")), &moc);

    // textures, in slot order
    let tex_dir_name = format!("{name}.textures");
    let tex_dir = dir.join(&tex_dir_name);
    let mut tex_refs = Vec::new();
    for (i, png) in pngs.iter().enumerate() {
        let file = format!("texture_{i:02}.png");
        job.files.push((tex_dir.join(&file), Out::Page(png.clone(), opt.texture_size)));
        tex_refs.push(format!("{tex_dir_name}/{file}"));
    }

    let mut refs = serde_json::Map::new();
    refs.insert("Moc".into(), json!(format!("{name}.moc3")));
    refs.insert("Textures".into(), json!(tex_refs));

    if opt.physics
        && let Some(rig) = &model.physics
    {
        let file = format!("{name}.physics3.json");
        job.add(dir.join(&file), &ayatsuri_import::to_physics3(rig));
        refs.insert("Physics".into(), json!(file));
    }

    // display info, so other editors show the names riggers gave things
    let cdi = json!({
        "Version": 3,
        "Parameters": model.parameters.iter().enumerate().map(|(i, p)| {
            // loose parameters (outside folders) have no group: only
            // named folders are written as groups below
            let group = s
                .groups
                .iter()
                .position(|g| g.params.contains(&i) && !g.name.is_empty())
                .map(|gi| format!("ParamGroup{gi}"))
                .unwrap_or_default();
            json!({
                "Id": p.id,
                "GroupId": group,
                "Name": s.param_names.get(i).cloned().unwrap_or_else(|| p.id.clone()),
            })
        }).collect::<Vec<_>>(),
        "ParameterGroups": s.groups.iter().enumerate().filter(|(_, g)| !g.name.is_empty()).map(|(gi, g)| json!({
            "Id": format!("ParamGroup{gi}"),
            "GroupId": "",
            "Name": g.name,
        })).collect::<Vec<_>>(),
        "Parts": model.parts.iter().enumerate().map(|(i, p)| json!({
            "Id": p.id,
            "Name": s.part_names.get(i).cloned().unwrap_or_else(|| p.id.clone()),
        })).collect::<Vec<_>>(),
        "CombinedParameters": s.combined.iter().filter_map(|&(a, b)| {
            Some(json!([model.parameters.get(a)?.id, model.parameters.get(b)?.id]))
        }).collect::<Vec<_>>(),
    });
    let cdi_file = format!("{name}.cdi3.json");
    job.add(dir.join(&cdi_file), &ayatsuri_import::cubism_json(&cdi));
    refs.insert("DisplayInfo".into(), json!(cdi_file));

    // expressions go in their own folder, as published models have them, and
    // are listed before the motions in the model3.json, Cubism's order
    if opt.expressions && !s.expressions.is_empty() {
        let edir = dir.join("expressions");
        let mut list = Vec::new();
        let mut used: Vec<String> = Vec::new();
        for e in &s.expressions {
            // two expressions whose names clean up the same get numbered
            let base = safe(if e.name.is_empty() { "expression" } else { &e.name });
            let mut file = format!("{base}.exp3.json");
            let mut n = 2;
            while used.contains(&file) {
                file = format!("{base}_{n}.exp3.json");
                n += 1;
            }
            used.push(file.clone());
            job.add(edir.join(&file), &ayatsuri_import::to_exp3(e));
            list.push(json!({ "Name": e.name, "File": format!("expressions/{file}") }));
        }
        refs.insert("Expressions".into(), json!(list));
    }

    if opt.motions && (!s.animations.is_empty() || !s.compositions.is_empty()) {
        let mdir = dir.join("motions");
        let mut group = Vec::new();
        for anim in &s.animations {
            let file = format!("{}.motion3.json", safe(&anim.name));
            job.add(mdir.join(&file), &ayatsuri_import::to_motion3(anim));
            group.push(json!({ "File": format!("motions/{file}") }));
        }
        // compositions go out baked: a key every frame, flat like a
        // rendered precomp
        for comp in &s.compositions {
            let baked = s.bake_comp(comp);
            let file = format!("{}.motion3.json", safe(&comp.name));
            job.add(mdir.join(&file), &ayatsuri_import::to_motion3(&baked));
            group.push(json!({ "File": format!("motions/{file}") }));
        }
        refs.insert("Motions".into(), json!({ "Idle": group }));
    }

    let groups: Vec<serde_json::Value> = model_groups(s)
        .into_iter()
        .map(|(target, name, ids)| json!({ "Target": target, "Name": name, "Ids": ids }))
        .collect();
    let model3 = json!({
        "Version": 3,
        "FileReferences": refs,
        "Groups": groups,
    });
    job.add(dir.join(format!("{name}.model3.json")), &ayatsuri_import::cubism_json(&model3));
    Ok(job)
}

/// The model3.json Groups: the ones the model came with, else the two
/// every Cubism export carries, filled from the standard parameter ids
/// when the model has them (apps drive auto blink and lip sync off these).
fn model_groups(s: &Session) -> Vec<(String, String, Vec<String>)> {
    if let Some(g) = &s.model_groups {
        return g.clone();
    }
    let has = |id: &str| s.puppet.model().parameters.iter().any(|p| p.id == id);
    let pick = |ids: &[&str]| ids.iter().filter(|id| has(id)).map(|id| id.to_string()).collect();
    vec![
        ("Parameter".into(), "EyeBlink".into(), pick(&["ParamEyeLOpen", "ParamEyeROpen"])),
        ("Parameter".into(), "LipSync".into(), pick(&["ParamMouthOpenY"])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(ids: &[&str]) -> Session {
        let mut model = ayatsuri_format::Model::new();
        for id in ids {
            model.parameters.push(ayatsuri_format::model::Parameter {
                id: (*id).into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                repeat: false,
                decimals: 3,
                blend_shape: false,
            });
        }
        Session::create(&egui::Context::default(), "t".into(), String::new(), model).unwrap()
    }

    #[test]
    fn standard_groups_fill_from_known_ids() {
        let s = session(&["ParamEyeLOpen", "ParamEyeROpen", "ParamMouthOpenY", "ParamOther"]);
        let g = model_groups(&s);
        assert_eq!(g[0].1, "EyeBlink");
        assert_eq!(g[0].2, vec!["ParamEyeLOpen", "ParamEyeROpen"]);
        assert_eq!(g[1].1, "LipSync");
        assert_eq!(g[1].2, vec!["ParamMouthOpenY"]);
        // a model without them still gets both groups, just empty
        let bare = model_groups(&session(&["ParamOther"]));
        assert!(bare.iter().all(|(_, _, ids)| ids.is_empty()) && bare.len() == 2);
    }

    #[test]
    fn expressions_export_and_read_back() {
        let mut s = session(&["ParamEyeLOpen", "ParamSmile"]);
        let smile = ayatsuri_format::Expression {
            name: "Smile".into(),
            fade_in: Some(0.5),
            fade_out: None,
            params: vec![
                ayatsuri_format::ExpressionParam {
                    id: "ParamSmile".into(),
                    value: 1.0,
                    blend: ayatsuri_format::ExpressionBlend::Add,
                },
                ayatsuri_format::ExpressionParam {
                    id: "ParamEyeLOpen".into(),
                    value: 0.5,
                    blend: ayatsuri_format::ExpressionBlend::Multiply,
                },
            ],
        };
        // a second expression with the same name still gets its own file
        s.expressions = vec![smile.clone(), smile.clone()];
        s.animations = vec![ayatsuri_format::Animation {
            name: "idle".into(),
            fps: 30.0,
            duration: 1.0,
            looped: true,
            ..ayatsuri_format::Animation::default()
        }];
        let dir = std::env::temp_dir().join(format!("ayatsuri-exp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let opt = Options {
            name: "t".into(),
            version: ayatsuri_import::ExportVersion::DEFAULT,
            physics: false,
            motions: true,
            expressions: true,
            texture_size: None,
        };
        export(&s, &dir, &opt).unwrap();
        let model3 = std::fs::read_to_string(dir.join("t.model3.json")).unwrap();
        // listed, and before Motions as Cubism orders them
        let (e, m) = (model3.find("\"Expressions\"").unwrap(), model3.find("\"Motions\"").unwrap());
        assert!(e < m, "{model3}");
        assert!(model3.contains("expressions/Smile.exp3.json"), "{model3}");
        assert!(model3.contains("expressions/Smile_2.exp3.json"), "{model3}");
        let back = ayatsuri_import::from_exp3(
            &std::fs::read(dir.join("expressions/Smile.exp3.json")).unwrap(),
            "Smile",
        )
        .unwrap();
        assert_eq!(back, smile);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imported_groups_win() {
        let mut s = session(&["ParamEyeLOpen", "ParamMouthOpenY"]);
        let original = vec![
            ("Parameter".to_string(), "EyeBlink".to_string(), vec!["ParamEyeLOpen".to_string()]),
            ("Parameter".to_string(), "LipSync".to_string(), Vec::new()),
        ];
        s.model_groups = Some(original.clone());
        assert_eq!(model_groups(&s), original);
    }
}

#[cfg(test)]
mod overwrite_tests {
    use super::*;

    #[test]
    fn clearing_removes_only_what_the_old_model_listed() {
        let dir = std::env::temp_dir().join(format!("ay-export-overwrite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["ariu.4096", "motions", "expressions"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let files = [
            "ariu.moc3",
            "ariu.physics3.json",
            "ariu.cdi3.json",
            "ariu.4096/texture_00.png",
            "motions/idle.motion3.json",
            "motions/idle.wav",
            "expressions/smile.exp3.json",
            "ariu.vtube.json",
            "items_pinned_to_model.json",
        ];
        for f in files {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        // something outside the folder that a hostile path points at
        let outside = dir.with_extension("outside");
        std::fs::write(&outside, b"keep").unwrap();
        let model = serde_json::json!({
            "Version": 3,
            "FileReferences": {
                "Moc": "ariu.moc3",
                "Textures": ["ariu.4096/texture_00.png"],
                "Physics": "ariu.physics3.json",
                "DisplayInfo": "ariu.cdi3.json",
                "UserData": format!("../{}", outside.file_name().unwrap().to_string_lossy()),
                "Motions": {"Idle": [{"File": "motions/idle.motion3.json", "Sound": "motions/idle.wav"}]},
                "Expressions": [{"Name": "smile", "File": "expressions/smile.exp3.json"}]
            }
        });
        std::fs::write(dir.join("ariu.model3.json"), model.to_string()).unwrap();
        assert_eq!(existing_models(&dir).len(), 1);

        let removed = clear_previous(&dir).unwrap();
        assert_eq!(removed, 7, "6 listed files and the model3.json");
        assert!(existing_models(&dir).is_empty());
        for gone in ["ariu.moc3", "ariu.4096", "expressions", "motions/idle.motion3.json"] {
            assert!(!dir.join(gone).exists(), "{gone} should be gone");
        }
        // not ours: VTube Studio's settings, items, the motion's sound
        for kept in ["ariu.vtube.json", "items_pinned_to_model.json", "motions/idle.wav"] {
            assert!(dir.join(kept).exists(), "{kept} should stay");
        }
        assert!(outside.exists(), "a path out of the folder is never followed");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn a_page_resizes_on_its_longer_side() {
        let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([10, 20, 30, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let out = image::load_from_memory(&page_out(&png, Some(256)).unwrap()).unwrap();
        assert_eq!((out.width(), out.height()), (256, 128));
    }

    #[test]
    fn clear_pixels_take_the_nearest_art_colour_so_edges_stay_clean() {
        // red art on the left, blue on the right, clear black between
        let mut img = image::RgbaImage::new(8, 1);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(7, 0, image::Rgba([0, 0, 255, 128]));
        crate::atlas::bleed(&mut img);
        assert_eq!(img.get_pixel(0, 0), &image::Rgba([255, 0, 0, 255]), "art untouched");
        assert_eq!(img.get_pixel(7, 0), &image::Rgba([0, 0, 255, 128]));
        assert_eq!(img.get_pixel(2, 0), &image::Rgba([255, 0, 0, 0]), "still clear");
        assert_eq!(img.get_pixel(5, 0), &image::Rgba([0, 0, 255, 0]));
        // a page with no art at all stays as it is
        let mut empty = image::RgbaImage::new(4, 4);
        crate::atlas::bleed(&mut empty);
        assert!(empty.pixels().all(|p| p.0 == [0, 0, 0, 0]));
    }
}
