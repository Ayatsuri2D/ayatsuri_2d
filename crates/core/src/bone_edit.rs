//! Editing bones: making them, their keyed values, weights (bound by
//! distance or painted) and IK constraints, and carrying the deformer
//! indices that weights and IK hold through every edit that adds, removes
//! or reorders deformers.
//!
//! These are edits, run once per click or stroke; the frame loop never
//! calls them, so they may allocate.

use ayatsuri_format::model::{
    Binding, BoneData, Deformer, DeformerKind, IkConstraint, Inherit, Model, RotationData, Skin,
};

use crate::Puppet;
use crate::edit::{self, Channel, Child};

/// Weights under this are dropped, by binding and by painting.
const WEIGHT_FLOOR: f32 = 0.01;

/// A vertex's (bone, weight) pairs, one list per vertex. A skin whose
/// offsets do not fit gives every vertex none.
fn pairs_of(skin: Option<&Skin>, vertex_count: usize) -> Vec<Vec<(u32, f32)>> {
    let mut out = vec![Vec::new(); vertex_count];
    let Some(skin) = skin.filter(|s| s.fits(vertex_count)) else { return out };
    for (v, span) in skin.offsets.windows(2).enumerate() {
        let &[a, b] = span else { continue };
        let (a, b) = (a as usize, b as usize);
        if let (Some(slot), Some(bones), Some(ws)) =
            (out.get_mut(v), skin.bones.get(a..b), skin.weights.get(a..b))
        {
            slot.extend(bones.iter().copied().zip(ws.iter().copied()));
        }
    }
    out
}

/// Back from per vertex pairs; None when no vertex keeps a bone.
fn skin_from(pairs: &[Vec<(u32, f32)>]) -> Option<Skin> {
    if pairs.iter().all(Vec::is_empty) {
        return None;
    }
    Some(Skin::from_pairs(pairs.iter().map(Vec::as_slice)))
}

/// A mesh's weights carried through `map`, old bone index to new (None
/// when the bone is gone). A vertex that loses a bone gives its weight to
/// its other bones in proportion, so it keeps its total; one left with
/// none follows its parent alone. None when no vertex keeps a bone.
pub fn remap_skin(
    skin: &Skin,
    vertex_count: usize,
    map: impl Fn(u32) -> Option<u32>,
) -> Option<Skin> {
    if !skin.fits(vertex_count) {
        // offsets that do not fit are not read anyway: carry what can be
        let bones: Option<Vec<u32>> = skin.bones.iter().map(|&b| map(b)).collect();
        return bones.map(|bones| Skin { bones, ..skin.clone() });
    }
    let mut pairs = pairs_of(Some(skin), vertex_count);
    for v in &mut pairs {
        let total: f32 = v.iter().map(|p| p.1).sum();
        let mut kept: Vec<(u32, f32)> =
            v.iter().filter_map(|&(b, w)| map(b).map(|n| (n, w))).collect();
        if kept.len() < v.len() {
            let left: f32 = kept.iter().map(|p| p.1).sum();
            if left > 1e-6 {
                for p in &mut kept {
                    p.1 *= total / left;
                }
            } else {
                kept.clear();
            }
        }
        *v = kept;
    }
    skin_from(&pairs)
}

/// Renumber the deformer indices that bone weights and IK constraints
/// hold: `map` takes an old index to the new one, None when that deformer
/// is gone. Weights on a gone bone go to the vertex's other bones (see
/// [`remap_skin`]); an IK constraint naming a gone bone goes too.
pub(crate) fn remap_refs(model: &mut Model, map: impl Fn(u32) -> Option<u32>) {
    for m in &mut model.art_meshes {
        if let Some(skin) = &m.skin {
            m.skin = remap_skin(skin, m.vertex_count as usize, &map);
        }
    }
    model.ik.retain_mut(|c| {
        let bones: Option<Vec<u32>> = c.bones.iter().map(|&b| map(b)).collect();
        match (bones, map(c.target)) {
            (Some(bones), Some(target)) => {
                c.bones = bones;
                c.target = target;
                true
            }
            _ => false,
        }
    });
}

fn is_bone(model: &Model, d: usize) -> bool {
    model.deformers.get(d).is_some_and(|x| x.kind.is_bone())
}

/// An id no deformer has yet: `BoneN` from the bone count up.
pub fn free_bone_id(model: &Model) -> String {
    let mut n = model.deformers.iter().filter(|d| d.kind.is_bone()).count();
    loop {
        let id = format!("Bone{n}");
        if !model.deformers.iter().any(|d| d.id == id) {
            return id;
        }
        n += 1;
    }
}

/// Where a bone's joint and tip are drawn now, in model units.
pub fn bone_ends(puppet: &Puppet, bone: usize) -> Option<((f32, f32), (f32, f32))> {
    let DeformerKind::Bone(b) = &puppet.model().deformers.get(bone)?.kind else { return None };
    Some((puppet.deformer_point(bone, 0.0, 0.0), puppet.deformer_point(bone, 0.0, b.length)))
}

/// A point in model units.
type Pt = (f32, f32);

