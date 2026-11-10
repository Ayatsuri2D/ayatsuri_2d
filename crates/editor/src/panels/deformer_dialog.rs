// The New Warp / New Rotation Deformer dialog, laid out like Cubism's:
// destination part, name, where the deformer goes (as parent of the
// selection, as child of the selected deformer, or under a parent picked by
// hand), then Continuous creation / Create / Close. A bone drawn with the
// Bone tool asks here too (New Bone): part, name and ID only, since the
// drag already said where it goes; Close makes nothing.

use ayatsuri_core::edit::{Child, NewDeformer};
use ayatsuri_format::model::DeformerKind;
use egui::Ui;

use super::{EditorUi, Sel, parent_delta};

/// Where the new deformer goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    ParentOfSelection,
    ChildOfSelection,
    Manual,
}

pub(super) struct DeformerDialog {
    warp: bool,
    /// The name shown in the palettes; empty shows the ID.
    name: String,
    id: String,
    part: Option<u32>,
    placement: Placement,
    /// Set Parent Manually's choice; None is the root.
    manual_parent: Option<u32>,
    cols: u32,
    rows: u32,
    /// Where the canvas was clicked, model space (rotation origin).
    click: Option<(f32, f32)>,
    /// New Bone: the bone the Bone tool drew, made on Create.
    bone: Option<DrawnBone>,
    /// The last Create was turned down: the ID is empty or taken.
    refused: bool,
}

/// A bone dragged out on the canvas, waiting for its name and ID.
#[derive(Clone)]
pub(super) struct DrawnBone {
    pub(super) parent: Option<usize>,
    pub(super) joint: (f32, f32),
    pub(super) tip: (f32, f32),
    /// The mesh the weights go on when it was drawn, by id: a new root
    /// bone files into that mesh's folder.
    pub(super) mesh: Option<String>,
}

impl DeformerDialog {
    /// The bone waiting in New Bone: joint and tip, model units.
    pub(super) fn drawn_bone(&self) -> Option<((f32, f32), (f32, f32))> {
        self.bone.as_ref().map(|b| (b.joint, b.tip))
    }
}

impl EditorUi {
    /// Open the dialog for the current selection, prefilled the way Cubism
    /// does: the selection's part, a fresh name, as parent of the selection.
    pub(super) fn open_deformer_dialog(&mut self, warp: bool, click: Option<(f32, f32)>) {
        let Some(s) = &self.session else { return };
        let model = s.puppet.model();
        let meshes = self.selection_meshes();
        let part = match self.selected {
            Some(Sel::Deformer(d)) => model.deformers.get(d).and_then(|x| x.parent_part),
            _ => meshes.first().and_then(|&m| model.art_meshes.get(m)).and_then(|m| m.parent_part),
        };
        let id = self.next_def_id(warp);
        let has_selection = !meshes.is_empty() || matches!(self.selected, Some(Sel::Deformer(_)));
        self.deformer_dialog = Some(DeformerDialog {
            warp,
            name: String::new(),
            id,
            part,
            placement: if has_selection { Placement::ParentOfSelection } else { Placement::Manual },
            manual_parent: None,
            cols: self.tool_opts.warp_cols,
            rows: self.tool_opts.warp_rows,
            click,
            bone: None,
            refused: false,
        });
    }

    /// Ask for the name and ID of a bone just drawn. Drawing another while
    /// it asks moves the bone and keeps what was typed.
    pub(super) fn open_bone_dialog(&mut self, drawn: DrawnBone, part: Option<u32>) {
        let Some(s) = &self.session else { return };
        if let Some(d) = self.deformer_dialog.as_mut().filter(|d| d.bone.is_some()) {
            d.bone = Some(drawn);
            d.part = part;
            return;
        }
        self.deformer_dialog = Some(DeformerDialog {
            warp: false,
            name: String::new(),
            id: ayatsuri_core::bone_edit::free_bone_id(s.puppet.model()),
            part,
            placement: Placement::Manual,
            manual_parent: None,
            cols: self.tool_opts.warp_cols,
            rows: self.tool_opts.warp_rows,
            click: None,
            bone: Some(drawn),
            refused: false,
        });
    }

