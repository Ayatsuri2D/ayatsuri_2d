// Bones in the editor, behind Settings > Experimental > Bones. The Bone
// tool drags a bone from joint to tip; a drag that starts on any bone's
// tip or joint starts there and makes a child of that bone, so a chain is
// drawn joint after joint. A
// click picks a bone, Ctrl or Shift+click adds one to the pick or takes it
// out, and the arrow's marquee from empty canvas picks every bone it
// touches. Several picked bones turn, slide and scale together, each
// about its own joint (see `multi_drag`). Weight
// Paint brushes one bone's weights on a mesh over a heat map (blue none,
// red all), one undo step per stroke. The Inspector edits a bone's keyed
// values, its length and what it takes from its parent, its IK, and a
// mesh's weights.
//
// The canvas borrows the session while it draws, so the tools here take
// what they need and hand back what to do.

use std::collections::BTreeMap;

use ayatsuri_core::bone_edit::{self, BindError, BoneField, IkError, WeightBrush};
use ayatsuri_core::edit::Target;
use ayatsuri_format::model::{DeformerKind, Inherit, Model};
use egui::{Color32, Painter, Pos2, Response, Stroke, Ui, vec2};

use crate::i18n::{tr, trf};
use crate::session::Session;
use crate::theme;

/// Weight binding and painting settings, in Tool details.
pub(super) struct BoneOpts {
    /// Auto weights: 0 keeps each vertex on its nearest bone, 1 blends far.
    pub(super) smoothness: f32,
    pub(super) brush: WeightBrush,
    /// Brush radius in screen pixels.
    pub(super) radius: f32,
    pub(super) strength: f32,
    /// What Replace paints toward.
    pub(super) value: f32,
}

impl Default for BoneOpts {
    fn default() -> Self {
        Self { smoothness: 0.5, brush: WeightBrush::Add, radius: 40.0, strength: 0.5, value: 1.0 }
    }
}

/// What the bone tools work on between frames.
#[derive(Default)]
pub(super) struct BoneState {
    /// Bones picked on the canvas, in pick order: for Add IK the last is
    /// the target.
    pub(super) pick: Vec<usize>,
    /// The mesh weights go on, by id, so it survives edits that renumber.
    pub(super) mesh: Option<String>,
    /// The bone Weight Paint paints.
    pub(super) active: Option<usize>,
    drag: Option<NewBone>,
    stroke: Option<PaintStroke>,
    /// The arrow's marquee: where it started.
    marquee: Option<Pos2>,
    /// Several picked bones being dragged with the arrow.
    multi: Option<MultiDrag>,
}

/// A bone being dragged out.
struct NewBone {
    joint: Pos2,
    parent: Option<usize>,
}

/// A weight stroke being brushed: per vertex, how far it has gone.
struct PaintStroke {
    mesh: usize,
    bone: usize,
    hits: BTreeMap<usize, f32>,
}

impl BoneState {
    /// The mesh weights go on, by index now.
    pub(super) fn mesh_index(&self, model: &Model) -> Option<usize> {
        let id = self.mesh.as_deref()?;
        model.art_meshes.iter().position(|m| m.id == id)
    }

    /// A multi-bone drag cut short (the pick or the tool changed): its
    /// live pose goes back and nothing is edited.
    pub(super) fn drop_multi(&mut self, s: &mut Session) {
        if let Some(d) = self.multi.take() {
            put_back(s, &d.live);
        }
    }

    /// The picked bones that still are bones.
    pub(super) fn bones(&self, model: &Model) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for &b in &self.pick {
            if is_bone(model, b) && !out.contains(&b) {
                out.push(b);
            }
        }
        out
    }

    /// Follow the selection: a picked mesh becomes the weights' mesh, a
    /// bone selected elsewhere (the palette) the pick when it is not in it,
    /// several bones picked there (`group`) the pick, and nothing selected
    /// clears it.
    pub(super) fn follow(
        &mut self,
        model: &Model,
        nothing: bool,
        mesh: Option<usize>,
        deformer: Option<usize>,
        group: &[usize],
    ) {
        if let Some(m) = mesh.and_then(|m| model.art_meshes.get(m)) {
            self.mesh = Some(m.id.clone());
        }
        if nothing {
            self.pick.clear();
        }
        let many: Vec<usize> = group.iter().copied().filter(|&d| is_bone(model, d)).collect();
        if many.len() > 1 {
            if many != self.pick {
                self.pick = many;
            }
        } else if let Some(d) = deformer.filter(|&d| is_bone(model, d))
            && !self.pick.contains(&d)
        {
            self.pick = vec![d];
        }
        self.pick.retain(|&b| is_bone(model, b));
        if self.active.is_some_and(|b| !is_bone(model, b)) {
            self.active = None;
        }
    }
}

fn is_bone(model: &Model, d: usize) -> bool {
    model.deformers.get(d).is_some_and(|x| x.kind.is_bone())
}

/// Model units to the screen and back: screen = origin + (canvas origin +
/// unit * pixels per unit) * scale.
#[derive(Clone, Copy)]
pub(super) struct View {
    pub(super) origin: Pos2,
    pub(super) scale: f32,
    pub(super) canvas: ayatsuri_format::model::Canvas,
}