fn dist(a: Pt, b: Pt) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Make a bone from `joint` to `tip` (model units, as drawn at rest) under
/// `parent` (None: the root), in `part`. It is placed after every other
/// deformer, so it evaluates after its parent; its one keyform holds the
/// origin and angle that put it there, and its length reaches the tip.
/// Returns its index.
pub fn create_bone(
    model: &mut Model,
    parent: Option<usize>,
    part: Option<u32>,
    joint: (f32, f32),
    tip: (f32, f32),
    id: String,
) -> Option<usize> {
    if parent.is_some_and(|p| model.deformers.get(p).is_none()) || dist(joint, tip) < 1e-6 {
        return None;
    }
    let mut next = model.clone();
    let rest = edit::rest_puppet(&next);
    let (ox, oy) = edit::to_local(&rest, parent, joint);
    let rotation = RotationData {
        base_angle: 0.0,
        opacities: vec![1.0],
        angles: vec![0.0],
        origins_x: vec![ox],
        origins_y: vec![oy],
        scales: vec![1.0],
        reflect_x: vec![false],
        reflect_y: vec![false],
    };
    let bi = next.deformers.len();
    next.deformers.push(Deformer {
        id,
        parent_part: part,
        parent_deformer: parent.map(|p| p as u32),
        visible: true,
        enabled: true,
        binding: Binding::default(),
        kind: DeformerKind::Bone(BoneData::new(rotation, 1.0)),
        multiply: Vec::new(),
        screen: Vec::new(),
        blend_shapes: Vec::new(),
    });
    // the bone's own y axis as its parents draw it, unturned
    let probe = edit::rest_puppet(&next);
    let at = probe.deformer_point(bi, 0.0, 0.0);
    let one = probe.deformer_point(bi, 0.0, 1.0);
    let u = (one.0 - at.0, one.1 - at.1);
    let d = (tip.0 - joint.0, tip.1 - joint.1);
    let unit = u.0.hypot(u.1);
    if unit < 1e-9 || !unit.is_finite() {
        return None;
    }
    let turn = (u.0 * d.1 - u.1 * d.0).atan2(u.0 * d.0 + u.1 * d.1).to_degrees();
    let length = d.0.hypot(d.1) / unit;
    let set = |m: &mut Model, angle: f32| {
        if let Some(DeformerKind::Bone(b)) = m.deformers.get_mut(bi).map(|d| &mut d.kind) {
            b.rotation.angles = vec![angle];
            b.length = length;
        }
    };
    set(&mut next, turn);
    // a mirrored parent turns the other way
    let reach =
        |m: &Model| bone_ends(&edit::rest_puppet(m), bi).map_or(f32::MAX, |e| dist(e.1, tip));
    if reach(&next) > 1e-3 * (1.0 + d.0.hypot(d.1)) {
        let mut other = next.clone();
        set(&mut other, -turn);
        if reach(&other) < reach(&next) {
            next = other;
        }
    }
    *model = next;
    Some(bi)
}

/// A value a bone holds per keyform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoneField {
    Angle,
    OriginX,
    OriginY,
    ScaleX,
    ScaleY,
    ShearX,
    ShearY,
}

/// A bone's value in one keyform. An absent y scale reads as the x scale,
/// an absent shear as none.
pub fn bone_field(model: &Model, bone: usize, keyform: usize, field: BoneField) -> Option<f32> {
    let DeformerKind::Bone(b) = &model.deformers.get(bone)?.kind else { return None };
    let r = &b.rotation;
    let pool = |v: &[f32], fallback: Option<f32>| match v.get(keyform) {
        Some(&x) if v.len() == r.scales.len() => Some(x),
        _ => fallback,
    };
    match field {
        BoneField::Angle => r.angles.get(keyform).copied(),
        BoneField::OriginX => r.origins_x.get(keyform).copied(),
        BoneField::OriginY => r.origins_y.get(keyform).copied(),
        BoneField::ScaleX => r.scales.get(keyform).copied(),
        BoneField::ScaleY => pool(&b.scales_y, r.scales.get(keyform).copied()),
        BoneField::ShearX => pool(&b.shears_x, Some(0.0)),
        BoneField::ShearY => pool(&b.shears_y, Some(0.0)),
    }
}

/// Set a bone's value in one keyform. A y scale or shear the bone does not
/// have yet is made for every keyform first (the x scale, no shear).
pub fn set_bone_field(
    model: &mut Model,
    bone: usize,
    keyform: usize,
    field: BoneField,
    value: f32,
) -> bool {
    if !value.is_finite() {
        return false;
    }
    let Some(DeformerKind::Bone(b)) = model.deformers.get_mut(bone).map(|d| &mut d.kind) else {
        return false;
    };
    let n = b.rotation.scales.len();
    let fill = |pool: &mut Vec<f32>, from: &[f32]| {
        if pool.len() != n {
            *pool = from.to_vec();
        }
    };
    let slot = match field {
        BoneField::Angle => b.rotation.angles.get_mut(keyform),
        BoneField::OriginX => b.rotation.origins_x.get_mut(keyform),
        BoneField::OriginY => b.rotation.origins_y.get_mut(keyform),
        BoneField::ScaleX => b.rotation.scales.get_mut(keyform),
        BoneField::ScaleY => {
            fill(&mut b.scales_y, &b.rotation.scales);
            b.scales_y.get_mut(keyform)
        }
        BoneField::ShearX => {
            fill(&mut b.shears_x, &vec![0.0; n]);
            b.shears_x.get_mut(keyform)
        }
        BoneField::ShearY => {
            fill(&mut b.shears_y, &vec![0.0; n]);
            b.shears_y.get_mut(keyform)
        }
    };
    match slot {
        Some(s) if *s != value => {
            *s = value;
            true
        }
        _ => false,
    }
}

