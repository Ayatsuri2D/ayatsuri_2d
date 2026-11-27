//! The decisions behind the drawing, with no Godot type in sight so they
//! can be tested on their own: how a model unit becomes a pixel, which
//! blend a drawable wants, what a frame's draw order means for the canvas
//! items already made, and what a `model3.json` bundle names.

use std::collections::HashSet;

use ayatsuri_capi::{
    AY2D_CHANGED_OPACITY, AY2D_CHANGED_ORDER, AY2D_CHANGED_POSITIONS, AY2D_CHANGED_VISIBILITY,
    AY2D_FLAG_ADDITIVE, AY2D_FLAG_INVERTED_MASK, AY2D_FLAG_MULTIPLICATIVE, Canvas,
};

// ---- units -------------------------------------------------------------

/// Model units to the node's pixels. The model's origin (the rig's `0, 0`,
/// most often the middle of its canvas) sits on the node, as it sits on
/// the GameObject in the Unity package, so a point is `unit *
/// pixels_per_unit`. The model's y points down, as a Godot 2D canvas does,
/// so nothing flips here; the Unity package negates y because Unity's
/// points up. Texture rows are read top-down on both sides, so a uv passes
/// through as it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pixels {
    pub per_unit: f32,
    /// Where the origin sits on the canvas, in pixels from its top left.
    pub origin_x: f32,
    pub origin_y: f32,
    pub width: f32,
    pub height: f32,
}

impl Pixels {
    pub fn of(canvas: &Canvas) -> Self {
        Self {
            // a zero scale would fold the model onto one point; the
            // runtime never writes one, a hand-made file might
            per_unit: if canvas.pixels_per_unit.abs() < 1e-6 {
                1.0
            } else {
                canvas.pixels_per_unit
            },
            origin_x: canvas.origin_x,
            origin_y: canvas.origin_y,
            width: canvas.width,
            height: canvas.height,
        }
    }

    /// One model-unit point as pixels on the node.
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.per_unit, y * self.per_unit)
    }

    /// The canvas on the node: its top left corner (x, y) and its size
    /// (width, height), in pixels. The origin is on the node, so the
    /// corner is at minus the origin.
    pub fn canvas_rect(&self) -> (f32, f32, f32, f32) {
        (-self.origin_x, -self.origin_y, self.width, self.height)
    }
}

// ---- blends ------------------------------------------------------------

/// What a canvas item material can do with a drawable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Mix,
    Add,
    Mul,
}

/// A drawable's blend from its flags, and whether the mesh's moc3 5.3
/// blend word asks for something the canvas cannot do (the advanced
/// colour blends, or an alpha blend other than Over), which then falls
/// back to the flags' answer.
pub fn blend_of(flags: u32, blend_word: u32) -> (Blend, bool) {
    let blend = if flags & AY2D_FLAG_ADDITIVE != 0 {
        Blend::Add
    } else if flags & AY2D_FLAG_MULTIPLICATIVE != 0 {
        Blend::Mul
    } else {
        Blend::Mix
    };
    let colour = blend_word & 0xff;
    let alpha = (blend_word >> 8) & 0xff;
    let plain = matches!(colour, 0 | 1 | 2 | 3 | 6);
    (blend, !plain || alpha != 0)
}

/// 1 when the drawable's masks cut it out where they are drawn.
pub fn inverted_mask(flags: u32) -> bool {
    flags & AY2D_FLAG_INVERTED_MASK != 0
}

// ---- the frame's draw order ---------------------------------------------

/// What one update asks of the canvas items, worked out from the meshes
/// drawn (back to front) and the runtime's change bits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// The draw order changed: every drawn mesh takes a new draw index.
    pub reorder: bool,
    /// Meshes whose triangles must be submitted again this frame, in draw
    /// order: their positions or opacity moved, they came into view, one
    /// of their masks moved, or everything is new.
    pub redraw: Vec<u32>,
    /// Meshes that have an item but left the drawables: hide them.
    pub hide: Vec<u32>,
}