    /// The ID a new deformer starts with: the last one made, counted on,
    /// else Warp or Rotation and the first free number.
    fn next_def_id(&self, warp: bool) -> String {
        let Some(s) = &self.session else { return String::new() };
        let taken = |id: &str| s.puppet.model().deformers.iter().any(|d| d.id == id);
        match &self.last_def_id {
            Some(last) => next_id(last, taken),
            None => {
                let kind = if warp { "Warp" } else { "Rotation" };
                next_id(&format!("{kind}{}", s.puppet.model().deformers.len().max(1) - 1), taken)
            }
        }
    }

    pub(super) fn deformer_dialog_window(&mut self, ui: &mut Ui) {
        let deformer_selected = matches!(self.selected, Some(Sel::Deformer(_)));
        let has_selection = deformer_selected || !self.selection_meshes().is_empty();
        let Some(dlg) = &mut self.deformer_dialog else { return };
        let Some(s) = &self.session else {
            self.deformer_dialog = None;
            return;
        };
        let model = s.puppet.model();
        let part_label = |i: usize| {
            s.part_names
                .get(i)
                .cloned()
                .filter(|n| !n.is_empty())
                .or_else(|| model.parts.get(i).map(|p| p.id.clone()))
                .unwrap_or_default()
        };
        let is_bone = dlg.bone.is_some();
        let title = if is_bone {
            crate::i18n::tr("New Bone")
        } else if dlg.warp {
            crate::i18n::tr("New Warp Deformer")
        } else {
            crate::i18n::tr("New Rotation Deformer")
        };
        let mut open = true;
        let (mut create, mut keep_open, mut close) = (false, false, false);
        crate::windows::Dialog::new(title).open(&mut open).show(ui.ctx(), |ui| {
            ui.set_width(330.0);
            if !has_selection && !is_bone {
                // an empty deformer moves nothing until something is under it
                ui.label(
                    egui::RichText::new(crate::i18n::tr(
                        "Nothing is selected, so this deformer will hold nothing and moving it \
                         won't move the model. Select meshes (or click one) first.",
                    ))
                    .size(11.0)
                    .color(crate::theme::pal().accent),
                );
                ui.add_space(6.0);
            }
            crate::windows::form(ui, "newdef", |ui| {
                ui.label(crate::i18n::tr("Destination Part"));
                egui::ComboBox::from_id_salt("newdef-part")
                    .width(200.0)
                    .selected_text(
                        dlg.part
                            .map_or(crate::i18n::tr("(none)").into(), |p| part_label(p as usize)),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut dlg.part, None, crate::i18n::tr("(none)"));
                        for i in 0..model.parts.len() {
                            ui.selectable_value(&mut dlg.part, Some(i as u32), part_label(i));
                        }
                    });
                ui.end_row();
                ui.label(crate::i18n::tr("Name"));
                let name = ui.add(
                    egui::TextEdit::singleline(&mut dlg.name)
                        .desired_width(200.0)
                        .hint_text(dlg.id.clone()),
                );
                ui.end_row();
                ui.label(crate::i18n::tr("ID"));
                let id = ui.add(egui::TextEdit::singleline(&mut dlg.id).desired_width(200.0));
                if id.changed() {
                    id_text(&mut dlg.id);
                    dlg.refused = false;
                }
                ui.end_row();
                if is_bone {
                    // the name takes the keyboard, and Enter makes the bone,
                    // so a chain is named as fast as it is drawn
                    if ui.memory(|m| m.focused().is_none()) {
                        name.request_focus();
                    }
                    let enter = ui.input(|k| k.key_pressed(egui::Key::Enter));
                    create |= (name.lost_focus() || id.lost_focus()) && enter;
                }
                if dlg.warp {
                    ui.label(crate::i18n::tr("Conversion divisions"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut dlg.cols).range(1..=32).prefix("W "));
                        ui.add(egui::DragValue::new(&mut dlg.rows).range(1..=32).prefix("H "));
                    });
                    ui.end_row();
                }
            });
            if dlg.refused {
                ui.colored_label(
                    crate::theme::pal().rotation_red,
                    crate::i18n::tr("That ID is empty or already used."),
                );
            }
            if is_bone {
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Create")).clicked() {
                        create = true;
                    }
                    if ui.button(crate::i18n::tr("Close")).clicked() {
                        close = true;
                    }
                });
                return;
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("Add"));
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.add_enabled_ui(has_selection, |ui| {
                        ui.radio_value(
                            &mut dlg.placement,
                            Placement::ParentOfSelection,
                            crate::i18n::tr("As Parent of Selected Object"),
                        );
                    });
                    ui.add_enabled_ui(deformer_selected, |ui| {
                        ui.radio_value(
                            &mut dlg.placement,
                            Placement::ChildOfSelection,
                            crate::i18n::tr("As Child of Selected Object"),
                        );
                    });
                    ui.radio_value(
                        &mut dlg.placement,
                        Placement::Manual,
                        crate::i18n::tr("Set Parent Manually"),
                    );
                    ui.add_enabled_ui(dlg.placement == Placement::Manual, |ui| {
                        let label = |p: Option<u32>| {
                            p.and_then(|i| model.deformers.get(i as usize))
                                .map_or(crate::i18n::tr("Root Deformer").to_string(), |d| {
                                    d.id.clone()
                                })
                        };
                        egui::ComboBox::from_id_salt("newdef-parent")
                            .width(180.0)
                            .selected_text(label(dlg.manual_parent))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut dlg.manual_parent,
                                    None,
                                    crate::i18n::tr("Root Deformer"),
                                );
                                for (i, d) in model.deformers.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut dlg.manual_parent,
                                        Some(i as u32),
                                        &d.id,
                                    );
                                }
                            });
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(crate::i18n::tr("Continuous creation"))
                    .on_hover_text(crate::i18n::tr(
                        "Create, and keep this dialog open for the next one",
                    ))
                    .clicked()
                {
                    create = true;
                    keep_open = true;
                }
                if ui.button(crate::i18n::tr("Create")).clicked() {
                    create = true;
                }
                if ui.button(crate::i18n::tr("Close")).clicked() {
                    close = true;
                }
            });
        });
        if create && is_bone {
            // a refused ID keeps the dialog open to fix it
            if self.create_bone_from_dialog() {
                self.deformer_dialog = None;
            }
        } else if create {
            self.create_from_dialog();
            if keep_open {
                // next ID, same settings
                let warp = self.deformer_dialog.as_ref().is_some_and(|d| d.warp);
                let id = self.next_def_id(warp);
                if let Some(d) = &mut self.deformer_dialog {
                    d.id = id;
                    d.name.clear();
                }
            } else {
                self.deformer_dialog = None;
            }
        } else if !open || close {
            self.deformer_dialog = None;
        }
        // the creation tools work through this dialog: once it is gone the
        // canvas goes back to the Arrow, so the next click edits what was
        // made instead of opening the dialog again
        if self.deformer_dialog.is_none()
            && matches!(self.tool, super::Tool::WarpCreate | super::Tool::RotationCreate)
        {
            self.tool = super::Tool::Arrow;
        }
    }

    /// New Bone's Create: the drawn bone with its ID, name and part, one
    /// undo step. A new root bone drawn for a mesh files into that mesh's
    /// folder (made when missing); a child goes where its root is. False
    /// when the ID is turned down, so the dialog stays to fix it.
    fn create_bone_from_dialog(&mut self) -> bool {
        let Some(dlg) = &mut self.deformer_dialog else { return true };
        let Some(drawn) = dlg.bone.clone() else { return true };
        let mut id = dlg.id.trim().to_string();
        id_text(&mut id);
        let label = dlg.name.trim().to_string();
        let part = dlg.part;
        let Some(s) = &mut self.session else { return true };
        if id.is_empty() || s.puppet.model().deformers.iter().any(|d| d.id == id) {
            log::warn!("New Bone: the ID {id:?} is empty or already used");
            dlg.refused = true;
            return false;
        }
        let made = std::cell::Cell::new(None);
        let DrawnBone { parent, joint, tip, mesh } = drawn;
        s.edit(|m| {
            made.set(ayatsuri_core::bone_edit::create_bone(
                m,
                parent,
                part,
                joint,
                tip,
                id.clone(),
            ));
            made.get().is_some()
        });
        let Some(b) = made.get() else {
            log::warn!("New Bone {id}: could not be made (its parent is gone)");
            return true;
        };
        let model = s.puppet.model();
        let root =
            !parent.is_some_and(|p| model.deformers.get(p).is_some_and(|d| d.kind.is_bone()));
        log::info!("bone {b} made as {id}{}", if root { "" } else { " (a child)" });
        if !label.is_empty() && label != id {
            if s.deformer_names.len() <= b {
                s.deformer_names.resize(b + 1, String::new());
            }
            if let Some(slot) = s.deformer_names.get_mut(b) {
                *slot = label;
            }
        }
        let mesh = mesh.and_then(|id| model.art_meshes.iter().position(|m| m.id == id));
        if root && let Some(mi) = mesh {
            let folder = format!("{}_bones", s.mesh_name(mi));
            let at = match s.bone_folders.iter().position(|f| f.name == folder) {
                Some(at) => at,
                None => {
                    s.bone_folders.push(crate::session::BoneFolder {
                        name: folder.clone(),
                        bones: Vec::new(),
                        open: true,
                    });
                    s.bone_folders.len() - 1
                }
            };
            if let Some(f) = s.bone_folders.get_mut(at) {
                f.bones.push(b);
            }
            s.modified = true;
            log::info!("bone {id} filed in {folder}");
        }
        self.bones.pick = vec![b];
        self.selected = Some(Sel::Deformer(b));
        self.def_pick = vec![b];
        true
    }

    fn create_from_dialog(&mut self) {
        let Some(dlg) = &self.deformer_dialog else { return };
        let mut name = dlg.id.trim().to_string();
        id_text(&mut name);
        let label = dlg.name.trim().to_string();
        let (warp, part, placement, manual_parent, cols, rows, click) =
            (dlg.warp, dlg.part, dlg.placement, dlg.manual_parent, dlg.cols, dlg.rows, dlg.click);
        if name.is_empty() {
            log::warn!("give the deformer an ID");
            return;
        }
        self.tool_opts.warp_cols = cols;
        self.tool_opts.warp_rows = rows;
        let meshes = self.selection_meshes();
        let selected_def = match self.selected {
            Some(Sel::Deformer(d)) => Some(d),
            _ => None,
        };
        // several deformers picked: the new one wraps them all
        let picked_defs = super::def_group(self.selected, &self.def_pick);
        let selection_note = format!(
            "selected {:?}, deformer pick {:?}, mesh pick {:?}, part pick {:?}",
            self.selected, self.def_pick, self.multi_sel, self.part_pick
        );
        let Some(s) = &mut self.session else { return };
        let model = s.puppet.model();
        if model.deformers.iter().any(|d| d.id == name) {
            log::warn!("a deformer named {name} already exists");
            return;
        }
        // what the new deformer wraps
        let mut children: Vec<Child> = match placement {
            Placement::ParentOfSelection => {
                let mut c: Vec<Child> = meshes.iter().map(|&m| Child::Mesh(m)).collect();
                c.extend(picked_defs.iter().map(|&d| Child::Deformer(d)));
                c
            }
            Placement::ChildOfSelection => {
                let Some(d) = selected_def else {
                    log::warn!("select a deformer to add a child to");
                    return;
                };
                let d32 = Some(d as u32);
                let mut c: Vec<Child> = model
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.parent_deformer == d32)
                    .map(|(i, _)| Child::Mesh(i))
                    .collect();
                c.extend(
                    model
                        .deformers
                        .iter()
                        .enumerate()
                        .filter(|(_, x)| x.parent_deformer == d32)
                        .map(|(i, _)| Child::Deformer(i)),
                );
                c
            }
            Placement::Manual => Vec::new(),
        };
        // the parent the deformer lands under, for an empty one
        let empty_parent = match placement {
            Placement::ChildOfSelection => selected_def.map(|d| d as u32),
            _ => manual_parent,
        };
        // Set Parent Manually on the selection's own parent still wraps it
        if placement == Placement::Manual {
            let mut sel: Vec<Child> = meshes.iter().map(|&m| Child::Mesh(m)).collect();
            sel.extend(picked_defs.iter().map(|&d| Child::Deformer(d)));
            let same = !sel.is_empty()
                && sel.iter().all(|&c| {
                    let p = match c {
                        Child::Mesh(i) => model.art_meshes.get(i).map(|m| m.parent_deformer),
                        Child::Deformer(i) => model.deformers.get(i).map(|x| x.parent_deformer),
                    };
                    p == Some(manual_parent)
                });
            if same {
                children = sel;
            }
        }
        // an empty deformer gets a modest box in its parent's space
        let empty_extent = match empty_parent.and_then(|p| model.deformers.get(p as usize)) {
            Some(d) if matches!(d.kind, DeformerKind::Warp(_)) => (0.25, 0.25, 0.75, 0.75),
            Some(_) => (-0.1, -0.1, 0.1, 0.1),
            None => (-0.2, -0.2, 0.2, 0.2),
        };
        // a rotation's origin: the clicked point, carried into the parent's
        // space through its local jacobian around the first mesh
        let origin = if warp {
            None
        } else {
            click.and_then(|(mx, my)| {
                let first = children.iter().find_map(|c| match c {
                    Child::Mesh(m) => Some(*m),
                    _ => None,
                });
                match first {
                    None => (empty_parent.is_none() && children.is_empty()).then_some((mx, my)),
                    Some(mi) => {
                        let parent = model.art_meshes.get(mi).and_then(|m| m.parent_deformer);
                        if parent.is_none() {
                            return Some((mx, my));
                        }
                        let local = s.puppet.mesh_local_positions(mi)?;
                        let (lp, _) = local.as_chunks::<2>();
                        let n = lp.len().max(1) as f32;
                        let (clx, cly) =
                            lp.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0] / n, y + q[1] / n));
                        let g = s.puppet.mesh_geometry(mi)?;
                        let (gp, _) = g.positions.as_chunks::<2>();
                        let n = gp.len().max(1) as f32;
                        let (cmx, cmy) =
                            gp.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0] / n, y + q[1] / n));
                        let (dx, dy) =
                            parent_delta(&s.puppet, parent, clx, cly, mx - cmx, my - cmy);
                        Some((clx + dx, cly + dy))
                    }
                }
            })
        };
        // what is asked for, by name, to set against what it ends up holding
        let name_of = |model: &ayatsuri_format::Model, c: &Child| match *c {
            Child::Mesh(i) => {
                format!("mesh {}", model.art_meshes.get(i).map_or("?", |m| m.id.as_str()))
            }
            Child::Deformer(i) => {
                format!("deformer {}", model.deformers.get(i).map_or("?", |d| d.id.as_str()))
            }
        };
        let asked: Vec<String> = children.iter().map(|c| name_of(s.puppet.model(), c)).collect();
        log::info!(
            "New Deformer {name}: {selection_note}; {} wraps {} object(s): {}",
            match placement {
                Placement::ParentOfSelection => "as parent of the selection",
                Placement::ChildOfSelection => "as child of the selected deformer",
                Placement::Manual => "with its parent set by hand",
            },
            asked.len(),
            asked.join(", ")
        );
        let kind =
            if warp { NewDeformer::Warp { rows, cols } } else { NewDeformer::Rotation { origin } };
        let mut made = None;
        s.edit(|m| {
            made = ayatsuri_core::edit::create_deformer(
                m,
                kind,
                &children,
                empty_parent,
                empty_extent,
                part,
                name.clone(),
            );
            made.is_some()
        });
        match made {
            Some(di) => {
                // its original shape, folded into the undo step that made it
                let t = ayatsuri_core::edit::Target::Deformer(di);
                if let Some(p) = ayatsuri_core::original::shape_now(&s.puppet, t) {
                    let time = super::original::now();
                    let how = ayatsuri_format::model::OriginalHow::Created;
                    s.amend(|m| ayatsuri_core::original::record(m, t, p, how, time));
                }
                // what it holds now, against what was asked
                let model = s.puppet.model();
                let d32 = Some(di as u32);
                let mut got: Vec<String> = model
                    .art_meshes
                    .iter()
                    .filter(|m| m.parent_deformer == d32)
                    .map(|m| format!("mesh {}", m.id))
                    .collect();
                got.extend(
                    model
                        .deformers
                        .iter()
                        .filter(|d| d.parent_deformer == d32)
                        .map(|d| format!("deformer {}", d.id)),
                );
                let missing: Vec<&String> = asked.iter().filter(|a| !got.contains(a)).collect();
                if missing.is_empty() {
                    log::info!("New Deformer {name}: holds {}", got.join(", "));
                } else {
                    log::warn!(
                        "New Deformer {name}: holds {} but not {}",
                        got.join(", "),
                        missing.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ")
                    );
                }
                // indices at or past the insert point moved up one
                let shift = |set: &std::collections::HashSet<usize>| {
                    set.iter().map(|&i| if i >= di { i + 1 } else { i }).collect()
                };
                self.def_locked = shift(&self.def_locked);
                self.def_collapsed = shift(&self.def_collapsed);
                log::info!(
                    "created {name} ({}) holding {} object(s)",
                    if warp { "warp" } else { "rotation" },
                    children.len()
                );
                if !label.is_empty() && label != name {
                    if s.deformer_names.len() <= di {
                        s.deformer_names.resize(di + 1, String::new());
                    }
                    if let Some(slot) = s.deformer_names.get_mut(di) {
                        *slot = label;
                    }
                }
                self.last_def_id = Some(name.clone());
                self.selected = Some(Sel::Deformer(di));
                self.multi_sel.clear();
                self.def_pick.clear();
                self.def_anchor = None;
            }
            None => log::warn!(
                "could not create {name}: the selected objects must share one parent deformer"
            ),
        }
    }
}