/// Set a bone's rest length (its own units).
pub fn set_bone_length(model: &mut Model, bone: usize, length: f32) -> bool {
    match model.deformers.get_mut(bone).map(|d| &mut d.kind) {
        Some(DeformerKind::Bone(b)) if length.is_finite() && length >= 0.0 => {
            b.length = length;
            true
        }
        _ => false,
    }
}

/// Set what a bone takes from its parent.
pub fn set_bone_inherit(model: &mut Model, bone: usize, inherit: Inherit) -> bool {
    match model.deformers.get_mut(bone).map(|d| &mut d.kind) {
        Some(DeformerKind::Bone(b)) if b.inherit != inherit => {
            b.inherit = inherit;
            true
        }
        _ => false,
    }
}

/// Why a mesh can't be weighted to some bones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    NoMesh,
    /// One of them is not a bone.
    NotBone,
    /// No deformer has every bone under it with only rotation deformers
    /// and bones between: they sit under different warps.
    NoCommonParent,
}

/// The deformers `bone` may be weighted from: its ancestors up to and
/// including the first one that is not a rotation deformer or bone, then
/// None (the root) when there is no such one. Nearest first.
fn weight_parents(model: &Model, bone: usize) -> Vec<Option<u32>> {
    let mut out = Vec::new();
    let mut at = model.deformers.get(bone).and_then(|d| d.parent_deformer);
    for _ in 0..=model.deformers.len() {
        out.push(at);
        let Some(p) = at else { break };
        match model.deformers.get(p as usize) {
            Some(d) if d.kind.rotation().is_some() => at = d.parent_deformer,
            _ => break,
        }
    }
    out
}

/// The parent deformer a mesh weighted to `bones` must have: every bone
/// under it (or under the root) with only rotation deformers and bones
/// between. The mesh's own parent when it will do, else the nearest that
/// does.
pub fn bind_parent(model: &Model, mesh: usize, bones: &[usize]) -> Result<Option<u32>, BindError> {
    let m = model.art_meshes.get(mesh).ok_or(BindError::NoMesh)?;
    if bones.is_empty() || bones.iter().any(|&b| !is_bone(model, b)) {
        return Err(BindError::NotBone);
    }
    let mut common: Option<Vec<Option<u32>>> = None;
    for &b in bones {
        let here = weight_parents(model, b);
        common = Some(match common {
            None => here,
            Some(c) => c.into_iter().filter(|p| here.contains(p)).collect(),
        });
    }
    let common = common.unwrap_or_default();
    if common.contains(&m.parent_deformer) {
        return Ok(m.parent_deformer);
    }
    common.first().copied().ok_or(BindError::NoCommonParent)
}

/// Put `mesh` under the parent `bones` need, keeping where it is drawn.
fn seat_mesh(model: &mut Model, mesh: usize, bones: &[usize]) -> Result<(), BindError> {
    let parent = bind_parent(model, mesh, bones)?;
    let now = model.art_meshes.get(mesh).ok_or(BindError::NoMesh)?.parent_deformer;
    if now != parent
        && edit::set_parent(model, Child::Mesh(mesh), parent.map(|p| p as usize)).is_none()
    {
        return Err(BindError::NoCommonParent);
    }
    Ok(())
}

/// The shortest distance from `p` to the segment `a`..`b`.
fn to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 1e-12 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(p, (a.0 + dx * t, a.1 + dy * t))
}

/// Weight `mesh` to `bones` by distance: each vertex leans on each bone
/// (joint to tip, as drawn at rest) by inverse distance raised to a power
/// that `smoothness` (0 to 1) lowers, so a smoother bind blends further
/// from each bone. Weights under a hundredth are dropped and the rest sum
/// to 1. The mesh moves under the parent the bones need (see
/// [`bind_parent`]) without moving on screen; its old weights are replaced.
pub fn auto_weights(
    model: &mut Model,
    mesh: usize,
    bones: &[usize],
    smoothness: f32,
) -> Result<(), BindError> {
    let mut next = model.clone();
    seat_mesh(&mut next, mesh, bones)?;
    // drawn at rest, with no weights of its own in the way
    if let Some(m) = next.art_meshes.get_mut(mesh) {
        m.skin = None;
    }
    let rest = edit::rest_puppet(&next);
    let pts = rest.mesh_drawn_positions(mesh).ok_or(BindError::NoMesh)?;
    let segs: Vec<(u32, Pt, Pt)> =
        bones.iter().filter_map(|&b| bone_ends(&rest, b).map(|(j, t)| (b as u32, j, t))).collect();
    if segs.is_empty() {
        return Err(BindError::NotBone);
    }
    // distances are taken against the longest bone, so the falloff reads
    // the same at any model size
    let reach = segs.iter().map(|s| dist(s.1, s.2)).fold(0.0f32, f32::max).max(1e-6);
    let power = 1.0 + 7.0 * (1.0 - smoothness.clamp(0.0, 1.0));
    let pairs: Vec<Vec<(u32, f32)>> = pts
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[x, y]| {
            let p = (x, y);
            let mut ws: Vec<(u32, f32)> = segs
                .iter()
                .map(|&(b, j, t)| (b, (to_segment(p, j, t) / reach + 1e-3).powf(-power)))
                .collect();
            normalise(&mut ws);
            ws.retain(|w| w.1 >= WEIGHT_FLOOR);
            normalise(&mut ws);
            ws
        })
        .collect();
    if let Some(m) = next.art_meshes.get_mut(mesh) {
        m.skin = skin_from(&pairs);
    }
    *model = next;
    Ok(())
}