/// `drawn` is this frame's meshes back to front; `masks_of` gives a mesh's
/// masks; `changes` the `AY2D_CHANGED_*` bits by mesh; `known` the meshes
/// that already have a canvas item, in any order; `last_order` the meshes
/// drawn last frame, back to front.
pub fn plan(
    drawn: &[u32],
    masks_of: impl Fn(u32) -> Vec<u32>,
    changes: &[u32],
    known: &[u32],
    last_order: &[u32],
) -> Plan {
    let known: HashSet<u32> = known.iter().copied().collect();
    let drawn_set: HashSet<u32> = drawn.iter().copied().collect();
    let bits = |m: u32| changes.get(m as usize).copied().unwrap_or(u32::MAX);
    let reorder = drawn != last_order || drawn.iter().any(|&m| bits(m) & AY2D_CHANGED_ORDER != 0);
    let moved = |m: u32| bits(m) & (AY2D_CHANGED_POSITIONS | AY2D_CHANGED_OPACITY) != 0;
    let redraw = drawn
        .iter()
        .copied()
        .filter(|&m| {
            !known.contains(&m)
                || moved(m)
                || bits(m) & AY2D_CHANGED_VISIBILITY != 0
                || masks_of(m).iter().any(|&k| bits(k) & AY2D_CHANGED_POSITIONS != 0)
        })
        .collect();
    let mut hide: Vec<u32> = known.iter().copied().filter(|m| !drawn_set.contains(m)).collect();
    hide.sort_unstable();
    Plan { reorder, redraw, hide }
}

// ---- a moc3 bundle -------------------------------------------------------

/// What a `model3.json` names, as paths relative to its own folder.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bundle {
    pub moc: String,
    pub textures: Vec<String>,
    pub physics: Option<String>,
    /// (name, group, file) per motion; the name is the file's stem.
    pub motions: Vec<(String, String, String)>,
    /// (name, file) per expression; the name is the file's `Name`, else
    /// its stem.
    pub expressions: Vec<(String, String)>,
}

/// Read a `model3.json`. The texture pages are listed for completeness;
/// the runtime reads them from the `.moc3` bundle's own PNG files.
pub fn parse_model3(text: &str) -> Result<Bundle, String> {
    let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let refs = root.get("FileReferences").ok_or("no FileReferences")?;
    let moc = refs.get("Moc").and_then(|m| m.as_str()).ok_or("no Moc")?.to_string();
    let textures = refs
        .get("Textures")
        .and_then(|t| t.as_array())
        .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let physics = refs.get("Physics").and_then(|p| p.as_str()).map(str::to_string);
    let mut names = Vec::new();
    let mut motions = Vec::new();
    if let Some(groups) = refs.get("Motions").and_then(|m| m.as_object()) {
        for (group, list) in groups {
            for m in list.as_array().into_iter().flatten() {
                let Some(file) = m.get("File").and_then(|f| f.as_str()) else { continue };
                let name = unique(&mut names, clip_name(file, ".motion3.json"));
                motions.push((name, group.clone(), file.to_string()));
            }
        }
    }
    names.clear();
    let mut expressions = Vec::new();
    for e in refs.get("Expressions").and_then(|e| e.as_array()).into_iter().flatten() {
        let Some(file) = e.get("File").and_then(|f| f.as_str()) else { continue };
        let name = e
            .get("Name")
            .and_then(|n| n.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| clip_name(file, ".exp3.json"));
        expressions.push((unique(&mut names, name), file.to_string()));
    }
    Ok(Bundle { moc, textures, physics, motions, expressions })
}

/// A clip's name from its file: `motions/idle.motion3.json` is `idle`.
fn clip_name(file: &str, ext: &str) -> String {
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let lower = base.to_lowercase();
    if lower.ends_with(ext) {
        base.get(..base.len() - ext.len()).unwrap_or(base).to_string()
    } else {
        base.rsplit_once('.').map_or(base, |(stem, _)| stem).to_string()
    }
}

/// `name`, or `name_2`, `name_3`... when it was taken.
fn unique(taken: &mut Vec<String>, name: String) -> String {
    let mut n = name.clone();
    let mut i = 2;
    while taken.contains(&n) {
        n = format!("{name}_{i}");
        i += 1;
    }
    taken.push(n.clone());
    n
}

