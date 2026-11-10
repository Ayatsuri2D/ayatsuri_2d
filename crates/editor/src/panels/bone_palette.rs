// The Bones palette, behind Settings > Experimental > Bones: only bones,
// as their parent/child tree, filed in folders the rigger names (a tester's
// "front_bang_bones"). A folder holds whole chains: it lists root bones,
// and every bone goes where its root goes, so dragging any bone of a chain
// moves the chain. Rows pick like the Deformer palette's (Ctrl/Shift for
// several) and the canvas follows; an IK chain's first bone shows a target.
// Folders live in the session beside the deformer names and are saved in
// the .aom; like the Parameter palette's folders they take no undo step.

use std::collections::HashMap;

use ayatsuri_format::Model;
use egui::{Align2, FontId, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{DefCmd, EditorUi, Sel, def_group, paint_bone_glyph, paint_picked_row, truncate};
use crate::i18n::{tr, trf};
use crate::session::{BoneFolder, Session};
use crate::theme;

/// The palette's state between frames.
#[derive(Default)]
pub(super) struct BonePalette {
    search: String,
    /// New Folder (no folder yet) or Rename Folder: which, the name typed,
    /// and the bones a new folder takes in.
    dialog: Option<FolderDialog>,
}

struct FolderDialog {
    folder: Option<usize>,
    name: String,
    bones: Vec<usize>,
}

/// A bone row being dragged.
#[derive(Clone, Copy)]
struct Dragged(usize);

/// What the palette was asked to do this frame.
enum BoneCmd {
    Pick(usize),
    /// These bones' chains into a folder (None: the top level).
    Move(Vec<usize>, Option<usize>),
    NewFolder(Vec<usize>),
    RenameFolder(usize),
    DeleteFolder(usize),
    ToggleFolder(usize),
    Fold(usize),
    Rename(usize),
    Delete(usize),
}

enum Row {
    Folder(usize),
    Bone { bone: usize, depth: usize, folder: Option<usize>, has_kids: bool },
}

fn is_bone(model: &Model, d: usize) -> bool {
    model.deformers.get(d).is_some_and(|x| x.kind.is_bone())
}

/// The first bone of `bone`'s chain: up through its parents while they
/// are bones.
pub(super) fn chain_root(model: &Model, bone: usize) -> usize {
    let mut at = bone;
    for _ in 0..=model.deformers.len() {
        match model.deformers.get(at).and_then(|d| d.parent_deformer).map(|p| p as usize) {
            Some(p) if is_bone(model, p) => at = p,
            _ => break,
        }
    }
    at
}

/// Keep each folder to root bones, each in one folder only: a bone that
/// is gone, no longer a bone, or now inside another bone's chain (it was
/// moved under it) leaves its folder, since its root says where it goes.
pub(super) fn tidy(folders: &mut [BoneFolder], model: &Model) {
    let mut seen = std::collections::HashSet::new();
    for f in folders {
        f.bones.retain(|&b| is_bone(model, b) && chain_root(model, b) == b && seen.insert(b));
    }
}

/// Move the chains of `bones` into folder `to`, or out to the top level.
pub(super) fn move_chains(s: &mut Session, bones: &[usize], to: Option<usize>) {
    let model = s.puppet.model();
    let mut roots: Vec<usize> = Vec::new();
    for &b in bones {
        let r = chain_root(model, b);
        if is_bone(model, r) && !roots.contains(&r) {
            roots.push(r);
        }
    }
    if roots.is_empty() || to.is_some_and(|f| f >= s.bone_folders.len()) {
        return;
    }
    for f in &mut s.bone_folders {
        f.bones.retain(|b| !roots.contains(b));
    }
    if let Some(f) = to.and_then(|f| s.bone_folders.get_mut(f)) {
        f.bones.extend(roots.iter().copied());
    }
    s.modified = true;
    match to.and_then(|f| s.bone_folders.get(f)) {
        Some(f) => log::info!("{} bone chain(s) moved into {}", roots.len(), f.name),
        None => log::info!("{} bone chain(s) moved to the top level", roots.len()),
    }
}

/// Every bone and the bones under it, top to bottom: each folder's chains
/// under its header, then the loose ones. A search shows the bones that
/// match and their parents, everything open.
fn rows(s: &Session, folded: &std::collections::HashSet<usize>, query: &str) -> Vec<Row> {
    let model = s.puppet.model();
    let nd = model.deformers.len();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); nd];
    let mut roots: Vec<usize> = Vec::new();
    for i in (0..nd).filter(|&i| is_bone(model, i)) {
        match model.deformers.get(i).and_then(|d| d.parent_deformer).map(|p| p as usize) {
            Some(p) if is_bone(model, p) => {
                if let Some(k) = kids.get_mut(p) {
                    k.push(i);
                }
            }
            _ => roots.push(i),
        }
    }
    // a search keeps a bone that matches or holds one that does
    let mut keep = vec![query.is_empty(); nd];
    if !query.is_empty() {
        for i in (0..nd).filter(|&i| is_bone(model, i)) {
            let id = model.deformers.get(i).map(|d| d.id.as_str()).unwrap_or_default();
            if super::matches_search(query, &[id, &s.deformer_name(i)]) {
                let mut at = i;
                for _ in 0..=nd {
                    if let Some(k) = keep.get_mut(at) {
                        *k = true;
                    }
                    match model.deformers.get(at).and_then(|d| d.parent_deformer) {
                        Some(p) if is_bone(model, p as usize) => at = p as usize,
                        _ => break,
                    }
                }
            }
        }
    }
    let searching = !query.is_empty();
    let chain = |out: &mut Vec<Row>, root: usize, folder: Option<usize>| {
        let mut stack = vec![(root, 0usize)];
        let mut guard = 0;
        while let Some((b, depth)) = stack.pop() {
            guard += 1;
            if guard > nd + 1 {
                break;
            }
            if !keep.get(b).copied().unwrap_or(false) {
                continue;
            }
            let k = kids.get(b).map(Vec::as_slice).unwrap_or_default();
            out.push(Row::Bone { bone: b, depth, folder, has_kids: !k.is_empty() });
            if searching || !folded.contains(&b) {
                stack.extend(k.iter().rev().map(|&c| (c, depth + 1)));
            }
        }
    };
    let mut out = Vec::new();
    let mut filed: HashMap<usize, usize> = HashMap::new();
    for (fi, f) in s.bone_folders.iter().enumerate() {
        for &b in &f.bones {
            filed.entry(b).or_insert(fi);
        }
    }
    for (fi, f) in s.bone_folders.iter().enumerate() {
        let shown: Vec<usize> = f.bones.iter().copied().filter(|r| roots.contains(r)).collect();
        if searching && !shown.iter().any(|&r| keep.get(r).copied().unwrap_or(false)) {
            continue;
        }
        out.push(Row::Folder(fi));
        if f.open || searching {
            for r in shown {
                chain(&mut out, r, Some(fi));
            }
        }
    }
    for r in roots.into_iter().filter(|r| !filed.contains_key(r)) {
        chain(&mut out, r, None);
    }
    out
}