fn normalise(ws: &mut [(u32, f32)]) {
    let sum: f32 = ws.iter().map(|w| w.1).sum();
    if sum > 1e-12 && sum.is_finite() {
        for w in ws {
            w.1 /= sum;
        }
    }
}

/// What a weight brush stroke does to the active bone's weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightBrush {
    Add,
    Subtract,
    /// Toward the mean of the vertex's neighbours.
    Smooth,
    /// Toward the brush's value.
    Replace,
}

/// The bones a mesh is weighted to, in the order first met.
pub fn mesh_bones(model: &Model, mesh: usize) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    if let Some(s) = model.art_meshes.get(mesh).and_then(|m| m.skin.as_ref()) {
        for &b in &s.bones {
            if !out.contains(&b) {
                out.push(b);
            }
        }
    }
    out
}

/// Each vertex's weight to `bone` (0 where it has none).
pub fn vertex_weights(model: &Model, mesh: usize, bone: u32) -> Vec<f32> {
    let Some(m) = model.art_meshes.get(mesh) else { return Vec::new() };
    pairs_of(m.skin.as_ref(), m.vertex_count as usize)
        .iter()
        .map(|v| v.iter().filter(|p| p.0 == bone).map(|p| p.1).sum())
        .collect()
}

/// One brush stroke on `mesh` for `bone`: each hit vertex's weight moves
/// by its amount (0 to 1, the brush's falloff and strength), toward
/// `value` for Replace. Then each vertex is renormalised: its other bones
/// share what the bone does not take, in proportion, so the total stays 1;
/// a vertex with only this bone keeps what was painted, the rest following
/// its parent deformer. Weights under a hundredth drop. The mesh moves
/// under the parent the bones need, as for [`auto_weights`].
pub fn paint_weights(
    model: &mut Model,
    mesh: usize,
    bone: usize,
    hits: &[(usize, f32)],
    brush: WeightBrush,
    value: f32,
) -> Result<bool, BindError> {
    let mut bones: Vec<usize> = mesh_bones(model, mesh).iter().map(|&b| b as usize).collect();
    if !bones.contains(&bone) {
        bones.push(bone);
    }
    let mut next = model.clone();
    seat_mesh(&mut next, mesh, &bones)?;
    let m = next.art_meshes.get_mut(mesh).ok_or(BindError::NoMesh)?;
    let n = m.vertex_count as usize;
    let mut pairs = pairs_of(m.skin.as_ref(), n);
    let b32 = bone as u32;
    let own = |v: &[(u32, f32)]| v.iter().filter(|p| p.0 == b32).map(|p| p.1).sum::<f32>();
    let before: Vec<f32> = pairs.iter().map(|v| own(v.as_slice())).collect();
    // neighbours along the triangles' edges, for Smooth
    let mut near: Vec<Vec<usize>> = Vec::new();
    if brush == WeightBrush::Smooth {
        near = vec![Vec::new(); n];
        for &[i, j, k] in m.indices.as_chunks::<3>().0 {
            for (a, b) in [(i, j), (j, k), (k, i)] {
                let (a, b) = (a as usize, b as usize);
                if let Some(l) = near.get_mut(a).filter(|l| !l.contains(&b)) {
                    l.push(b);
                }
                if let Some(l) = near.get_mut(b).filter(|l| !l.contains(&a)) {
                    l.push(a);
                }
            }
        }
    }
    let mut changed = false;
    for &(v, amount) in hits {
        let (Some(&w), Some(list)) = (before.get(v), pairs.get_mut(v)) else { continue };
        let a = amount.clamp(0.0, 1.0);
        let goal = match brush {
            WeightBrush::Add => w + a,
            WeightBrush::Subtract => w - a,
            WeightBrush::Replace => w + (value.clamp(0.0, 1.0) - w) * a,
            WeightBrush::Smooth => {
                let ns = near.get(v).map(Vec::as_slice).unwrap_or(&[]);
                if ns.is_empty() {
                    continue;
                }
                let mean = ns.iter().filter_map(|&k| before.get(k)).sum::<f32>() / ns.len() as f32;
                w + (mean - w) * a
            }
        };
        let mut goal = goal.clamp(0.0, 1.0);
        if goal < WEIGHT_FLOOR {
            goal = 0.0;
        }
        if (goal - w).abs() < 1e-6 {
            continue;
        }
        changed = true;
        list.retain(|p| p.0 != b32);
        let others: f32 = list.iter().map(|p| p.1).sum();
        if others > 1e-6 {
            let k = (1.0 - goal) / others;
            for p in list.iter_mut() {
                p.1 *= k;
            }
            list.retain(|p| p.1 >= WEIGHT_FLOOR);
        }
        if goal > 0.0 {
            list.push((b32, goal));
        }
        // what rounding dropped goes back in proportion
        if others > 1e-6 && !list.is_empty() {
            normalise(list);
        }
    }
    if !changed {
        return Ok(false);
    }
    m.skin = skin_from(&pairs);
    *model = next;
    Ok(true)
}