impl View {
    pub(super) fn screen(self, (x, y): (f32, f32)) -> Pos2 {
        let c = self.canvas;
        Pos2::new(
            self.origin.x + (c.origin_x + x * c.pixels_per_unit) * self.scale,
            self.origin.y + (c.origin_y + y * c.pixels_per_unit) * self.scale,
        )
    }

    fn model(self, p: Pos2) -> (f32, f32) {
        let c = self.canvas;
        let k = (self.scale * c.pixels_per_unit).abs().max(1e-9).copysign(c.pixels_per_unit);
        let k = if k == 0.0 { 1e-9 } else { k };
        (
            (p.x - self.origin.x - c.origin_x * self.scale) / k,
            (p.y - self.origin.y - c.origin_y * self.scale) / k,
        )
    }
}

/// Each shown bone's joint and tip on screen.
pub(super) fn segments(s: &Session, view: View) -> Vec<(usize, Pos2, Pos2)> {
    let model = s.puppet.model();
    (0..model.deformers.len())
        .filter(|&d| is_bone(model, d) && s.deformer_shown.get(d).copied().unwrap_or(true))
        .filter_map(|d| {
            let (j, t) = bone_edit::bone_ends(&s.puppet, d)?;
            Some((d, view.screen(j), view.screen(t)))
        })
        .collect()
}

fn to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 1e-6 {
        ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Screen pixels within which a bone is under the pointer.
const REACH: f32 = 6.0;

/// Whether `p` is on the bone from `a` to `b`.
pub(super) fn near_segment(p: Pos2, a: Pos2, b: Pos2) -> bool {
    to_segment(p, a, b) <= REACH
}

/// The bone under `p`: the nearest within reach.
pub(super) fn bone_at(segs: &[(usize, Pos2, Pos2)], p: Pos2) -> Option<usize> {
    segs.iter()
        .map(|&(d, a, b)| (d, to_segment(p, a, b)))
        .filter(|x| x.1 <= REACH)
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|x| x.0)
}

/// A bone as a tapered shape: widest a little past the joint, a point at
/// the tip, a ring at the joint.
fn bone_shape(vp: &Painter, a: Pos2, b: Pos2, color: Color32, strong: bool) {
    let ab = b - a;
    let len = ab.length();
    if len < 1.0 {
        vp.circle_stroke(a, 4.0, Stroke::new(1.4, color));
        return;
    }
    let dir = ab / len;
    let side = vec2(-dir.y, dir.x) * (len * 0.12).clamp(2.5, 9.0);
    let wide = a + ab * 0.18;
    let pts = vec![a, wide + side, b, wide - side];
    let fill = color.gamma_multiply(if strong { 0.45 } else { 0.25 });
    vp.add(egui::Shape::convex_polygon(
        pts,
        fill,
        Stroke::new(if strong { 1.8 } else { 1.2 }, color),
    ));
    vp.circle_stroke(a, 3.5, Stroke::new(1.4, color));
}

/// Every shown bone, the picked ones in the accent.
pub(super) fn draw_bones(
    vp: &Painter,
    segs: &[(usize, Pos2, Pos2)],
    picked: &[usize],
    hover: Option<usize>,
    color: Color32,
) {
    for &(d, a, b) in segs {
        let on = picked.contains(&d);
        let col = if on || hover == Some(d) { theme::pal().accent } else { color };
        bone_shape(vp, a, b, col, on);
    }
}

/// What a canvas tool asks for.
pub(super) enum BoneAct {
    /// Pick a bone (None: clear the pick), or add it to the pick.
    Pick {
        bone: Option<usize>,
        add: bool,
    },
    /// The bones a marquee caught: the pick, or added to it.
    PickMany {
        bones: Vec<usize>,
        add: bool,
    },
    Create {
        parent: Option<usize>,
        joint: (f32, f32),
        tip: (f32, f32),
    },
    Paint {
        mesh: usize,
        bone: usize,
        hits: Vec<(usize, f32)>,
    },
}