impl EditorUi {
    pub(super) fn bones_panel(&mut self, ui: &mut Ui) {
        if self.tdeform.is_some() {
            ui.disable();
        }
        let rename_sc = self.sc("rename");
        let picked = def_group(self.selected, &self.def_pick);
        let Some(s) = &mut self.session else {
            ui.weak(tr("No model loaded."));
            return;
        };
        tidy(&mut s.bone_folders, s.puppet.model());
        super::search_box(ui, &mut self.bone_pal.search, tr("Search bones"));
        let query = self.bone_pal.search.trim().to_lowercase();
        let rows = rows(s, &self.def_collapsed, &query);
        let model = s.puppet.model();
        // an IK chain's first bone, and the bone it reaches for
        let ik_root: HashMap<usize, usize> = model
            .ik
            .iter()
            .filter_map(|c| Some((*c.bones.first()? as usize, c.target as usize)))
            .collect();
        let order: Vec<usize> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Bone { bone, .. } => Some(*bone),
                Row::Folder(_) => None,
            })
            .collect();
        let mut cmds: Vec<BoneCmd> = Vec::new();
        // a drag moves the pick when it starts on a picked bone
        let moving = |d: usize| if picked.contains(&d) { picked.clone() } else { vec![d] };
        let accent = theme::pal().accent;
        let drop_ring = |ui: &Ui, r: Rect| {
            ui.painter().rect_stroke(
                r.shrink(1.0),
                3.0,
                Stroke::new(1.0, accent.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        };
        ScrollArea::vertical().id_salt("bones").auto_shrink([false, false]).show(ui, |ui| {
            if rows.is_empty() && query.is_empty() {
                ui.weak(tr("No bones yet. Draw one with the Bone tool."));
            }
            for row in &rows {
                match *row {
                    Row::Folder(fi) => {
                        let Some(f) = s.bone_folders.get(fi) else { continue };
                        let (rect, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
                        ui.painter().rect_filled(rect, 2.0, theme::pal().header);
                        let arrow = if f.open || !query.is_empty() { "⏷" } else { "⏵" };
                        ui.painter().text(
                            rect.left_center() + vec2(6.0, 0.0),
                            Align2::LEFT_CENTER,
                            format!("{arrow} {} {}", egui_phosphor::regular::FOLDER_SIMPLE, f.name),
                            FontId::proportional(11.0),
                            theme::pal().text,
                        );
                        if resp.dnd_hover_payload::<Dragged>().is_some() {
                            drop_ring(ui, rect);
                        }
                        if let Some(d) = resp.dnd_release_payload::<Dragged>() {
                            cmds.push(BoneCmd::Move(moving(d.0), Some(fi)));
                        }
                        if resp.double_clicked() {
                            cmds.push(BoneCmd::RenameFolder(fi));
                        } else if resp.clicked() {
                            cmds.push(BoneCmd::ToggleFolder(fi));
                        }
                        resp.context_menu(|ui| {
                            if ui.button(tr("Rename Folder")).clicked() {
                                cmds.push(BoneCmd::RenameFolder(fi));
                                ui.close();
                            }
                            if ui
                                .button(tr("Delete Folder"))
                                .on_hover_text(tr("Its bones move to the top level"))
                                .clicked()
                            {
                                cmds.push(BoneCmd::DeleteFolder(fi));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(tr("New Folder…")).clicked() {
                                cmds.push(BoneCmd::NewFolder(Vec::new()));
                                ui.close();
                            }
                        });
                    }
                    Row::Bone { bone: i, depth, folder, has_kids } => {
                        let (rect, resp) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 18.0),
                            Sense::click_and_drag(),
                        );
                        if self.selected == Some(Sel::Deformer(i)) || picked.contains(&i) {
                            paint_picked_row(ui, rect);
                        }
                        if resp.drag_started() {
                            resp.dnd_set_drag_payload(Dragged(i));
                        }
                        // dropped on a bone: into the folder that bone is in
                        if resp.dnd_hover_payload::<Dragged>().is_some() {
                            drop_ring(ui, rect);
                        }
                        if let Some(d) = resp.dnd_release_payload::<Dragged>() {
                            cmds.push(BoneCmd::Move(moving(d.0), folder));
                        }
                        let indent = if folder.is_some() { 12.0 } else { 0.0 };
                        let x = rect.min.x + 18.0 + indent + depth as f32 * 14.0;
                        if has_kids {
                            let caret = Rect::from_center_size(
                                pos2(x - 10.0, rect.center().y),
                                vec2(12.0, 14.0),
                            );
                            let open = !self.def_collapsed.contains(&i) || !query.is_empty();
                            ui.painter().text(
                                caret.center(),
                                Align2::CENTER_CENTER,
                                if open { "⏷" } else { "⏵" },
                                FontId::proportional(9.0),
                                theme::pal().text_faint,
                            );
                            let c = ui.interact(
                                caret,
                                egui::Id::new(("bone-caret", i)),
                                Sense::click(),
                            );
                            if c.clicked() {
                                cmds.push(BoneCmd::Fold(i));
                            }
                        }
                        paint_bone_glyph(ui, pos2(x, rect.center().y));
                        ui.painter().text(
                            pos2(x + 16.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            truncate(&s.deformer_name(i), 26),
                            FontId::proportional(11.5),
                            theme::pal().text,
                        );
                        if let Some(&target) = ik_root.get(&i) {
                            let mark = Rect::from_center_size(
                                pos2(rect.max.x - 10.0, rect.center().y),
                                vec2(16.0, 16.0),
                            );
                            ui.painter().text(
                                mark.center(),
                                Align2::CENTER_CENTER,
                                egui_phosphor::regular::TARGET,
                                FontId::proportional(11.0),
                                accent,
                            );
                            ui.interact(mark, egui::Id::new(("bone-ik", i)), Sense::hover())
                                .on_hover_text(trf(
                                    "IK chain: reaches for {bone}",
                                    &[("bone", &s.deformer_name(target))],
                                ));
                        }
                        if resp.clicked() {
                            cmds.push(BoneCmd::Pick(i));
                        }
                        resp.context_menu(|ui| {
                            if ui
                                .add(egui::Button::new(tr("Rename…")).shortcut_text(&rename_sc))
                                .clicked()
                            {
                                cmds.push(BoneCmd::Rename(i));
                                ui.close();
                            }
                            ui.menu_button(tr("Move to Folder"), |ui| {
                                for (fi, f) in s.bone_folders.iter().enumerate() {
                                    if ui
                                        .add_enabled(folder != Some(fi), egui::Button::new(&f.name))
                                        .clicked()
                                    {
                                        cmds.push(BoneCmd::Move(moving(i), Some(fi)));
                                        ui.close();
                                    }
                                }
                                if !s.bone_folders.is_empty() {
                                    ui.separator();
                                }
                                if ui
                                    .add_enabled(
                                        folder.is_some(),
                                        egui::Button::new(tr("Top level")),
                                    )
                                    .clicked()
                                {
                                    cmds.push(BoneCmd::Move(moving(i), None));
                                    ui.close();
                                }
                                if ui.button(tr("New Folder…")).clicked() {
                                    cmds.push(BoneCmd::NewFolder(moving(i)));
                                    ui.close();
                                }
                            });
                            ui.separator();
                            if ui
                                .button(tr("Delete"))
                                .on_hover_text(tr(
                                    "Its meshes and deformers move up to its parent, unchanged",
                                ))
                                .clicked()
                            {
                                cmds.push(BoneCmd::Delete(i));
                                ui.close();
                            }
                        });
                    }
                }
            }
            // the empty space under the rows: dropped here, out of every folder
            let rest = ui.available_height().max(40.0);
            let (bg_rect, bg) =
                ui.allocate_exact_size(vec2(ui.available_width(), rest), Sense::click());
            if bg.dnd_hover_payload::<Dragged>().is_some() {
                drop_ring(ui, bg_rect);
            }
            if let Some(d) = bg.dnd_release_payload::<Dragged>() {
                cmds.push(BoneCmd::Move(moving(d.0), None));
            }
            bg.context_menu(|ui| {
                if ui.button(tr("New Folder…")).clicked() {
                    cmds.push(BoneCmd::NewFolder(Vec::new()));
                    ui.close();
                }
            });
        });
        // what is being dragged follows the pointer
        let ctx = ui.ctx().clone();
        if let Some(d) = egui::DragAndDrop::payload::<Dragged>(&ctx)
            && let Some(at) = ctx.pointer_latest_pos()
        {
            let n = moving(d.0).len();
            let text =
                if n > 1 { trf("{count} bones", &[("count", &n)]) } else { s.deformer_name(d.0) };
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("bone-drag"),
            ));
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), accent);
            let card = Rect::from_min_size(at + vec2(14.0, -6.0), galley.size() + vec2(12.0, 8.0));
            painter.rect_filled(card, 4.0, theme::pal().chrome.gamma_multiply(0.95));
            painter.rect_stroke(
                card,
                4.0,
                Stroke::new(1.0, accent.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
            painter.galley(card.min + vec2(6.0, 4.0), galley, accent);
        }
        for cmd in cmds {
            self.run_bone_cmd(ui, cmd, &order);
        }
    }

    fn run_bone_cmd(&mut self, ui: &Ui, cmd: BoneCmd, order: &[usize]) {
        match cmd {
            BoneCmd::Pick(i) => {
                let (shift, ctrl) = ui.ctx().input(|i| (i.modifiers.shift, i.modifiers.command));
                self.pick_def_rows(i, shift, ctrl, order);
                if !shift {
                    self.def_row_anchor = Some((true, i));
                }
            }
            BoneCmd::Move(bones, to) => {
                if let Some(s) = &mut self.session {
                    move_chains(s, &bones, to);
                }
            }
            BoneCmd::NewFolder(bones) => {
                self.bone_pal.dialog =
                    Some(FolderDialog { folder: None, name: String::new(), bones });
            }
            BoneCmd::RenameFolder(fi) => {
                let Some(s) = &self.session else { return };
                let Some(f) = s.bone_folders.get(fi) else { return };
                self.bone_pal.dialog = Some(FolderDialog {
                    folder: Some(fi),
                    name: f.name.clone(),
                    bones: Vec::new(),
                });
            }
            BoneCmd::DeleteFolder(fi) => {
                let Some(s) = &mut self.session else { return };
                if fi < s.bone_folders.len() {
                    let f = s.bone_folders.remove(fi);
                    s.modified = true;
                    log::info!(
                        "bone folder {} deleted; its {} chain(s) moved to the top level",
                        f.name,
                        f.bones.len()
                    );
                }
            }
            BoneCmd::ToggleFolder(fi) => {
                if let Some(f) = self.session.as_mut().and_then(|s| s.bone_folders.get_mut(fi)) {
                    f.open = !f.open;
                }
            }
            BoneCmd::Fold(i) => {
                if !self.def_collapsed.remove(&i) {
                    self.def_collapsed.insert(i);
                }
            }
            BoneCmd::Rename(i) => self.open_def_rename(i),
            BoneCmd::Delete(i) => self.run_def_cmd(DefCmd::Delete(i)),
        }
    }

    /// New Folder and Rename Folder for the Bones palette.
    pub(super) fn bone_folder_window(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.bone_pal.dialog else { return };
        let (mut open, mut ok) = (true, false);
        let title = if dlg.folder.is_some() { tr("Rename Folder") } else { tr("New Folder") };
        crate::windows::Dialog::new(title).open(&mut open).show(ctx, |ui| {
            ui.set_width(220.0);
            let edit = ui.text_edit_singleline(&mut dlg.name);
            if ui.memory(|m| m.focused().is_none()) {
                edit.request_focus();
            }
            ok |= edit.lost_focus() && ui.input(|k| k.key_pressed(egui::Key::Enter));
            ok |= ui.button(tr("OK")).clicked();
        });
        if ok {
            let Some(dlg) = self.bone_pal.dialog.take() else { return };
            let Some(s) = &mut self.session else { return };
            let name = dlg.name.trim().to_string();
            match dlg.folder {
                Some(fi) => {
                    if let Some(f) = s.bone_folders.get_mut(fi).filter(|_| !name.is_empty()) {
                        log::info!("bone folder {} renamed to {name}", f.name);
                        f.name = name;
                        s.modified = true;
                    }
                }
                None => {
                    let name = if name.is_empty() { "Folder".to_string() } else { name };
                    log::info!("bone folder {name} made");
                    s.bone_folders.push(BoneFolder { name, bones: Vec::new(), open: true });
                    s.modified = true;
                    let fi = s.bone_folders.len() - 1;
                    move_chains(s, &dlg.bones, Some(fi));
                }
            }
        } else if !open {
            self.bone_pal.dialog = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_core::bone_edit::create_bone;

    /// A (with child B) and C, A filed in "hair".
    fn session() -> Session {
        let ctx = egui::Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap();
        s.edit(|m| {
            let a = create_bone(m, None, None, (0.0, 0.0), (0.0, 1.0), "A".into()).unwrap();
            create_bone(m, Some(a), None, (0.0, 1.0), (0.0, 2.0), "B".into()).unwrap();
            create_bone(m, None, None, (1.0, 0.0), (1.0, 1.0), "C".into()).is_some()
        });
        s.bone_folders = vec![BoneFolder { name: "hair".into(), bones: vec![0], open: true }];
        s
    }

    fn shown(rows: &[Row]) -> Vec<(Option<usize>, usize, usize)> {
        rows.iter()
            .filter_map(|r| match *r {
                Row::Bone { bone, depth, folder, .. } => Some((folder, bone, depth)),
                Row::Folder(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_child_shows_under_its_root_in_the_roots_folder() {
        let s = session();
        let all = rows(&s, &Default::default(), "");
        assert!(matches!(all[0], Row::Folder(0)));
        assert_eq!(shown(&all), vec![(Some(0), 0, 0), (Some(0), 1, 1), (None, 2, 0)]);
    }

    #[test]
    fn moving_a_child_moves_its_whole_chain() {
        let mut s = session();
        move_chains(&mut s, &[1], None);
        assert!(s.bone_folders[0].bones.is_empty());
        move_chains(&mut s, &[1, 2], Some(0));
        assert_eq!(s.bone_folders[0].bones, vec![0, 2]);
        assert!(s.modified);
    }

    #[test]
    fn a_bone_moved_under_another_chain_leaves_its_folder() {
        let mut s = session();
        s.bone_folders[0].bones = vec![0, 2, 2, 7];
        s.edit(|m| {
            m.deformers[2].parent_deformer = Some(1);
            true
        });
        tidy(&mut s.bone_folders, s.puppet.model());
        assert_eq!(s.bone_folders[0].bones, vec![0]);
    }

    #[test]
    fn a_search_shows_the_matches_and_their_parents() {
        let s = session();
        let found = rows(&s, &Default::default(), "b");
        assert_eq!(shown(&found), vec![(Some(0), 0, 0), (Some(0), 1, 1)]);
        // a folded bone still shows what matches under it while searching
        let folded = [0usize].into_iter().collect();
        assert_eq!(shown(&rows(&s, &folded, "")), vec![(Some(0), 0, 0), (None, 2, 0)]);
        assert_eq!(shown(&rows(&s, &folded, "b")).len(), 2);
    }
}
