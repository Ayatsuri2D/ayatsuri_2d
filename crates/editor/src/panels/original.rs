// Edit Form > Revert to Original and the original shapes behind it, after
// the Cubism manual's "Restore deformer shape": the selected meshes and
// deformers go back to the shape recorded for them, in the keyform the
// parameters sit on.

use ayatsuri_core::edit::Target;
use ayatsuri_core::original;
use ayatsuri_format::model::OriginalHow;
use egui::Ui;

use super::EditorUi;
use crate::session::Session;

/// Seconds since 1970, for stamping an original shape.
pub(super) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `2026/11/02 21:40:05` in the computer's own time zone, as Cubism's
/// Inspector shows it, from seconds since 1970.
fn date(secs: u64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(secs as i64, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => stamp(&t),
        chrono::LocalResult::None => stamp(&chrono::Utc.timestamp_opt(secs as i64, 0).unwrap()),
    }
}

fn stamp<Tz: chrono::TimeZone>(t: &chrono::DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    t.format("%Y/%m/%d %H:%M:%S").to_string()
}

/// The shapes to record `how` for the objects that take one, read off the
/// puppet as it stands.
pub(super) fn shapes_to_take(
    s: &Session,
    targets: &[Target],
    how: OriginalHow,
) -> Vec<(Target, Vec<f32>)> {
    targets
        .iter()
        .filter(|&&t| original::takes(s.puppet.model(), t, how))
        .filter_map(|&t| original::shape_now(&s.puppet, t).map(|p| (t, p)))
        .collect()
}

/// The inspector's Original Shape row, and Auto Update for a deformer.
pub(super) fn inspector_rows(ui: &mut Ui, s: &mut Session, t: Target) {
    let shape = original::find(s.puppet.model(), t).cloned();
    let fits = shape.as_ref().is_some_and(|o| {
        original::shape_now(&s.puppet, t).is_some_and(|p| p.len() == o.points.len())
    });
    let text = match &shape {
        Some(o) if fits => {
            let how = match (o.how, t) {
                (OriginalHow::Created, Target::Deformer(_)) => crate::i18n::tr("Deformer creation"),
                (OriginalHow::Created, _) => crate::i18n::tr("Creation"),
                (OriginalHow::KeyAdded, _) => crate::i18n::tr("Key addition"),
                (OriginalHow::Manual, _) => crate::i18n::tr("Manual Update"),
            };
            format!("{} ({how})", date(o.time))
        }
        _ if matches!(t, Target::Deformer(_)) => crate::i18n::tr("Uncreated").to_string(),
        _ => crate::i18n::tr("Uncreated (default pose)").to_string(),
    };
    ui.horizontal(|ui| {
        ui.add_sized(
            [74.0, 16.0],
            egui::Label::new(
                egui::RichText::new(crate::i18n::tr("Original shape"))
                    .color(crate::theme::pal().text_faint)
                    .size(11.0),
            ),
        );
        ui.label(egui::RichText::new(text).size(11.0)).on_hover_text(crate::i18n::tr(
            "What Revert to Original puts it back to. Taken when it is made and when its first key is added; a mesh with none goes back to its shape at every parameter's default",
        ));
    });
    if matches!(t, Target::Deformer(_)) {
        let mut auto = shape.as_ref().is_none_or(|o| o.auto);
        if ui
            .add_enabled(
                shape.is_some(),
                egui::Checkbox::new(&mut auto, crate::i18n::tr("Auto Update")),
            )
            .on_hover_text(crate::i18n::tr(
                "Take the original shape again each time a key is added",
            ))
            .changed()
        {
            s.edit(|m| original::set_auto(m, t, auto));
        }
    }
}

impl EditorUi {
    /// The selected meshes and deformers, deformers first and parents
    /// before children, so each reverts against parents already back.
    fn original_targets(&self) -> Vec<Target> {
        let mut out: Vec<Target> =
            self.param_targets().into_iter().filter(|t| !matches!(t, Target::Part(_))).collect();
        out.sort_by_key(|t| match *t {
            Target::Deformer(i) => (0, i),
            Target::ArtMesh(i) => (1, i),
            Target::Part(i) => (2, i),
        });
        out
    }

    /// Whether Revert to Original and Update Original Shape have anything
    /// to act on.
    pub(super) fn has_original_targets(&self) -> bool {
        self.session.is_some() && !self.original_targets().is_empty()
    }