/// The Bone tool: click picks, a drag draws a bone. `selected` is the bone
/// whose tip starts a child, `parent` where a bone drawn elsewhere goes.
pub(super) fn bone_tool(
    ui: &Ui,
    vp: &Painter,
    resp: &Response,
    st: &mut BoneState,
    segs: &[(usize, Pos2, Pos2)],
    view: View,
    selected: Option<usize>,
    parent: Option<usize>,
) -> Option<BoneAct> {
    let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
    let mods = ui.ctx().input(|i| i.modifiers);
    // any bone's end or start under the pointer: a drag from it starts
    // there and makes a child of that bone; a tip wins over a joint on
    // the same spot (the next bone of a chain), the selected bone first
    let on_tip = |p: Pos2| {
        let near = |q: Pos2| q.distance(p) < 9.0;
        let order = segs.iter().filter(|g| Some(g.0) == selected).chain(segs.iter());
        let ends: Vec<_> = order.collect();
        ends.iter()
            .find(|g| near(g.2))
            .map(|g| (g.0, g.2))
            .or_else(|| ends.iter().find(|g| near(g.1)).map(|g| (g.0, g.1)))
    };
    if st.drag.is_none() {
        // the ends a chain can grow from
        for g in segs {
            vp.circle_filled(g.2, 2.5, theme::pal().accent.gamma_multiply(0.6));
        }
        if let Some(p) = ptr
            && resp.hovered()
            && let Some((_, t)) = on_tip(p)
        {
            vp.circle_stroke(t, 6.0, Stroke::new(1.5, theme::pal().accent));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
    let mut act = None;
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        act = Some(BoneAct::Pick { bone: bone_at(segs, p), add: mods.command || mods.shift });
    }
    if resp.drag_started() {
        let start = ui.ctx().input(|i| i.pointer.press_origin()).or(ptr);
        if let Some(start) = start {
            st.drag = Some(match on_tip(start) {
                Some((d, t)) => NewBone { joint: t, parent: Some(d) },
                None => NewBone { joint: start, parent },
            });
        }
    }
    if let (Some(d), Some(p)) = (&st.drag, ptr) {
        bone_shape(vp, d.joint, p, theme::pal().accent, true);
    }
    if resp.drag_stopped()
        && let Some(d) = st.drag.take()
        && let Some(p) = ptr
        && d.joint.distance(p) > 4.0
    {
        act = Some(BoneAct::Create {
            parent: d.parent,
            joint: view.model(d.joint),
            tip: view.model(p),
        });
    }
    act
}

/// Whether a press at `p` is on no bone and on no handle of a picked
/// one, so the arrow's marquee may start there.
pub(super) fn off_bones(
    segs: &[(usize, Pos2, Pos2)],
    picked: &[usize],
    ring: f32,
    p: Pos2,
) -> bool {
    bone_at(segs, p).is_none()
        && !segs.iter().any(|g| g.1.distance(p) < 9.0 || g.2.distance(p) < 9.0)
        && !segs
            .iter()
            .filter(|g| picked.contains(&g.0))
            .any(|g| (g.1.distance(p) - ring).abs() < 6.0)
}

/// The arrow's marquee over bones: a drag that starts where `start_ok`
/// says draws a box, and on release the bones it touches are picked; Ctrl
/// or Shift adds them to the pick.
pub(super) fn marquee(
    ui: &Ui,
    vp: &Painter,
    resp: &Response,
    st: &mut BoneState,
    segs: &[(usize, Pos2, Pos2)],
    start_ok: impl Fn(Pos2) -> bool,
) -> Option<BoneAct> {
    let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
    if resp.drag_started()
        && let Some(p) = ui.ctx().input(|i| i.pointer.press_origin()).or(ptr)
        && start_ok(p)
    {
        st.marquee = Some(p);
    }
    let a = st.marquee?;
    let r = egui::Rect::from_two_pos(a, ptr.unwrap_or(a));
    let accent = theme::pal().accent;
    vp.rect_filled(r, 0.0, accent.gamma_multiply(0.08));
    vp.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Middle);
    let down = ui.ctx().input(|i| i.pointer.any_down());
    if !resp.drag_stopped() && down {
        return None;
    }
    st.marquee = None;
    // a bone is caught when any of it is inside the box
    let touches = |j: Pos2, t: Pos2| (0..=16).any(|i| r.contains(j.lerp(t, i as f32 / 16.0)));
    let bones: Vec<usize> = segs.iter().filter(|g| touches(g.1, g.2)).map(|g| g.0).collect();
    let mods = ui.ctx().input(|i| i.modifiers);
    Some(BoneAct::PickMany { bones, add: mods.command || mods.shift })
}

/// What a drag on several picked bones holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grab {
    Turn,
    Joint,
    Scale,
}

/// Several picked bones dragged together with the arrow.
pub(super) struct MultiDrag {
    grab: Grab,
    /// The grabbed bone: the turn and the ring go round its joint.
    bone: usize,
    /// That joint on screen at the press.
    pivot: Pos2,
    /// Ctrl when the drag began: every picked bone moves alone.
    keep: bool,
    /// The picked bones.
    bones: Vec<usize>,
    degrees: f32,
    last: Pos2,
    /// The joint drag so far, in model units.
    wx: f32,
    wy: f32,
    scale_from: f32,
    factor: f32,
    /// Without Ctrl: each moving bone as it was, for the live preview,
    /// put back before the edit.
    live: Vec<LiveBone>,
}

struct LiveBone {
    bone: usize,
    keyform: usize,
    angle: Option<f32>,
    scale: Option<f32>,
    origin: Option<(f32, f32)>,
    /// Its origin in its parent's space at the press.
    local: (f32, f32),
    parent: Option<u32>,
}

impl MultiDrag {
    /// The turn so far; Shift holds it to 45 degree steps.
    fn turn(&self, shift: bool) -> f32 {
        if shift { (self.degrees / 45.0).round() * 45.0 } else { self.degrees }
    }
}

/// With Ctrl nothing in the model moves until the release, so the bone
/// overlay shows the new pose: each picked bone alone turned or scaled
/// about its own joint, or its joint slid. Bones under them that are not
/// picked stay where they are, as the edit keeps them. True while it
/// shows.
pub(super) fn ctrl_preview(
    st: &BoneState,
    segs: &mut [(usize, Pos2, Pos2)],
    view: View,
    shift: bool,
) -> bool {
    let Some(d) = st.multi.as_ref().filter(|d| d.keep) else { return false };
    let v = vec2(d.wx, d.wy) * (view.scale * view.canvas.pixels_per_unit);
    let (sin, cos) = match d.grab {
        Grab::Turn => d.turn(shift).to_radians().sin_cos(),
        _ => (0.0, 1.0),
    };
    let k = if d.grab == Grab::Scale { d.factor } else { 1.0 };
    for g in segs.iter_mut().filter(|g| d.bones.contains(&g.0)) {
        if d.grab == Grab::Joint {
            g.1 += v;
            g.2 += v;
        } else {
            let t = (g.2 - g.1) * k;
            g.2 = g.1 + vec2(t.x * cos - t.y * sin, t.x * sin + t.y * cos);
        }
    }
    true
}