/// An ID as typed: IDs hold no spaces, so each becomes `_`.
pub(super) fn id_text(id: &mut String) {
    if id.contains(char::is_whitespace) {
        *id = id.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect();
    }
}

/// The next ID after `last`: its trailing number counted on to the first
/// one free, so Head_Rotation gives Head_Rotation2 and Hair_L3 Hair_L4.
pub(super) fn next_id(last: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = last.trim_end_matches(|c: char| c.is_ascii_digit());
    let mut n: u32 =
        last.get(base.len()..).and_then(|d| d.parse().ok()).map_or(2, |n: u32| n.saturating_add(1));
    loop {
        let id = format!("{base}{n}");
        if !taken(&id) || n == u32::MAX {
            return id;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use ayatsuri_format::Model;

    #[test]
    fn a_new_parent_wraps_every_picked_deformer() {
        let mut m = Model::new();
        let warp = |m: &mut Model, id: &str| {
            let kind = NewDeformer::Warp { rows: 1, cols: 1 };
            ayatsuri_core::edit::create_deformer(
                m,
                kind,
                &[],
                None,
                (0.0, 0.0, 1.0, 1.0),
                None,
                id.into(),
            )
            .unwrap()
        };
        for id in ["EyeL", "WhiteEyeL", "LowEyeL"] {
            warp(&mut m, id);
        }
        let ctx = egui::Context::default();
        let mut ed = EditorUi::new();
        ed.session = Some(Session::create(&ctx, "t".into(), String::new(), m).unwrap());
        ed.selected = Some(Sel::Deformer(0));
        ed.def_pick = vec![0, 1, 2];
        ed.open_deformer_dialog(true, None);
        if let Some(d) = &mut ed.deformer_dialog {
            d.id = "Eyes".into();
        }
        ed.create_from_dialog();
        let model = ed.session.as_ref().unwrap().puppet.model();
        let parent = model.deformers.iter().position(|d| d.id == "Eyes").unwrap() as u32;
        for id in ["EyeL", "WhiteEyeL", "LowEyeL"] {
            let d = model.deformers.iter().find(|d| d.id == id).unwrap();
            assert_eq!(d.parent_deformer, Some(parent), "{id} goes under the new warp");
        }
        assert!(ed.def_pick.is_empty());
    }

    #[test]
    fn the_next_id_counts_on_from_the_last() {
        let taken = |id: &str| ["Head_Rotation2", "Hair_L4"].contains(&id);
        assert_eq!(next_id("Head_Rotation", taken), "Head_Rotation3");
        assert_eq!(next_id("Hair_L3", taken), "Hair_L5");
        assert_eq!(next_id("Warp9", |_| false), "Warp10");
        let mut id = "Head Rotation L".to_string();
        id_text(&mut id);
        assert_eq!(id, "Head_Rotation_L");
    }

    #[test]
    fn a_new_deformer_takes_its_id_and_name_and_the_next_starts_from_it() {
        let ctx = egui::Context::default();
        let mut ed = EditorUi::new();
        ed.session = Some(Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap());
        ed.open_deformer_dialog(true, None);
        if let Some(d) = &mut ed.deformer_dialog {
            d.id = "Head Rotation".into();
            d.name = "Head".into();
        }
        ed.create_from_dialog();
        let s = ed.session.as_ref().unwrap();
        let di = s.puppet.model().deformers.iter().position(|d| d.id == "Head_Rotation").unwrap();
        assert_eq!(s.deformer_names[di], "Head");
        ed.open_deformer_dialog(true, None);
        assert_eq!(ed.deformer_dialog.as_ref().unwrap().id, "Head_Rotation2");
    }

    fn bang() -> ayatsuri_format::model::ArtMesh {
        ayatsuri_format::model::ArtMesh {
            id: "ArtMesh0".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 4,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            indices: vec![0, 1, 2, 0, 2, 3],
            masks: vec![],
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
            skin: None,
        }
    }

    fn drawn(parent: Option<usize>, joint: (f32, f32), tip: (f32, f32)) -> DrawnBone {
        DrawnBone { parent, joint, tip, mesh: Some("ArtMesh0".into()) }
    }

    #[test]
    fn closing_new_bone_makes_nothing() {
        let ctx = egui::Context::default();
        let mut ed = EditorUi::new();
        ed.session = Some(Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap());
        ed.open_bone_dialog(drawn(None, (0.0, 0.0), (0.0, 1.0)), None);
        assert_eq!(ed.deformer_dialog.as_ref().unwrap().id, "Bone0");
        ed.deformer_dialog = None;
        let s = ed.session.as_ref().unwrap();
        assert!(s.puppet.model().deformers.is_empty());
        assert!(!s.can_undo());
    }

    #[test]
    fn a_new_root_bone_files_into_its_meshs_folder_and_a_child_follows_its_root() {
        let ctx = egui::Context::default();
        let mut ed = EditorUi::new();
        let mut m = Model::new();
        m.art_meshes.push(bang());
        let mut s = Session::create(&ctx, "t".into(), String::new(), m).unwrap();
        s.mesh_names[0] = "front_bang".into();
        s.edit(|m| {
            ayatsuri_core::bone_edit::create_bone(
                m,
                None,
                None,
                (5.0, 5.0),
                (5.0, 6.0),
                "Taken".into(),
            )
            .is_some()
        });
        ed.session = Some(s);
        ed.open_bone_dialog(drawn(None, (0.0, 0.0), (0.0, 1.0)), None);
        // a taken ID is turned down and the dialog stays
        ed.deformer_dialog.as_mut().unwrap().id = "Taken".into();
        assert!(!ed.create_bone_from_dialog());
        assert!(ed.deformer_dialog.as_ref().unwrap().refused);
        if let Some(d) = &mut ed.deformer_dialog {
            d.id = "Bang Root".into();
            d.name = "Bang".into();
        }
        assert!(ed.create_bone_from_dialog());
        let s = ed.session.as_ref().unwrap();
        let root = s.puppet.model().deformers.iter().position(|d| d.id == "Bang_Root").unwrap();
        assert_eq!(s.deformer_names[root], "Bang");
        assert_eq!(s.bone_folders.len(), 1);
        assert_eq!(s.bone_folders[0].name, "front_bang_bones");
        assert_eq!(s.bone_folders[0].bones, vec![root]);
        assert_eq!(ed.selected, Some(Sel::Deformer(root)));
        // its child is made without a folder entry of its own
        ed.deformer_dialog = None;
        ed.open_bone_dialog(drawn(Some(root), (0.0, 1.0), (0.0, 2.0)), None);
        assert!(ed.create_bone_from_dialog());
        let s = ed.session.as_ref().unwrap();
        assert_eq!(s.puppet.model().deformers.len(), 3);
        assert_eq!(s.bone_folders[0].bones, vec![root]);
    }
}