/// Take a mesh's weights off: it follows its parent deformer alone again.
pub fn remove_weights(model: &mut Model, mesh: usize) -> bool {
    model.art_meshes.get_mut(mesh).and_then(|m| m.skin.take()).is_some()
}

/// Why an IK constraint can't be added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IkError {
    /// The chain is one bone or two.
    ChainLength,
    /// Two bones that are not a bone and its child.
    NotChain,
    NotBone,
    /// The target is in the chain or under it.
    TargetInChain,
    /// A bone of the chain is already in a chain.
    Taken,
    /// The target can't be placed before the chain.
    Order,
}

/// Add an IK constraint: `chain` is one bone, or two where one is the
/// other's child (in either order), and `target` the bone it reaches for.
/// The target must evaluate first, so when it comes after the chain it and
/// everything under it move ahead of the chain. Mix starts at 1 and the
/// elbow bends counterclockwise. Returns the constraint's index.
pub fn add_ik(model: &mut Model, chain: &[usize], target: usize) -> Result<usize, IkError> {
    let (root, child) = match *chain {
        [r] => (r, None),
        [a, b] => {
            let parent = |d: usize| model.deformers.get(d).and_then(|x| x.parent_deformer);
            if parent(b) == Some(a as u32) {
                (a, Some(b))
            } else if parent(a) == Some(b as u32) {
                (b, Some(a))
            } else {
                return Err(IkError::NotChain);
            }
        }
        _ => return Err(IkError::ChainLength),
    };
    if !is_bone(model, root) || child.is_some_and(|c| !is_bone(model, c)) || !is_bone(model, target)
    {
        return Err(IkError::NotBone);
    }
    if edit::deformer_under(model, target, root) {
        return Err(IkError::TargetInChain);
    }
    let taken = |d: usize| model.ik.iter().any(|c| c.bones.contains(&(d as u32)));
    if taken(root) || child.is_some_and(taken) {
        return Err(IkError::Taken);
    }
    let mut next = model.clone();
    let (mut root, mut child, mut target) = (root, child, target);
    if target > root {
        // the target and what is under it go just before the chain
        let n = next.deformers.len();
        let moving: Vec<bool> = (0..n).map(|d| edit::deformer_under(&next, d, target)).collect();
        let is_moving = |d: usize| moving.get(d).copied().unwrap_or(false);
        let mut order: Vec<usize> = (0..root).filter(|&d| !is_moving(d)).collect();
        order.extend((0..n).filter(|&d| is_moving(d)));
        order.extend((root..n).filter(|&d| !is_moving(d)));
        let mut new_of = vec![0usize; n];
        for (new, &old) in order.iter().enumerate() {
            if let Some(slot) = new_of.get_mut(old) {
                *slot = new;
            }
        }
        edit::put_in_order(&mut next, &order, &new_of);
        let again = edit::sort_parents_first(&mut next);
        let at = |d: usize| new_of.get(d).and_then(|&k| again.get(k)).copied().unwrap_or(d);
        (root, child, target) = (at(root), child.map(at), at(target));
        if target > root {
            return Err(IkError::Order);
        }
    }
    let mut n = next.ik.len();
    let id = loop {
        let id = format!("IK{n}");
        if !next.ik.iter().any(|c| c.id == id) {
            break id;
        }
        n += 1;
    };
    let mut bones = vec![root as u32];
    bones.extend(child.map(|c| c as u32));
    next.ik.push(IkConstraint {
        id,
        bones,
        target: target as u32,
        binding: Binding::default(),
        mix: vec![1.0],
        bend_positive: vec![true],
    });
    let ci = next.ik.len() - 1;
    if !crate::ik::plans(&next).iter().any(|p| p.constraint == ci) {
        return Err(IkError::Order);
    }
    *model = next;
    Ok(ci)
}

/// Delete an IK constraint.
pub fn delete_ik(model: &mut Model, ik: usize) -> bool {
    if ik < model.ik.len() {
        model.ik.remove(ik);
        true
    } else {
        false
    }
}

/// Set an IK constraint's mix (0 to 1) in one keyform.
pub fn set_ik_mix(model: &mut Model, ik: usize, keyform: usize, mix: f32) -> bool {
    match model.ik.get_mut(ik).and_then(|c| c.mix.get_mut(keyform)) {
        Some(m) if mix.is_finite() => {
            *m = mix.clamp(0.0, 1.0);
            true
        }
        _ => false,
    }
}

/// Set which way an IK constraint's elbow bends in one keyform.
pub fn set_ik_bend(model: &mut Model, ik: usize, keyform: usize, positive: bool) -> bool {
    match model.ik.get_mut(ik).and_then(|c| c.bend_positive.get_mut(keyform)) {
        Some(b) if *b != positive => {
            *b = positive;
            true
        }
        _ => false,
    }
}