/// `rel` resolved against the folder `dir` of a `model3.json`, with `.`
/// and `..` folded, as a Godot path (`res://models/x/x.moc3`). The scheme
/// of `dir` is kept; `..` never climbs past it.
pub fn join(dir: &str, rel: &str) -> String {
    let (scheme, rest) = match dir.find("://") {
        Some(i) => dir.split_at(i + 3),
        None if dir.starts_with('/') => ("/", dir.get(1..).unwrap_or("")),
        None => ("", dir),
    };
    let mut parts: Vec<&str> = Vec::new();
    for s in rest.split(['/', '\\']).chain(rel.split(['/', '\\'])) {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(s),
        }
    }
    format!("{scheme}{}", parts.join("/"))
}

/// The folder a file sits in, scheme kept: `res://a/b.json` is `res://a`.
pub fn dir_of(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(i) => path.get(..i).unwrap_or(path),
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        Canvas {
            width: 1024.0,
            height: 768.0,
            pixels_per_unit: 512.0,
            origin_x: 512.0,
            origin_y: 384.0,
        }
    }

    #[test]
    fn units_become_pixels_without_a_flip() {
        let px = Pixels::of(&canvas());
        assert_eq!(px.point(0.0, 0.0), (0.0, 0.0));
        // one unit right and one unit down the model is 512 pixels right
        // and down on the node
        assert_eq!(px.point(1.0, 0.75), (512.0, 384.0));
        assert_eq!(px.point(-1.0, -0.75), (-512.0, -384.0));
        // the canvas's corners are the points above: the origin is its
        // middle here, so it spans minus the origin to the size past it
        assert_eq!(px.canvas_rect(), (-512.0, -384.0, 1024.0, 768.0));
    }

    #[test]
    fn a_zero_scale_is_not_used() {
        let px = Pixels::of(&Canvas { pixels_per_unit: 0.0, ..canvas() });
        assert_eq!(px.point(2.0, 3.0), (2.0, 3.0));
    }

    #[test]
    fn blends_follow_the_flags_and_notice_advanced_codes() {
        assert_eq!(blend_of(0, 0), (Blend::Mix, false));
        assert_eq!(blend_of(AY2D_FLAG_ADDITIVE, 3), (Blend::Add, false));
        assert_eq!(blend_of(AY2D_FLAG_MULTIPLICATIVE, 6), (Blend::Mul, false));
        // Screen (10) has no canvas blend; the flags say Normal
        assert_eq!(blend_of(0, 10), (Blend::Mix, true));
        // an Atop alpha blend over a plain Add
        assert_eq!(blend_of(AY2D_FLAG_ADDITIVE, 3 | (1 << 8)), (Blend::Add, true));
        assert!(inverted_mask(AY2D_FLAG_INVERTED_MASK));
        assert!(!inverted_mask(AY2D_FLAG_ADDITIVE));
    }

    const ALL: u32 = AY2D_CHANGED_POSITIONS
        | AY2D_CHANGED_VISIBILITY
        | AY2D_CHANGED_OPACITY
        | AY2D_CHANGED_ORDER;

    #[test]
    fn the_first_frame_draws_everything() {
        let p = plan(&[2, 0, 1], |_| vec![], &[ALL, ALL, ALL], &[], &[]);
        assert_eq!(p, Plan { reorder: true, redraw: vec![2, 0, 1], hide: vec![] });
    }

    #[test]
    fn a_still_frame_asks_for_nothing() {
        let p = plan(&[0, 1], |_| vec![], &[0, 0], &[0, 1], &[0, 1]);
        assert_eq!(p, Plan::default());
    }

    #[test]
    fn moved_meshes_are_redrawn_in_draw_order() {
        let p = plan(
            &[1, 0],
            |_| vec![],
            &[AY2D_CHANGED_POSITIONS, AY2D_CHANGED_OPACITY],
            &[0, 1],
            &[1, 0],
        );
        assert_eq!(p, Plan { reorder: false, redraw: vec![1, 0], hide: vec![] });
    }

    #[test]
    fn a_changed_order_reorders_without_redrawing_still_meshes() {
        let p =
            plan(&[1, 0], |_| vec![], &[AY2D_CHANGED_ORDER, AY2D_CHANGED_ORDER], &[0, 1], &[0, 1]);
        assert_eq!(p, Plan { reorder: true, redraw: vec![], hide: vec![] });
        // the order bits may lag a frame behind a host that missed one:
        // a different sequence alone is enough
        let p = plan(&[1, 0], |_| vec![], &[0, 0], &[0, 1], &[0, 1]);
        assert!(p.reorder);
    }

    #[test]
    fn a_mesh_that_leaves_is_hidden_and_one_that_returns_is_redrawn() {
        let p = plan(&[0], |_| vec![], &[0, AY2D_CHANGED_VISIBILITY], &[0, 1], &[0, 1]);
        assert_eq!(p, Plan { reorder: true, redraw: vec![], hide: vec![1] });
        let p = plan(&[0, 1], |_| vec![], &[0, AY2D_CHANGED_VISIBILITY], &[0, 1], &[0]);
        assert_eq!(p, Plan { reorder: true, redraw: vec![1], hide: vec![] });
    }

    #[test]
    fn a_moving_mask_redraws_the_mesh_it_clips() {
        // mesh 0 is clipped by mesh 5, which moved; mesh 0 itself did not
        let p = plan(
            &[0],
            |m| if m == 0 { vec![5] } else { vec![] },
            &[0, 0, 0, 0, 0, AY2D_CHANGED_POSITIONS],
            &[0],
            &[0],
        );
        assert_eq!(p.redraw, vec![0]);
    }

    #[test]
    fn a_model3_lists_its_files() {
        let text = r#"{
          "Version": 3,
          "FileReferences": {
            "Moc": "hiyori.moc3",
            "Textures": ["hiyori.2048/texture_00.png"],
            "Physics": "hiyori.physics3.json",
            "Motions": {
              "Idle": [{"File": "motions/idle_01.motion3.json"}, {"File": "motions/idle_01.motion3.json"}],
              "TapBody": [{"File": "motions/tap.motion3.json", "FadeInTime": 0.5}]
            },
            "Expressions": [{"Name": "smile", "File": "exp/f01.exp3.json"}, {"File": "exp/f02.exp3.json"}]
          }
        }"#;
        let b = parse_model3(text).unwrap_or_default();
        assert_eq!(b.moc, "hiyori.moc3");
        assert_eq!(b.textures, vec!["hiyori.2048/texture_00.png"]);
        assert_eq!(b.physics.as_deref(), Some("hiyori.physics3.json"));
        let names: Vec<&str> = b.motions.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, vec!["idle_01", "idle_01_2", "tap"]);
        assert_eq!(b.motions.get(2).map(|m| m.1.as_str()), Some("TapBody"));
        assert_eq!(
            b.expressions,
            vec![
                ("smile".into(), "exp/f01.exp3.json".into()),
                ("f02".into(), "exp/f02.exp3.json".into())
            ]
        );
    }

    #[test]
    fn a_model3_without_a_moc_is_refused() {
        assert!(parse_model3(r#"{"FileReferences": {}}"#).is_err());
        assert!(parse_model3("not json").is_err());
    }

    #[test]
    fn bundle_paths_resolve_beside_the_model3() {
        assert_eq!(dir_of("res://models/hiyori/hiyori.model3.json"), "res://models/hiyori");
        assert_eq!(
            join("res://models/hiyori", "motions/idle.motion3.json"),
            "res://models/hiyori/motions/idle.motion3.json"
        );
        assert_eq!(join("res://models/hiyori", "../shared/x.moc3"), "res://models/shared/x.moc3");
        assert_eq!(join("res://models", "../../x.moc3"), "res://x.moc3");
        assert_eq!(join("/home/me/model", ".\\a.moc3"), "/home/me/model/a.moc3");
        assert_eq!(join("", "a.moc3"), "a.moc3");
        assert_eq!(dir_of("a.model3.json"), "");
    }
}