/// The live bones back as they were before the drag.
fn put_back(s: &mut Session, live: &[LiveBone]) {
    for l in live {
        let k = l.keyform;
        if let (Some(v), Some(a)) =
            (l.angle, s.puppet.rotation_angles_mut(l.bone).and_then(|a| a.get_mut(k)))
        {
            *a = v;
        }
        if let (Some(v), Some(x)) =
            (l.scale, s.puppet.rotation_scales_mut(l.bone).and_then(|a| a.get_mut(k)))
        {
            *x = v;
        }
        if let (Some((bx, by)), Some((x, y))) = (l.origin, s.puppet.rotation_origin_mut(l.bone, k))
        {
            (*x, *y) = (bx, by);
        }
    }
    s.dirty = true;
}

/// The arrow on several picked bones: drag any picked bone's body to turn
/// them all by the same angle (Shift in 45 degree steps), its joint to
/// slide every joint by the same screen distance, or the ring round its
/// joint to scale them, each about its own joint. Without Ctrl the art
/// bends live and each bone changes in the keyform the parameters sit on;
/// a picked bone under another picked one rides along with it instead of
/// turning twice, and one between keys is left alone with a warning. With
/// Ctrl every picked bone moves alone in every keyform and its weighted
/// meshes are bound again where they sit. Either way one undo step.
#[allow(clippy::too_many_arguments)]
pub(super) fn multi_drag(
    ui: &Ui,
    vp: &Painter,
    resp: &Response,
    st: &mut BoneState,
    s: &mut Session,
    view: View,
    picked: &[usize],
    ring: f32,
    line: Color32,
) {
    let ends: Vec<(usize, Pos2, Pos2)> = picked
        .iter()
        .filter_map(|&b| {
            let (j, t) = bone_edit::bone_ends(&s.puppet, b)?;
            Some((b, view.screen(j), view.screen(t)))
        })
        .collect();
    let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
    let press = ui.ctx().input(|i| i.pointer.press_origin());
    let at = if resp.drag_started() { press.or(ptr) } else { ptr };
    let idle = st.multi.is_none() && st.marquee.is_none();
    let on_joint = at.filter(|_| idle).and_then(|p| ends.iter().find(|g| g.1.distance(p) < 9.0));
    let on_body = at
        .filter(|_| idle && on_joint.is_none())
        .and_then(|p| ends.iter().find(|g| g.2.distance(p) < 9.0 || near_segment(p, g.1, g.2)));
    let on_ring = at
        .filter(|_| idle && on_joint.is_none() && on_body.is_none())
        .and_then(|p| ends.iter().find(|g| (p.distance(g.1) - ring).abs() < 5.0));
    let k_unit = view.scale * view.canvas.pixels_per_unit;
    let k_unit = if k_unit.abs() < 1e-9 { 1e-9 } else { k_unit };
    let shift = ui.input(|i| i.modifiers.shift);
    if resp.drag_started()
        && let Some(start) = press.or(ptr)
        && let Some((grab, g)) = on_joint
            .map(|g| (Grab::Joint, g))
            .or(on_body.map(|g| (Grab::Turn, g)))
            .or(on_ring.map(|g| (Grab::Scale, g)))
    {
        let keep = ui.input(|i| i.modifiers.command);
        let mut live = Vec::new();
        if !keep {
            let values = values(s);
            let model = s.puppet.model();
            for b in bone_edit::top_bones(model, picked) {
                let Some(k) =
                    ayatsuri_core::keyform::keyform_at(model, Target::Deformer(b), &values)
                else {
                    continue;
                };
                let Some(local) = s.puppet.rotation_local_origin(b) else { continue };
                let r = model.deformers.get(b).and_then(|d| d.kind.rotation());
                live.push(LiveBone {
                    bone: b,
                    keyform: k,
                    angle: r.and_then(|r| r.angles.get(k).copied()),
                    scale: r.and_then(|r| r.scales.get(k).copied()),
                    origin: r.and_then(|r| Some((*r.origins_x.get(k)?, *r.origins_y.get(k)?))),
                    local,
                    parent: model.deformers.get(b).and_then(|d| d.parent_deformer),
                });
            }
        }
        // the way already come before the drag started, less this frame's
        // step, which the drag adds
        let ahead = (ptr.unwrap_or(start) - start) - resp.drag_delta();
        let joint = grab == Grab::Joint;
        st.multi = Some(MultiDrag {
            grab,
            bone: g.0,
            pivot: g.1,
            keep,
            bones: picked.to_vec(),
            degrees: 0.0,
            last: start,
            wx: if joint { ahead.x / k_unit } else { 0.0 },
            wy: if joint { ahead.y / k_unit } else { 0.0 },
            scale_from: (start - g.1).length().max(1.0),
            factor: 1.0,
            live,
        });
    }
    if resp.dragged()
        && let Some(p) = ptr
        && let Some(d) = &mut st.multi
    {
        match d.grab {
            Grab::Turn => {
                let (prev, cur) = (d.last - d.pivot, p - d.pivot);
                if prev.length() > 2.0 && cur.length() > 2.0 {
                    let mut diff = cur.y.atan2(cur.x) - prev.y.atan2(prev.x);
                    while diff > std::f32::consts::PI {
                        diff -= std::f32::consts::TAU;
                    }
                    while diff < -std::f32::consts::PI {
                        diff += std::f32::consts::TAU;
                    }
                    d.degrees += diff.to_degrees();
                }
                d.last = p;
            }
            Grab::Scale => d.factor = ((p - d.pivot).length() / d.scale_from).clamp(0.05, 20.0),
            Grab::Joint => {
                let delta = resp.drag_delta();
                d.wx += delta.x / k_unit;
                d.wy += delta.y / k_unit;
            }
        }
        // without Ctrl the art follows live, in the keyform each bone's
        // parameters sit on
        let turn = d.turn(shift);
        for l in &d.live {
            let k = l.keyform;
            match d.grab {
                Grab::Turn => {
                    if let (Some(v), Some(a)) =
                        (l.angle, s.puppet.rotation_angles_mut(l.bone).and_then(|a| a.get_mut(k)))
                    {
                        *a = v + turn;
                    }
                }
                Grab::Scale => {
                    if let (Some(v), Some(x)) =
                        (l.scale, s.puppet.rotation_scales_mut(l.bone).and_then(|a| a.get_mut(k)))
                    {
                        *x = v * d.factor;
                    }
                }
                Grab::Joint => {
                    let (dx, dy) =
                        super::parent_delta(&s.puppet, l.parent, l.local.0, l.local.1, d.wx, d.wy);
                    if let (Some((bx, by)), Some((x, y))) =
                        (l.origin, s.puppet.rotation_origin_mut(l.bone, k))
                    {
                        (*x, *y) = (bx + dx, by + dy);
                    }
                }
            }
        }
        if !d.live.is_empty() {
            s.dirty = true;
        }
        ui.ctx().request_repaint();
    }
    let down = ui.ctx().input(|i| i.pointer.any_down());
    if (resp.drag_stopped() || !down)
        && let Some(d) = st.multi.take()
    {
        put_back(s, &d.live);
        commit(s, &d, picked, shift);
    }
    // the joints and their scale rings
    let active = st.multi.as_ref();
    for g in &ends {
        let held = active.filter(|d| d.bone == g.0);
        let scaling = held.filter(|d| d.grab == Grab::Scale);
        let hot = on_ring.is_some_and(|h| h.0 == g.0) || scaling.is_some();
        let stroke =
            if hot { Stroke::new(2.2, theme::pal().accent) } else { Stroke::new(1.2, line) };
        vp.circle_stroke(g.1, ring * scaling.map_or(1.0, |d| d.factor), stroke);
        let dot = on_joint.is_some_and(|h| h.0 == g.0);
        vp.circle_filled(g.1, if dot { 4.5 } else { 3.0 }, theme::pal().accent);
    }
    if on_joint.is_some() || on_body.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    } else if on_ring.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }
}