    /// Edit Form > Revert to Original: every selected mesh and deformer back
    /// to its original shape, in the keyform the parameters sit on, as one
    /// undo step.
    pub(super) fn revert_to_original(&mut self) {
        let targets = self.original_targets();
        let Some(s) = &mut self.session else { return };
        if targets.is_empty() {
            log::warn!("Revert to Original: select the meshes or deformers to revert first");
            return;
        }
        let values: Vec<f32> =
            (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
        let (mut done, mut between) = (0, 0);
        for t in targets {
            let Some(k) = ayatsuri_core::keyform::keyform_at(s.puppet.model(), t, &values) else {
                between += 1;
                continue;
            };
            // worked out against the puppet with its parents already reverted
            let Some(r) = original::revert(&s.puppet, t) else { continue };
            if s.edit(|m| original::apply(m, t, k, &r)) {
                if done > 0 {
                    s.join_last_edit();
                }
                done += 1;
            }
        }
        if between > 0 {
            log::warn!(
                "Revert to Original: the parameters are between keys for {between} object(s); move them onto a key"
            );
        }
        if done > 0 {
            log::info!("reverted {done} object(s) to their original shape");
        } else if between == 0 {
            log::info!("Revert to Original: already in their original shape");
        }
    }

    /// Edit Form > Update Original Shape: take the selected objects' shapes
    /// as they are drawn now. This turns a deformer's Auto Update off.
    pub(super) fn update_original_shapes(&mut self) {
        let targets = self.original_targets();
        let Some(s) = &mut self.session else { return };
        let shapes = shapes_to_take(s, &targets, OriginalHow::Manual);
        if shapes.is_empty() {
            log::warn!("Update Original Shape: select the meshes or deformers to take first");
            return;
        }
        let n = shapes.len();
        let time = now();
        if s.edit(|m| {
            shapes.into_iter().fold(false, |any, (t, p)| {
                original::record(m, t, p, OriginalHow::Manual, time) | any
            })
        }) {
            log::info!("took the original shape of {n} object(s)");
        }
    }

    /// Run one of the menu's original shape commands by its shortcut id.
    pub(super) fn original_cmd(&mut self, id: &str) {
        match id {
            "revert-original" => self.revert_to_original(),
            "update-original" => self.update_original_shapes(),
            "clear-originals" => self.clear_original_shapes(),
            _ => {}
        }
    }

    /// Edit Form > Delete Original Shape of All Deformers.
    pub(super) fn clear_original_shapes(&mut self) {
        let Some(s) = &mut self.session else { return };
        if s.edit(original::clear_deformers) {
            log::info!("deleted every deformer's original shape");
        } else {
            log::info!("no deformer has an original shape to delete");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Sel;
    use super::*;
    use ayatsuri_format::Model;
    use ayatsuri_format::model::{ArtMesh, Binding, Parameter};

    /// One unkeyed single-vertex mesh at (0.5, 0.5) and a parameter P.
    fn editor() -> EditorUi {
        let mut model = Model::new();
        model.parameters.push(Parameter {
            id: "P".into(),
            min: -1.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape: false,
        });
        model.art_meshes.push(ArtMesh {
            id: "ArtMesh0".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 1,
            uvs: vec![0.0, 0.0],
            indices: vec![0, 0, 0],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.5, 0.5],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
            skin: None,
        });
        let ctx = egui::Context::default();
        let mut ed = EditorUi::new();
        ed.session = Some(Session::create(&ctx, "t".into(), String::new(), model).unwrap());
        ed.selected = Some(Sel::Mesh(0));
        ed.sel_param = Some(0);
        ed
    }

    fn drawn(ed: &EditorUi, value: f32) -> Vec<f32> {
        let s = ed.session.as_ref().unwrap();
        let mut p = ayatsuri_core::Puppet::new(s.puppet.model().clone());
        p.set_value(0, value);
        p.update();
        p.mesh_drawn_positions(0).unwrap()
    }

    #[test]
    fn a_mesh_keyed_then_moved_reverts_to_the_art_as_it_was() {
        let mut ed = editor();
        ed.add_keys_to_selection(vec![-1.0, 0.0, 1.0]);
        let s = ed.session.as_mut().unwrap();
        let o = original::find(s.puppet.model(), Target::ArtMesh(0)).unwrap();
        assert_eq!((o.points.clone(), o.how), (vec![0.5, 0.5], OriginalHow::KeyAdded));
        // move the vertex in the key at 1, then revert with P on that key
        let k = ayatsuri_core::keyform::keyform_at(s.puppet.model(), Target::ArtMesh(0), &[1.0])
            .unwrap();
        assert!(s.edit(|m| ayatsuri_core::edit::move_vertices_in(
            m,
            0,
            Some(k),
            &[(0, 0.3, -0.2)]
        )));
        s.puppet.set_value(0, 1.0);
        s.puppet.update();
        assert_eq!(drawn(&ed, 1.0), [0.8, 0.3]);
        ed.revert_to_original();
        let back = drawn(&ed, 1.0);
        assert!((back[0] - 0.5).abs() < 1e-5 && (back[1] - 0.5).abs() < 1e-5, "{back:?}");
        // one undo takes the revert back
        ed.session.as_mut().unwrap().undo();
        assert_eq!(drawn(&ed, 1.0), [0.8, 0.3]);
    }

    #[test]
    fn a_mesh_keeps_the_shape_from_its_first_key() {
        let mut ed = editor();
        ed.add_keys_to_selection(vec![0.0]);
        let s = ed.session.as_mut().unwrap();
        assert!(s.edit(|m| ayatsuri_core::edit::move_vertices_in(m, 0, Some(0), &[(0, 0.1, 0.0)])));
        ed.add_keys_to_selection(vec![1.0]);
        let s = ed.session.as_ref().unwrap();
        let o = original::find(s.puppet.model(), Target::ArtMesh(0)).unwrap();
        assert_eq!(o.points, [0.5, 0.5]);
    }

    #[test]
    fn dates_read_as_cubism_writes_them() {
        use chrono::TimeZone;
        let at = |s: i64| stamp(&chrono::Utc.timestamp_opt(s, 0).unwrap());
        assert_eq!(at(0), "1970/01/01 00:00:00");
        assert_eq!(at(1_793_655_605), "2026/11/02 21:40:05");
        assert_eq!(at(951_782_400), "2000/02/29 00:00:00");
        assert_eq!(date(0).len(), "1970/01/01 00:00:00".len());
    }
}