fn is_blend_shape(model: &Model, param: usize) -> bool {
    model.parameters.get(param).is_some_and(|p| p.blend_shape)
}

/// Add keys at `values` to an IK constraint's binding on `param`, as
/// `edit::add_keys` does for objects. Blend shape parameters don't key IK.
pub fn add_ik_keys(model: &mut Model, ik: usize, param: usize, values: &[f32]) -> bool {
    if is_blend_shape(model, param) || param >= model.parameters.len() {
        return false;
    }
    let Some(c) = model.ik.get_mut(ik) else { return false };
    let IkConstraint { binding, mix, bend_positive, .. } = c;
    let mut channels =
        [Channel::F32 { elems: 1, data: mix }, Channel::Bool { data: bend_positive }];
    edit::add_keys_to(binding, &mut channels, param, values)
}

/// Delete the key nearest `value` on an IK constraint's axis for `param`.
pub fn delete_ik_key(model: &mut Model, ik: usize, param: usize, value: f32) -> bool {
    let Some(c) = model.ik.get_mut(ik) else { return false };
    let IkConstraint { binding, mix, bend_positive, .. } = c;
    let mut channels =
        [Channel::F32 { elems: 1, data: mix }, Channel::Bool { data: bend_positive }];
    edit::delete_key_from(binding, &mut channels, param, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, BindingAxis, Parameter, WarpData};

    fn param(id: &str) -> Parameter {
        Parameter {
            id: id.into(),
            min: -1.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape: false,
        }
    }

    fn deformer(id: &str, parent: Option<u32>, kind: DeformerKind) -> Deformer {
        Deformer {
            id: id.into(),
            parent_part: None,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            kind,
            multiply: Vec::new(),
            screen: Vec::new(),
            blend_shapes: Vec::new(),
        }
    }

    /// A bone at `origin` in its parent's space, unturned, `length` long.
    fn bone(id: &str, parent: Option<u32>, origin: (f32, f32), length: f32) -> Deformer {
        let r = RotationData {
            base_angle: 0.0,
            opacities: vec![1.0],
            angles: vec![0.0],
            origins_x: vec![origin.0],
            origins_y: vec![origin.1],
            scales: vec![1.0],
            reflect_x: vec![false],
            reflect_y: vec![false],
        };
        deformer(id, parent, DeformerKind::Bone(BoneData::new(r, length)))
    }

    /// A 1x1 warp over [0,4]^2.
    fn warp(id: &str) -> Deformer {
        deformer(
            id,
            None,
            DeformerKind::Warp(WarpData {
                rows: 1,
                cols: 1,
                quad: true,
                opacities: vec![1.0],
                positions: vec![0.0, 0.0, 4.0, 0.0, 0.0, 4.0, 4.0, 4.0],
            }),
        )
    }

    fn mesh(parent: Option<u32>, positions: Vec<f32>) -> ArtMesh {
        let n = positions.len() / 2;
        ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: n as u32,
            uvs: vec![0.0; n * 2],
            indices: vec![0, 1, 2, 1, 2, 3],
            masks: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions,
            blend_shapes: Vec::new(),
            multiply: Vec::new(),
            screen: Vec::new(),
            skin: None,
        }
    }

    const QUAD: [f32; 8] = [0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 3.0];

    fn model(deformers: Vec<Deformer>, meshes: Vec<ArtMesh>) -> Model {
        let mut m = Model::new();
        m.parameters = vec![param("P0")];
        m.deformers = deformers;
        m.art_meshes = meshes;
        m
    }

    fn sums(m: &Model, mesh: usize) -> Vec<f32> {
        let mm = &m.art_meshes[mesh];
        pairs_of(mm.skin.as_ref(), mm.vertex_count as usize)
            .iter()
            .map(|v| v.iter().map(|p| p.1).sum())
            .collect()
    }

    fn two_bone_rig() -> Model {
        // a warp first, so indices shift when it goes; two root bones
        let mut m = model(
            vec![warp("W"), bone("A", None, (0.0, 0.0), 1.5), bone("B", Some(1), (0.0, 1.5), 1.5)],
            vec![mesh(None, QUAD.to_vec())],
        );
        m.art_meshes[0].skin = Some(Skin::from_pairs([
            &[(1u32, 1.0f32)][..],
            &[(1, 0.75), (2, 0.25)],
            &[(1, 0.25), (2, 0.75)],
            &[(2, 1.0)],
        ]));
        m.ik.push(IkConstraint {
            id: "IK0".into(),
            bones: vec![2],
            target: 1,
            binding: Binding::default(),
            mix: vec![1.0],
            bend_positive: vec![true],
        });
        m
    }

    #[test]
    fn deleting_a_deformer_before_the_bones_shifts_weights_and_ik() {
        let mut m = two_bone_rig();
        assert!(edit::delete_deformer(&mut m, 0));
        assert_eq!(mesh_bones(&m, 0), vec![0, 1]);
        assert_eq!(m.ik[0].bones, vec![1]);
        assert_eq!(m.ik[0].target, 0);
    }

    #[test]
    fn deleting_a_bone_gives_its_weight_to_the_rest_and_drops_its_ik() {
        let mut m = two_bone_rig();
        assert!(edit::delete_deformer(&mut m, 1));
        // B, now index 1 after the warp, holds what is left
        assert_eq!(mesh_bones(&m, 0), vec![1]);
        let s = sums(&m, 0);
        assert_eq!(s[0], 0.0, "a vertex left with no bone follows its parent");
        assert!(s[1..].iter().all(|x| (x - 1.0).abs() < 1e-6), "{s:?}");
        assert!(m.ik.is_empty());
    }

    #[test]
    fn a_new_parent_deformer_shifts_the_indices_past_it() {
        let mut m = two_bone_rig();
        let at = edit::create_deformer(
            &mut m,
            edit::NewDeformer::Rotation { origin: None },
            &[Child::Deformer(1)],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "R".into(),
        )
        .expect("made");
        assert_eq!(at, 1);
        assert_eq!(mesh_bones(&m, 0), vec![2, 3]);
        assert_eq!((m.ik[0].bones.clone(), m.ik[0].target), (vec![3], 2));
    }

    #[test]
    fn a_resort_renumbers_weights_and_ik() {
        // B moves under the warp: re-sorted, the weights follow it
        let mut m = two_bone_rig();
        m.deformers[0].parent_deformer = None;
        let order = vec![2usize, 0, 1];
        let mut new_of = vec![0usize; 3];
        for (new, &old) in order.iter().enumerate() {
            new_of[old] = new;
        }
        m.deformers[2].parent_deformer = None;
        edit::put_in_order(&mut m, &order, &new_of);
        assert_eq!(m.deformers[0].id, "B");
        assert_eq!(mesh_bones(&m, 0), vec![2, 0]);
        assert_eq!((m.ik[0].bones.clone(), m.ik[0].target), (vec![0], 2));
    }

    #[test]
    fn a_pasted_mesh_keeps_only_weights_on_bones() {
        let mut m = two_bone_rig();
        let mut copy = m.art_meshes[0].clone();
        copy.skin = Some(Skin::from_pairs([
            &[(0u32, 0.5f32), (1, 0.5)][..],
            &[(1, 1.0)],
            &[(0, 1.0)],
            &[(7, 1.0)],
        ]));
        let i = edit::insert_art_mesh(&mut m, copy, None, None);
        assert_eq!(mesh_bones(&m, i), vec![1]);
        assert_eq!(sums(&m, i), vec![1.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_new_bone_runs_from_joint_to_tip() {
        let mut m = model(vec![], vec![]);
        let a = create_bone(&mut m, None, None, (1.0, 2.0), (4.0, 6.0), "A".into()).unwrap();
        let ends = bone_ends(&edit::rest_puppet(&m), a).unwrap();
        assert!(dist(ends.0, (1.0, 2.0)) < 1e-4 && dist(ends.1, (4.0, 6.0)) < 1e-4, "{ends:?}");
        // chained: a child from the tip, under a turned and scaled parent
        if let DeformerKind::Bone(b) = &mut m.deformers[a].kind {
            b.rotation.scales = vec![2.0];
        }
        let b = create_bone(&mut m, Some(a), None, (4.0, 6.0), (4.0, 9.0), "B".into()).unwrap();
        let ends = bone_ends(&edit::rest_puppet(&m), b).unwrap();
        assert!(dist(ends.0, (4.0, 6.0)) < 1e-3 && dist(ends.1, (4.0, 9.0)) < 1e-3, "{ends:?}");
        assert_eq!(m.deformers[b].parent_deformer, Some(a as u32));
    }

    #[test]
    fn auto_weights_lean_on_the_nearest_bone_and_sum_to_one() {
        let mut m = model(
            vec![bone("A", None, (0.0, 0.0), 1.5), bone("B", Some(0), (0.0, 1.5), 1.5)],
            vec![mesh(None, vec![1.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0, 3.0])],
        );
        auto_weights(&mut m, 0, &[0, 1], 0.2).unwrap();
        let a = vertex_weights(&m, 0, 0);
        let b = vertex_weights(&m, 0, 1);
        assert!(a[0] > 0.9 && b[3] > 0.9, "{a:?} {b:?}");
        assert!(sums(&m, 0).iter().all(|s| (s - 1.0).abs() < 1e-5));
        // smoother reaches further
        let mut soft = m.clone();
        auto_weights(&mut soft, 0, &[0, 1], 1.0).unwrap();
        assert!(vertex_weights(&soft, 0, 1)[0] > b[0]);
    }

    #[test]
    fn binding_moves_the_mesh_to_where_the_bones_are_and_keeps_its_place() {
        let mut m = model(
            vec![warp("W"), bone("A", None, (0.0, 0.0), 2.0)],
            vec![mesh(Some(0), vec![0.1, 0.1, 0.2, 0.1, 0.1, 0.2, 0.2, 0.2])],
        );
        let before = edit::rest_puppet(&m).mesh_drawn_positions(0).unwrap();
        auto_weights(&mut m, 0, &[1], 0.5).unwrap();
        assert_eq!(m.art_meshes[0].parent_deformer, None);
        let after = edit::rest_puppet(&m).mesh_drawn_positions(0).unwrap();
        assert!(before.iter().zip(&after).all(|(a, b)| (a - b).abs() < 1e-4));
        // bones under different warps have nothing in common
        let mut m = model(
            vec![warp("W"), warp("V"), bone("A", Some(0), (0.5, 0.5), 1.0)],
            vec![mesh(None, QUAD.to_vec())],
        );
        m.deformers.push(bone("B", Some(1), (0.5, 0.5), 1.0));
        assert_eq!(auto_weights(&mut m, 0, &[2, 3], 0.5), Err(BindError::NoCommonParent));
        assert_eq!(auto_weights(&mut m, 0, &[0], 0.5), Err(BindError::NotBone));
    }

    #[test]
    fn painting_keeps_each_vertex_whole() {
        let mut m = two_bone_rig();
        m.ik.clear();
        assert!(paint_weights(&mut m, 0, 1, &[(1, 0.2), (3, 0.5)], WeightBrush::Add, 1.0).unwrap());
        let a = vertex_weights(&m, 0, 1);
        assert!((a[1] - 0.95).abs() < 1e-5 && (a[3] - 0.5).abs() < 1e-5, "{a:?}");
        assert!(sums(&m, 0).iter().all(|s| (s - 1.0).abs() < 1e-5), "{:?}", sums(&m, 0));
        // a lone bone painted down leaves the rest to the parent
        assert!(paint_weights(&mut m, 0, 1, &[(0, 0.4)], WeightBrush::Subtract, 1.0).unwrap());
        assert!((sums(&m, 0)[0] - 0.6).abs() < 1e-5);
        assert!(paint_weights(&mut m, 0, 2, &[(0, 1.0)], WeightBrush::Replace, 0.0).is_ok());
        assert!(paint_weights(&mut m, 0, 1, &[(1, 1.0)], WeightBrush::Smooth, 1.0).unwrap());
        assert!(remove_weights(&mut m, 0));
        assert!(m.art_meshes[0].skin.is_none());
    }

    #[test]
    fn ik_takes_a_chain_and_moves_a_late_target_ahead() {
        let mut m = model(
            vec![bone("A", None, (0.0, 0.0), 1.0), bone("B", Some(0), (0.0, 1.0), 1.0)],
            vec![mesh(None, QUAD.to_vec())],
        );
        m.art_meshes[0].skin = Some(Skin::from_pairs([&[(1u32, 1.0f32)][..], &[], &[], &[]]));
        m.deformers.push(bone("T", None, (1.0, 1.0), 0.5));
        assert_eq!(add_ik(&mut m, &[0], 0), Err(IkError::TargetInChain));
        assert_eq!(add_ik(&mut m, &[0, 2], 1), Err(IkError::NotChain));
        let ci = add_ik(&mut m, &[1, 0], 2).unwrap();
        let id = |m: &Model, i: u32| m.deformers[i as usize].id.clone();
        assert_eq!(id(&m, m.ik[ci].target), "T");
        assert_eq!(m.ik[ci].bones.iter().map(|&b| id(&m, b)).collect::<Vec<_>>(), ["A", "B"]);
        assert!(m.ik[ci].target < m.ik[ci].bones[0]);
        assert_eq!(id(&m, mesh_bones(&m, 0)[0]), "B", "weights follow the move");
        assert_eq!(add_ik(&mut m, &[2], 0), Err(IkError::Taken));
        // keyed mix and bend
        m.parameters.push(param("P1"));
        assert!(add_ik_keys(&mut m, ci, 1, &[-1.0, 1.0]));
        assert_eq!(m.ik[ci].mix, vec![1.0, 1.0]);
        assert!(set_ik_mix(&mut m, ci, 0, 0.25));
        assert!(set_ik_bend(&mut m, ci, 1, false));
        assert_eq!(m.ik[ci].bend_positive, vec![true, false]);
        assert!(delete_ik_key(&mut m, ci, 1, 1.0));
        assert_eq!(m.ik[ci].mix, vec![0.25]);
        assert!(delete_ik(&mut m, ci));
        assert!(m.ik.is_empty());
    }

    #[test]
    fn bone_fields_fill_their_pools_on_first_set() {
        let mut m = model(vec![bone("A", None, (0.0, 0.0), 1.0)], vec![]);
        m.deformers[0].binding.axes.push(BindingAxis { parameter: 0, keys: vec![0.0] });
        assert_eq!(bone_field(&m, 0, 0, BoneField::ScaleY), Some(1.0));
        assert!(set_bone_field(&mut m, 0, 0, BoneField::ShearX, 10.0));
        assert!(set_bone_field(&mut m, 0, 0, BoneField::ScaleY, 2.0));
        assert_eq!(bone_field(&m, 0, 0, BoneField::ShearX), Some(10.0));
        assert_eq!(bone_field(&m, 0, 0, BoneField::ShearY), Some(0.0));
        assert_eq!(bone_field(&m, 0, 0, BoneField::ScaleY), Some(2.0));
        assert!(set_bone_length(&mut m, 0, 3.0));
        assert!(set_bone_inherit(&mut m, 0, Inherit::NoScale));
        assert!(!set_bone_inherit(&mut m, 0, Inherit::NoScale));
    }
}