/// The release: one edit, one undo step, for every picked bone.
fn commit(s: &mut Session, d: &MultiDrag, picked: &[usize], shift: bool) {
    let values = values(s);
    let deg = d.turn(shift);
    let (wx, wy) = (d.wx, d.wy);
    let moved = match d.grab {
        Grab::Turn => deg.abs() > 1e-3,
        Grab::Scale => (d.factor - 1.0).abs() > 1e-4,
        Grab::Joint => wx != 0.0 || wy != 0.0,
    };
    if !moved {
        return;
    }
    let n = picked.len();
    if d.keep {
        // Ctrl: each bone alone; a joint slide moves each bone's pivot
        let mut pivots: Vec<bone_edit::Pivot> = Vec::new();
        let mut between = 0;
        if d.grab == Grab::Joint {
            for &b in picked {
                match super::pivot_move(&s.puppet, b, wx, wy) {
                    Some((deltas, child)) => pivots.push((b, deltas, child)),
                    None => between += 1,
                }
            }
        }
        let mv = match d.grab {
            Grab::Turn => bone_edit::BonesMove::Turn(deg),
            Grab::Scale => bone_edit::BonesMove::Scale(d.factor),
            Grab::Joint => bone_edit::BonesMove::Pivots(&pivots),
        };
        if s.edit(|m| bone_edit::move_bones_alone(m, picked, &values, mv)) {
            log::debug!("{n} bones moved alone: their meshes rebound");
        }
        if between > 0 {
            log::warn!(
                "the parameters are between keys: move them onto a key to move the pivot of {between} picked bone(s)"
            );
        }
        return;
    }
    let joints: Vec<(usize, f32, f32)> = d
        .live
        .iter()
        .map(|l| {
            let (dx, dy) = super::parent_delta(&s.puppet, l.parent, l.local.0, l.local.1, wx, wy);
            (l.bone, dx, dy)
        })
        .collect();
    let mv = match d.grab {
        Grab::Turn => bone_edit::BonesMove::Turn(deg),
        Grab::Scale => bone_edit::BonesMove::Scale(d.factor),
        Grab::Joint => bone_edit::BonesMove::Joints(&joints),
    };
    let mut skipped = Vec::new();
    if s.edit(|m| {
        let (changed, left) = bone_edit::move_bones(m, picked, &values, mv);
        skipped = left;
        changed
    }) {
        log::debug!("{n} bones moved together");
    }
    if !skipped.is_empty() {
        log::warn!(
            "the parameters are between keys: move them onto a key to move {} picked bone(s)",
            skipped.len()
        );
    }
}

/// Blue at 0 through green to red at 1.
pub(super) fn heat(w: f32) -> Color32 {
    let w = w.clamp(0.0, 1.0);
    let (r, g, b) = if w < 0.5 {
        let t = w * 2.0;
        (0.0, t, 1.0 - t)
    } else {
        let t = (w - 0.5) * 2.0;
        (t, 1.0 - t, 0.0)
    };
    Color32::from_rgba_unmultiplied((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 150)
}

/// Weight Paint: the mesh's weights for the active bone as a heat map, the
/// brush ring, and the stroke; on release, the stroke to apply. A click on
/// a bone makes it the active one. `pen` is the pen's size and strength.
#[allow(clippy::too_many_arguments)]
pub(super) fn weight_paint(
    ui: &Ui,
    vp: &Painter,
    resp: &Response,
    st: &mut BoneState,
    s: &Session,
    segs: &[(usize, Pos2, Pos2)],
    view: View,
    opts: &BoneOpts,
    pen: (f32, f32),
) -> Option<BoneAct> {
    let model = s.puppet.model();
    let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some(b) = bone_at(segs, p)
    {
        st.active = Some(b);
        return None;
    }
    let (Some(mi), Some(bone)) = (st.mesh_index(model), st.active) else { return None };
    let g = s.puppet.mesh_geometry(mi)?;
    let pts: Vec<Pos2> =
        g.positions.as_chunks::<2>().0.iter().map(|&[x, y]| view.screen((x, y))).collect();
    let mut weights = bone_edit::vertex_weights(model, mi, bone as u32);
    weights.resize(pts.len(), 0.0);
    // the stroke so far, shown before it lands
    if let Some(stroke) = st.stroke.as_ref().filter(|k| k.mesh == mi && k.bone == bone) {
        for (&v, &a) in &stroke.hits {
            if let Some(w) = weights.get_mut(v) {
                *w = match opts.brush {
                    WeightBrush::Add => *w + a,
                    WeightBrush::Subtract => *w - a,
                    WeightBrush::Replace => *w + (opts.value - *w) * a,
                    WeightBrush::Smooth => *w,
                }
                .clamp(0.0, 1.0);
            }
        }
    }
    let mut mesh = egui::Mesh::default();
    for (p, w) in pts.iter().zip(&weights) {
        mesh.colored_vertex(*p, heat(*w));
    }
    for &[a, b, c] in g.indices.as_chunks::<3>().0 {
        mesh.add_triangle(a as u32, b as u32, c as u32);
    }
    if !mesh.indices.iter().any(|&i| i as usize >= pts.len()) {
        vp.add(egui::Shape::mesh(mesh));
    }
    let r = opts.radius.clamp(2.0, 400.0) * pen.0;
    if let Some(p) = ptr {
        vp.circle_stroke(p, r, Stroke::new(1.2, Color32::WHITE));
    }
    if (resp.dragged() || resp.drag_started())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.001, 0.1);
        let stroke =
            st.stroke.get_or_insert_with(|| PaintStroke { mesh: mi, bone, hits: BTreeMap::new() });
        for (v, q) in pts.iter().enumerate() {
            let d = q.distance(p);
            if d <= r {
                let falloff = (1.0 - d / r).powi(2);
                let e = stroke.hits.entry(v).or_insert(0.0);
                *e = (*e + opts.strength * pen.1 * falloff * dt * 4.0).min(1.0);
            }
        }
    }
    if resp.drag_stopped()
        && let Some(k) = st.stroke.take()
        && !k.hits.is_empty()
    {
        return Some(BoneAct::Paint {
            mesh: k.mesh,
            bone: k.bone,
            hits: k.hits.into_iter().collect(),
        });
    }
    None
}

/// What a failed bind says.
pub(super) fn bind_message(e: BindError) -> &'static str {
    match e {
        BindError::NoMesh => tr("Pick a mesh first."),
        BindError::NotBone => tr("Pick one or more bones first."),
        BindError::NoCommonParent => tr(
            "These bones sit under different warp deformers, so no one parent holds them all. Weight the mesh to bones under the same warp, or to bones with no warp above them.",
        ),
    }
}

/// What a failed Add IK says.
pub(super) fn ik_message(e: IkError) -> &'static str {
    match e {
        IkError::ChainLength => {
            tr("Pick one or two bones for the chain, then the target bone last.")
        }
        IkError::NotChain => tr("The two chain bones must be a bone and its child."),
        IkError::NotBone => tr("The chain and the target must all be bones."),
        IkError::TargetInChain => tr("The target can't be in the chain or under it."),
        IkError::Taken => tr("A bone of this chain already has IK."),
        IkError::Order => tr("The target can't be placed before the chain."),
    }
}

/// A Drag value that edits the model: a drag is one undo step, a typed
/// value another.
fn edit_value(
    ui: &mut Ui,
    s: &mut Session,
    value: &mut f32,
    speed: f64,
    suffix: &str,
    apply: impl FnOnce(&mut Model, f32) -> bool,
) {
    let r = ui.add(egui::DragValue::new(value).speed(speed).max_decimals(3).suffix(suffix));
    let live = r.id.with("bone-live");
    let held = ui.ctx().data(|d| d.get_temp::<bool>(live)).unwrap_or(false);
    if r.changed() {
        let v = *value;
        if held && r.dragged() {
            s.amend(|m| apply(m, v));
        } else if s.edit(|m| apply(m, v)) && r.dragged() {
            ui.ctx().data_mut(|d| d.insert_temp(live, true));
        }
    }
    if held && !r.dragged() {
        ui.ctx().data_mut(|d| d.remove::<bool>(live));
    }
}

fn label(ui: &mut Ui, text: &str) {
    ui.add_sized(
        [74.0, 16.0],
        egui::Label::new(egui::RichText::new(text).color(theme::pal().text_faint).size(11.0)),
    );
}

fn inherit_name(i: Inherit) -> &'static str {
    match i {
        Inherit::Normal => tr("Everything"),
        Inherit::OnlyTranslation => tr("Position only"),
        Inherit::NoRotationOrReflection => tr("Position and scale"),
        Inherit::NoScale => tr("Position, turn and mirroring"),
        Inherit::NoScaleOrReflection => tr("Position and turn"),
    }
}

fn values(s: &Session) -> Vec<f32> {
    (0..s.puppet.model().parameters.len()).map(|p| s.puppet.value(p)).collect()
}

/// The Inspector's rows for a bone: its keyed values in the keyform the
/// parameters sit on, its length and inherit mode.
pub(super) fn bone_rows(ui: &mut Ui, s: &mut Session, bone: usize) {
    let model = s.puppet.model();
    let Some(DeformerKind::Bone(b)) = model.deformers.get(bone).map(|d| &d.kind) else { return };
    let (mut length, inherit) = (b.length, b.inherit);
    let at = ayatsuri_core::keyform::keyform_at(model, Target::Deformer(bone), &values(s));
    let fields = [
        (BoneField::Angle, tr("Angle"), 0.5, "°"),
        (BoneField::OriginX, tr("Origin X"), 0.01, ""),
        (BoneField::OriginY, tr("Origin Y"), 0.01, ""),
        (BoneField::ScaleX, tr("Scale X"), 0.01, ""),
        (BoneField::ScaleY, tr("Scale Y"), 0.01, ""),
        (BoneField::ShearX, tr("Shear X"), 0.5, "°"),
        (BoneField::ShearY, tr("Shear Y"), 0.5, "°"),
    ];
    for (field, name, speed, suffix) in fields {
        ui.horizontal(|ui| {
            label(ui, name);
            let now = at.and_then(|k| bone_edit::bone_field(s.puppet.model(), bone, k, field));
            match (at, now) {
                (Some(k), Some(mut v)) => edit_value(ui, s, &mut v, speed, suffix, |m, v| {
                    // its weighted meshes bend with it and stay bent
                    let pinned = bone_edit::pin_binds(m, bone);
                    bone_edit::set_bone_field(m, bone, k, field, v) || pinned
                }),
                _ => {
                    ui.add_enabled(false, egui::DragValue::new(&mut 0.0f32))
                        .on_disabled_hover_text(tr(
                            "Between keys. Move the parameters onto a key to set this there.",
                        ));
                }
            }
        });
    }
    ui.horizontal(|ui| {
        label(ui, tr("Length"));
        edit_value(ui, s, &mut length, 0.01, "", |m, v| bone_edit::set_bone_length(m, bone, v));
    });
    ui.horizontal(|ui| {
        label(ui, tr("Inherit"));
        let mut pick = inherit;
        egui::ComboBox::from_id_salt(("bone-inherit", bone))
            .selected_text(inherit_name(inherit))
            .show_ui(ui, |ui| {
                for i in [
                    Inherit::Normal,
                    Inherit::OnlyTranslation,
                    Inherit::NoRotationOrReflection,
                    Inherit::NoScale,
                    Inherit::NoScaleOrReflection,
                ] {
                    ui.selectable_value(&mut pick, i, inherit_name(i));
                }
            })
            .response
            .on_hover_text(tr("What the bone takes from its parent"));
        if pick != inherit && s.edit(|m| bone_edit::set_bone_inherit(m, bone, pick)) {
            log::info!("bone {bone} inherits {pick:?}");
        }
    });
}

/// The Inspector with several bones picked: how many, and what they take
/// from their parents, set on all of them at once.
pub(super) fn multi_rows(ui: &mut Ui, s: &mut Session, bones: &[usize]) {
    ui.label(egui::RichText::new(trf("{count} bones", &[("count", &bones.len())])).size(11.5));
    let model = s.puppet.model();
    let modes: Vec<Inherit> = bones
        .iter()
        .filter_map(|&b| match model.deformers.get(b).map(|d| &d.kind) {
            Some(DeformerKind::Bone(x)) => Some(x.inherit),
            _ => None,
        })
        .collect();
    let Some(&first) = modes.first() else { return };
    let shared = modes.iter().all(|&m| m == first).then_some(first);
    ui.horizontal(|ui| {
        label(ui, tr("Inherit"));
        let mut pick = shared;
        egui::ComboBox::from_id_salt("bones-inherit")
            .selected_text(shared.map_or(tr("Mixed"), inherit_name))
            .show_ui(ui, |ui| {
                for i in [
                    Inherit::Normal,
                    Inherit::OnlyTranslation,
                    Inherit::NoRotationOrReflection,
                    Inherit::NoScale,
                    Inherit::NoScaleOrReflection,
                ] {
                    ui.selectable_value(&mut pick, Some(i), inherit_name(i));
                }
            })
            .response
            .on_hover_text(tr("What every picked bone takes from its parent"));
        if let Some(i) = pick.filter(|_| pick != shared)
            && s.edit(|m| {
                let mut changed = false;
                for &b in bones {
                    changed |= bone_edit::set_bone_inherit(m, b, i);
                }
                changed
            })
        {
            log::info!("{} bones inherit {i:?}", bones.len());
        }
    });
}

/// The Inspector's IK rows for every constraint `bone` is in or is the
/// target of: mix and bend in the IK keyform the parameters sit on, keys
/// on the selected parameter, delete.
pub(super) fn ik_rows(ui: &mut Ui, s: &mut Session, bone: usize, param: Option<usize>) {
    let b32 = bone as u32;
    let list: Vec<usize> = (s.puppet.model().ik.iter().enumerate())
        .filter(|(_, c)| c.bones.contains(&b32) || c.target == b32)
        .map(|(i, _)| i)
        .collect();
    for ci in list {
        let model = s.puppet.model();
        let Some(c) = model.ik.get(ci).cloned() else { continue };
        ui.separator();
        let name = |d: u32| s.deformer_name(d as usize);
        let chain: Vec<String> = c.bones.iter().map(|&d| name(d)).collect();
        ui.label(
            egui::RichText::new(trf(
                "{id}: {chain} reaches for {target}",
                &[("id", &c.id), ("chain", &chain.join(", ")), ("target", &name(c.target))],
            ))
            .size(11.5),
        );
        let at = ayatsuri_core::keyform::keyform_on(model, &c.binding, &values(s));
        ui.horizontal(|ui| {
            label(ui, tr("Mix"));
            match at.and_then(|k| c.mix.get(k).map(|m| (k, *m))) {
                Some((k, mut v)) => {
                    edit_value(ui, s, &mut v, 0.01, "", |m, v| bone_edit::set_ik_mix(m, ci, k, v));
                }
                None => {
                    ui.add_enabled(false, egui::DragValue::new(&mut 0.0f32))
                        .on_disabled_hover_text(tr(
                            "Between keys. Move the parameters onto a key to set this there.",
                        ));
                }
            }
        });
        if c.bones.len() == 2 {
            ui.horizontal(|ui| {
                label(ui, tr("Bend"));
                let now = at.and_then(|k| c.bend_positive.get(k).copied());
                let mut on = now.unwrap_or(true);
                let r = ui.add_enabled(
                    now.is_some(),
                    egui::Checkbox::new(&mut on, tr("Counterclockwise")),
                );
                if r.changed()
                    && let Some(k) = at
                {
                    s.edit(|m| bone_edit::set_ik_bend(m, ci, k, on));
                }
            });
        }
        ui.horizontal(|ui| {
            label(ui, "");
            let pname = param.and_then(|p| s.param_names.get(p).cloned());
            let tip = match &pname {
                Some(n) => trf("On {name}, at its value now", &[("name", n)]),
                None => tr("Select a parameter first").to_string(),
            };
            if ui
                .add_enabled(param.is_some(), egui::Button::new(tr("Add key")))
                .on_hover_text(&tip)
                .clicked()
                && let Some(p) = param
            {
                let v = s.puppet.value(p);
                if s.edit(|m| bone_edit::add_ik_keys(m, ci, p, &[v])) {
                    log::info!("IK {ci}: key added at {v:.3}");
                }
            }
            if ui
                .add_enabled(param.is_some(), egui::Button::new(tr("Delete key")))
                .on_hover_text(&tip)
                .clicked()
                && let Some(p) = param
            {
                let v = s.puppet.value(p);
                s.edit(|m| bone_edit::delete_ik_key(m, ci, p, v));
            }
            if ui.button(tr("Delete IK")).clicked() && s.edit(|m| bone_edit::delete_ik(m, ci)) {
                log::info!("IK {ci} deleted");
            }
        });
    }
}

/// The Inspector's rows for a weighted mesh: its bones, and taking the
/// weights off.
pub(super) fn weight_rows(ui: &mut Ui, s: &mut Session, mesh: usize) {
    let bones = bone_edit::mesh_bones(s.puppet.model(), mesh);
    if bones.is_empty() {
        return;
    }
    ui.separator();
    let names: Vec<String> = bones.iter().map(|&b| s.deformer_name(b as usize)).collect();
    ui.horizontal(|ui| {
        label(ui, tr("Bones"));
        ui.add(egui::Label::new(egui::RichText::new(names.join(", ")).size(11.5)).wrap());
    });
    ui.horizontal(|ui| {
        label(ui, "");
        if ui
            .button(tr("Remove weights"))
            .on_hover_text(tr("The mesh follows its parent deformer alone again"))
            .clicked()
            && s.edit(|m| bone_edit::remove_weights(m, mesh))
        {
            log::info!("mesh {mesh}: weights removed");
        }
        if ui
            .button(tr("Set bind pose"))
            .on_hover_text(tr(
                "Bind the mesh to its bones as they sit now, so it shows its own shape in this pose",
            ))
            .clicked()
        {
            let values = values(s);
            if s.edit(|m| bone_edit::set_bind_pose(m, mesh, &values)) {
                log::info!("mesh {mesh}: bind pose set");
            }
        }
    });
}
