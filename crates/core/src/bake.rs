//! Bone bake: a bone-free copy of a model, for moc3 and cmo3 export, which
//! have no bones. Everything a bone does is carried by native objects:
//!
//! - a bone that turns, moves and scales evenly (and takes its parent's
//!   whole frame) is a rotation deformer already: it keeps its slot, its
//!   keyforms and its children, and evaluates the same bit for bit;
//! - a bone that shears, scales its axes unevenly or takes only part of
//!   its parent's frame becomes a warp in the same slot, its grid at every
//!   key the bone's affine map in its parent's space (exact there), with
//!   extended interpolation between the keys;
//! - a weighted mesh goes under the deepest deformer all its bones share
//!   and gets mesh keyforms at every key of the bones below that one,
//!   crossed with its own keys (exact there);
//! - an IK chain's bones get their solved turns as keyforms over their own
//!   keys and the keys that move the target and the constraint.
//!
//! Every deformer keeps its index, so nothing that points at one moves.

use ayatsuri_format::model::{
    AxisCurve, Binding, BlendShape, CurveKind, DeformerKind, Inherit, Model, WarpData, WeightLimit,
};

use crate::deform::Affine;
use crate::edit::{self, Target};
use crate::{Puppet, bones, ik};

/// The most keyforms one baked object may have. A mesh weighted across
/// many bones, each on its own parameters, multiplies their keys; past
/// this the bake stops instead of building a file nothing can load.
pub const MAX_KEYFORMS: usize = 4096;

/// Extended interpolation on the axes a bake samples: rotation-like motion
/// sampled at keys follows the curve through them, not the chord.
/// How finely a bake samples: how many times each sampled span may be
/// halved, and the keyforms export may write for the object doing so.
/// Warps and meshes carry a grid or a mesh per keyform, so they are split
/// once and follow the curve between keys; an IK chain's rotations carry a
/// few numbers each and follow a solve that bends hard, so they are split
/// further and blend straight between keys exactly sampled instead.
const SAMPLING: Sampling = Sampling { passes: 1, budget: 1024, curve: true };
const IK_SAMPLING: Sampling = Sampling { passes: 4, budget: MAX_KEYFORMS, curve: false };

/// See [`SAMPLING`].
#[derive(Clone, Copy)]
struct Sampling {
    passes: usize,
    budget: usize,
    /// Extended interpolation on the sampled axes; without it they blend
    /// straight between keys sampled closely enough not to need it.
    curve: bool,
}

const CURVE: CurveKind = CurveKind::Ellipse { points: 4, scale: 1.0 };

/// Cells per side of a bone baked to a warp. The bone's map is affine at
/// every key and a warp of an affine grid is that map exactly, inside the
/// grid and out, so the grid only needs enough points to bend smoothly
/// between keys.
const GRID: u32 = 2;

/// Spans each span of a bone's blend shape is split into when it moves a
/// weighted mesh: the mesh's blend shape goes straight between its keys.
const SHAPE_SPLIT: usize = 4;

/// What a bake did, for the export report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BakeReport {
    /// Bones that became rotation deformers.
    pub rotations: usize,
    /// Bones that became warps.
    pub warps: usize,
    /// Weighted meshes given keyforms in place of their weights.
    pub meshes: usize,
    /// IK constraints baked into their chains' keyforms.
    pub ik_chains: usize,
    /// The largest distance (model units) between a vertex of the baked
    /// model and the same vertex of the original, over a sweep of poses at
    /// the keys the bake sampled.
    pub at_keys: f32,
    /// The same between those keys.
    pub between_keys: f32,
    /// Anything the bake could only approximate.
    pub notes: Vec<String>,
}

impl BakeReport {
    /// One line for the export dialog; `ppu` turns model units to pixels.
    pub fn line(&self, ppu: f32) -> String {
        let px = |d: f32| d * if ppu.is_finite() && ppu > 0.0 { ppu } else { 1.0 };
        format!(
            "bones baked: {} to rotation deformers, {} to warps, {} weighted mesh(es), {} IK \
             chain(s); worst error {:.3} px at keys, {:.3} px between keys",
            self.rotations,
            self.warps,
            self.meshes,
            self.ik_chains,
            px(self.at_keys),
            px(self.between_keys)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BakeError {
    /// An object would need more than [`MAX_KEYFORMS`] keyforms.
    TooManyKeyforms { object: String, count: usize },
}

impl core::fmt::Display for BakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BakeError::TooManyKeyforms { object, count } => write!(
                f,
                "baking the bones would give {object} {count} keyforms (at most {MAX_KEYFORMS}); \
                 weight it to fewer bones or key those bones on fewer parameters"
            ),
        }
    }
}

impl std::error::Error for BakeError {}

/// Whether the model has anything for [`bake_bones`] to do.
pub fn has_bones(model: &Model) -> bool {
    model.deformers.iter().any(|d| d.kind.is_bone())
        || !model.ik.is_empty()
        || model.art_meshes.iter().any(|m| m.skin.as_ref().is_some_and(|s| !s.weights.is_empty()))
}

/// Keys to sample on: per parameter, its keys, and whether it is borrowed
/// (it moves the object through something else, so it gets the curve, and
/// spans the parameter's whole range when the object had no axis on it).
type Axes = Vec<(u32, Vec<f32>, bool)>;

fn own_axes(binding: &Binding) -> Axes {
    binding.axes.iter().map(|a| (a.parameter, a.keys.clone(), false)).collect()
}

fn merge(into: &mut Axes, from: &Axes, borrowed: bool) {
    for (p, keys, b) in from {
        let borrowed = borrowed || *b;
        match into.iter_mut().find(|a| a.0 == *p) {
            Some(a) => {
                for &k in keys {
                    if !a.1.iter().any(|x| (x - k).abs() < 1e-5) {
                        a.1.push(k);
                    }
                }
                a.1.sort_by(f32::total_cmp);
                a.2 = a.2 || borrowed;
            }
            None => into.push((*p, keys.clone(), borrowed)),
        }
    }
}

/// The deformers above `d`, nearest first.
fn ancestors(model: &Model, d: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = model.deformers.get(d).and_then(|x| x.parent_deformer);
    // bounded, so a parent loop in a broken file cannot spin
    while let Some(p) = at {
        if out.len() > model.deformers.len() {
            break;
        }
        out.push(p as usize);
        at = model.deformers.get(p as usize).and_then(|x| x.parent_deformer);
    }
    out
}

fn axes_of(model: &Model, d: usize) -> Axes {
    model.deformers.get(d).map(|x| own_axes(&x.binding)).unwrap_or_default()
}

/// The keys that move deformer `d` in its parent's space: its own, and for
/// a bone in an IK chain the keys of the constraint, the chain, what is
/// above it and the target with what is above that.
fn driving(model: &Model, plans: &[ik::Plan], d: usize) -> Axes {
    let mut s = axes_of(model, d);
    if let Some(plan) = plans.iter().find(|p| p.root == d || p.child == Some(d)) {
        if let Some(c) = model.ik.get(plan.constraint) {
            merge(&mut s, &own_axes(&c.binding), true);
        }
        let mut from = vec![plan.root, plan.target];
        from.extend(plan.child);
        from.extend(ancestors(model, plan.root));
        from.extend(ancestors(model, plan.target));
        for x in from {
            merge(&mut s, &axes_of(model, x), true);
        }
    }
    s
}

/// A warp's unit square laid over a box of a bone's local space.
#[derive(Debug, Clone, Copy)]
struct Domain {
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
}

impl Domain {
    fn to_unit(self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x0) / self.w, (y - self.y0) / self.h)
    }

    fn out_of_unit(self, u: f32, v: f32) -> (f32, f32) {
        (self.x0 + u * self.w, self.y0 + v * self.h)
    }

    fn map_pairs(self, pts: &mut [f32]) {
        let (pairs, _) = pts.as_chunks_mut::<2>();
        for p in pairs {
            let (u, v) = self.to_unit(p[0], p[1]);
            *p = [u, v];
        }
    }

    /// The box around `pts` (xy pairs), padded; a flat or empty one is
    /// `fallback` across, centred where it lies.
    fn around(pts: &[f32], fallback: f32) -> Domain {
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        let (pairs, _) = pts.as_chunks::<2>();
        for p in pairs.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
            lo = (lo.0.min(p[0]), lo.1.min(p[1]));
            hi = (hi.0.max(p[0]), hi.1.max(p[1]));
        }
        if lo.0 > hi.0 {
            (lo, hi) = ((0.0, 0.0), (0.0, 0.0));
        }
        let side = |a: f32, b: f32| {
            let w = b - a;
            if w > fallback * 1e-3 {
                (a - w * 0.1, w * 1.2)
            } else {
                (a - fallback * 0.5, fallback)
            }
        };
        let ((x0, w), (y0, h)) = (side(lo.0, hi.0), side(lo.1, hi.1));
        Domain { x0, y0, w, h }
    }
}

/// Every parameter at its default, then `pose`.
fn set_pose(p: &mut Puppet, defaults: &[f32], pose: &[(u32, f32)]) {
    for (i, &d) in defaults.iter().enumerate() {
        p.set_value(i, d);
    }
    for &(q, v) in pose {
        p.set_value(q as usize, v);
    }
    p.update();
}

/// The parameter values of keyform `k` of `binding` (first axis fastest).
fn key_pose(binding: &Binding, k: usize) -> Vec<(u32, f32)> {
    let mut idx = k;
    binding
        .axes
        .iter()
        .map(|a| {
            let n = a.keys.len().max(1);
            let c = idx % n;
            idx /= n;
            (a.parameter, a.keys.get(c).copied().unwrap_or(0.0))
        })
        .collect()
}

/// A bone's or rotation deformer's map from its local space to the canvas
/// in `p` now.
fn rotation_map(p: &Puppet, d: usize) -> Option<Affine> {
    let local = *p.def_local.get(d)? as usize;
    match &p.model.deformers.get(d)?.kind {
        DeformerKind::Bone(_) => p.bone_world.get(local).map(|r| r.map),
        DeformerKind::Rotation(_) => p.rot.get(local).map(Affine::of),
        DeformerKind::Warp(_) => None,
    }
}

/// A canvas point into the space children of `q` give their points in,
/// once baked: a bone baked to a warp reads its unit square over `domains`.
fn into_parent(
    p: &Puppet,
    domains: &[Option<Domain>],
    q: Option<usize>,
    world: (f32, f32),
) -> (f32, f32) {
    let Some(q) = q else { return world };
    let Some(m) = rotation_map(p, q) else { return edit::to_local(p, Some(q), world) };
    let (x, y) = m.inverse().map_or(world, |inv| inv.apply(world.0, world.1));
    match domains.get(q).copied().flatten() {
        Some(dom) => dom.to_unit(x, y),
        None => (x, y),
    }
}

/// A mesh's points on the canvas now, before glue (the baked model glues
/// them again).
fn mesh_world(p: &Puppet, mi: usize) -> Option<Vec<f32>> {
    let mut io = p.mesh_local_positions(mi)?;
    p.apply_skin(mi, &mut io);
    if let Some(pd) = p.model.art_meshes.get(mi)?.parent_deformer {
        p.apply_deformer(pd as usize, &mut io);
    }
    Some(io)
}

/// Bring `target`'s binding to `want` (its own keys and those it borrows),
/// with a middle key on every two-key axis so the curve has three to bend
/// through, borrowed axes over their parameter's whole range and the curve
/// on the borrowed axes (and on its own too when `curve_own`) when `how`
/// asks for it, their spans
/// halved up to `passes` times while export writes at most `budget`
/// keyforms for it. Its keyform data takes the interpolated values at the
/// new keys. True when some axes had to go without the curve to fit
/// [`MAX_KEYFORMS`].
fn widen(
    model: &Model,
    out: &mut Model,
    target: Target,
    name: &str,
    want: &Axes,
    curve_own: bool,
    how: Sampling,
) -> Result<bool, BakeError> {
    let binding = match target {
        Target::Deformer(i) => out.deformers.get(i).map(|d| &d.binding),
        Target::ArtMesh(i) => out.art_meshes.get(i).map(|m| &m.binding),
        Target::Part(i) => out.parts.get(i).map(|p| &p.binding),
    };
    let own: Vec<u32> =
        binding.map(|b| b.axes.iter().map(|a| a.parameter).collect()).unwrap_or_default();
    let mut all = binding.map(own_axes).unwrap_or_default();
    merge(&mut all, want, false);
    let mut count = 1usize;
    for (p, keys, borrowed) in &mut all {
        let Some(param) = model.parameters.get(*p as usize) else { continue };
        if param.blend_shape {
            keys.clear();
            continue;
        }
        // a borrowed axis must not switch the object off where its own
        // keys never did
        if *borrowed && !own.contains(p) {
            for end in [param.min, param.max] {
                if !keys.iter().any(|k| (k - end).abs() < 1e-5) {
                    keys.push(end);
                }
            }
            keys.sort_by(f32::total_cmp);
        }
        *borrowed |= curve_own;
        if *borrowed && let &[a, b] = keys.as_slice() {
            keys.insert(1, (a + b) * 0.5);
        }
        count = count.saturating_mul(keys.len().max(1));
    }
    if count > MAX_KEYFORMS {
        return Err(BakeError::TooManyKeyforms { object: name.to_string(), count });
    }
    // in parameter order, so objects that borrow the same axes (the two
    // bones of an IK chain) split them alike
    all.sort_by_key(|a| a.0);
    // the keyforms export writes: a curve carries `points` more keys in
    // every span of its axis
    let written = |all: &Axes| {
        all.iter().fold(1usize, |acc, (_, keys, curved)| {
            let len = keys.len().max(1);
            let len = if how.curve && *curved && len >= 3 {
                (len - 1).saturating_mul(usize::from(CURVE.points()) + 1) + 1
            } else {
                len
            };
            acc.saturating_mul(len)
        })
    };
    // too many to carry the curves at all: the longest axes go straight
    let mut straight = false;
    while written(&all) > MAX_KEYFORMS {
        let longest = all.iter_mut().filter(|a| a.2).max_by_key(|a| a.1.len());
        let Some(a) = longest else { break };
        a.2 = false;
        straight = true;
    }
    // then halve every curved span while the keyforms stay in budget, up
    // to `passes` times: the curve's miss shrinks with the cube of the span
    for _ in 0..how.passes {
        for at in 0..all.len() {
            let Some((_, keys, true)) = all.get(at) else { continue };
            if keys.len() < 2 {
                continue;
            }
            let mut split = Vec::with_capacity(2 * keys.len() - 1);
            for w in keys.windows(2) {
                if let &[a, b] = w {
                    split.extend_from_slice(&[a, (a + b) * 0.5]);
                }
            }
            split.extend(keys.last().copied());
            let mut tried = all.clone();
            if let Some(slot) = tried.get_mut(at) {
                slot.1 = split;
            }
            if written(&tried) <= how.budget {
                all = tried;
            }
        }
    }
    for (p, keys, borrowed) in &all {
        if keys.is_empty() {
            continue;
        }
        edit::add_keys(out, target, *p as usize, keys);
        if *borrowed && how.curve {
            let binding = match target {
                Target::Deformer(i) => out.deformers.get_mut(i).map(|d| &mut d.binding),
                Target::ArtMesh(i) => out.art_meshes.get_mut(i).map(|m| &mut m.binding),
                Target::Part(i) => out.parts.get_mut(i).map(|p| &mut p.binding),
            };
            if let Some(b) = binding
                && b.curve(*p).is_none()
                && b.axes.iter().any(|a| a.parameter == *p && a.keys.len() >= 3)
            {
                b.curves.push(AxisCurve { parameter: *p, kind: CURVE });
            }
        }
    }
    Ok(straight)
}

/// A bone a rotation deformer can stand in for: no shear and its y axis
/// scaled as its x axis, in every keyform.
fn plain_keyforms(b: &ayatsuri_format::model::BoneData) -> bool {
    let n = b.rotation.scales.len();
    // a pool out of step with the keyforms reads as absent (`local_bone`)
    let keyed = |pool: &[f32]| n > 0 && pool.len() == n;
    let no_shear = |pool: &[f32]| !keyed(pool) || pool.iter().all(|&s| s == 0.0);
    no_shear(&b.shears_x)
        && no_shear(&b.shears_y)
        && (!keyed(&b.scales_y) || b.scales_y.iter().zip(&b.rotation.scales).all(|(y, x)| y == x))
}

fn reflects(d: &ayatsuri_format::model::Deformer) -> bool {
    d.kind.rotation().is_some_and(|r| r.reflect_x.iter().chain(&r.reflect_y).any(|&f| f))
}

/// A bone-free copy of `model`: bones become rotation deformers or warps,
/// weights and IK become keyforms (see the module notes). The model itself
/// is not touched. Exact at every key the bake samples; between them the
/// report says how close it comes.
pub fn bake_bones(model: &Model) -> Result<(Model, BakeReport), BakeError> {
    let mut report = BakeReport::default();
    if !has_bones(model) {
        return Ok((model.clone(), report));
    }
    let defaults: Vec<f32> = model.parameters.iter().map(|p| p.default).collect();
    let mut orig = Puppet::new(model.clone());
    set_pose(&mut orig, &defaults, &[]);
    let plans = ik::plans(model);
    report.ik_chains = plans.len();
    let n = model.deformers.len();

    // which bones go to warps, parents first: one a rotation deformer
    // cannot express, and one under such a bone or under a bone that
    // reflects (a bone under a bone takes its whole map, which a rotation
    // deformer under a rotation does not)
    let mut warp = vec![false; n];
    for (i, d) in model.deformers.iter().enumerate() {
        let DeformerKind::Bone(b) = &d.kind else { continue };
        let parent =
            d.parent_deformer.and_then(|p| model.deformers.get(p as usize).map(|x| (p, x)));
        let under_bone = parent.is_some_and(|(p, x)| {
            x.kind.is_bone() && (warp.get(p as usize).copied().unwrap_or(false) || reflects(x))
        });
        let partial = parent.is_some() && b.inherit != Inherit::Normal;
        if let Some(w) = warp.get_mut(i) {
            *w = !plain_keyforms(b) || partial || under_bone;
        }
    }

    // each warp's unit square over what it holds, in the bone's space
    let mut domains: Vec<Option<Domain>> = vec![None; n];
    for (i, d) in model.deformers.iter().enumerate() {
        let (true, DeformerKind::Bone(b)) = (warp.get(i).copied().unwrap_or(false), &d.kind) else {
            continue;
        };
        let mut pts = vec![0.0, 0.0, 0.0, b.length];
        let held = Some(i as u32);
        for m in model.art_meshes.iter().filter(|m| m.parent_deformer == held) {
            pts.extend_from_slice(&m.positions);
        }
        for c in model.deformers.iter().filter(|c| c.parent_deformer == held) {
            match &c.kind {
                DeformerKind::Warp(w) => pts.extend_from_slice(&w.positions),
                DeformerKind::Rotation(r)
                | DeformerKind::Bone(ayatsuri_format::model::BoneData { rotation: r, .. }) => {
                    pts.extend(r.origins_x.iter().zip(&r.origins_y).flat_map(|(&x, &y)| [x, y]));
                }
            }
        }
        let fallback = if b.length.abs() > 1e-6 { b.length.abs() } else { 1.0 };
        if let Some(slot) = domains.get_mut(i) {
            *slot = Some(Domain::around(&pts, fallback));
        }
    }

    let mut out = model.clone();
    // the rig's own keys on what the bake touches, for the error sweep
    // (a key one object samples can sit between another's keys)
    let mut touched: Axes = Vec::new();
    for c in &model.ik {
        merge(&mut touched, &own_axes(&c.binding), false);
    }
    // and the blend shapes on bones, with their base key at 0
    for d in model.deformers.iter().filter(|d| d.kind.is_bone()) {
        for s in &d.blend_shapes {
            let mut keys = s.keys.clone();
            keys.push(0.0);
            merge(&mut touched, &vec![(s.parameter, keys, false)], false);
        }
    }

    for (i, d) in model.deformers.iter().enumerate() {
        let DeformerKind::Bone(b) = &d.kind else { continue };
        merge(&mut touched, &own_axes(&d.binding), false);
        let target = Target::Deformer(i);
        let in_ik = plans.iter().any(|p| p.root == i || p.child == Some(i));
        if !warp.get(i).copied().unwrap_or(false) {
            report.rotations += 1;
            if in_ik {
                let want = driving(model, &plans, i);
                if widen(model, &mut out, target, &d.id, &want, false, IK_SAMPLING)? {
                    report
                        .notes
                        .push(format!("{}: too many keys for the curve on some axes", d.id));
                }
                // the solved turn at every key
                let (binding, count) = match out.deformers.get(i) {
                    Some(od) => (od.binding.clone(), od.binding.keyform_count()),
                    None => continue,
                };
                let mut angles = Vec::with_capacity(count);
                for k in 0..count {
                    set_pose(&mut orig, &defaults, &key_pose(&binding, k));
                    angles.push(orig.bone_state(i).map(|s| s.angle - b.rotation.base_angle));
                }
                if let Some(DeformerKind::Bone(ob)) =
                    out.deformers.get_mut(i).map(|od| &mut od.kind)
                {
                    for (slot, a) in ob.rotation.angles.iter_mut().zip(angles) {
                        if let Some(a) = a {
                            *slot = a;
                        }
                    }
                }
            }
            if let Some(od) = out.deformers.get_mut(i)
                && let DeformerKind::Bone(ob) = &od.kind
            {
                od.kind = DeformerKind::Rotation(ob.rotation.clone());
            }
            continue;
        }

        // a warp: its grid at every key, the bone's map in its parent's space
        report.warps += 1;
        let mut want = driving(model, &plans, i);
        if b.inherit != Inherit::Normal {
            // what it takes of its parent's frame moves with everything above
            for a in ancestors(model, i) {
                merge(&mut want, &axes_of(model, a), true);
            }
        }
        if widen(model, &mut out, target, &d.id, &want, true, SAMPLING)? {
            report.notes.push(format!("{}: too many keys for the curve on some axes", d.id));
        }
        let Some(dom) = domains.get(i).copied().flatten() else { continue };
        let (binding, count, opacities) = match out.deformers.get(i) {
            Some(od) => (
                od.binding.clone(),
                od.binding.keyform_count(),
                od.kind.rotation().map(|r| r.opacities.clone()).unwrap_or_default(),
            ),
            None => continue,
        };
        let parent = d.parent_deformer.map(|p| p as usize);
        let side = (GRID + 1) as usize;
        let mut positions = Vec::with_capacity(count * side * side * 2);
        for k in 0..count {
            set_pose(&mut orig, &defaults, &key_pose(&binding, k));
            let m = rotation_map(&orig, i).unwrap_or_default();
            for r in 0..side {
                for c in 0..side {
                    let (x, y) = dom.out_of_unit(c as f32 / GRID as f32, r as f32 / GRID as f32);
                    let (gx, gy) = into_parent(&orig, &domains, parent, m.apply(x, y));
                    positions.extend_from_slice(&[gx, gy]);
                }
            }
        }
        if let Some(od) = out.deformers.get_mut(i) {
            od.kind = DeformerKind::Warp(WarpData {
                rows: GRID,
                cols: GRID,
                quad: true,
                opacities,
                positions,
            });
            // a turn, an origin or a scale has nothing to land on in a warp
            let mut dropped = false;
            for s in &mut od.blend_shapes {
                dropped |= !(s.angles.is_empty() && s.origins_x.is_empty() && s.scales.is_empty());
                s.angles.clear();
                s.origins_x.clear();
                s.origins_y.clear();
                s.scales.clear();
            }
            if dropped {
                report.notes.push(format!(
                    "{}: its blend shapes' turn, origin and scale are not baked",
                    d.id
                ));
            }
        }
        if parent.is_some_and(|p| {
            model.deformers.get(p).is_some_and(|x| matches!(x.kind, DeformerKind::Warp(_)))
        }) {
            report.notes.push(format!(
                "{}: a bone under a warp, baked to a warp, bends with it only at its grid points",
                d.id
            ));
        }
    }

    // what a bone baked to a warp held now gives its points in the warp's
    // unit square
    let skinned: Vec<bool> = orig.skinning.slot_joints.iter().map(|s| !s.is_empty()).collect();
    let baked_mesh = |mi: usize| skinned.get(mi).copied().unwrap_or(false);
    for (mi, m) in out.art_meshes.iter_mut().enumerate() {
        let Some(dom) = m.parent_deformer.and_then(|p| domains.get(p as usize).copied().flatten())
        else {
            continue;
        };
        if baked_mesh(mi) {
            continue;
        }
        dom.map_pairs(&mut m.positions);
        for s in &mut m.blend_shapes {
            dom.map_pairs(&mut s.positions);
        }
    }
    for (i, c) in out.deformers.iter_mut().enumerate() {
        // a bone baked to a warp has its grid in its parent's space already
        if warp.get(i).copied().unwrap_or(false) {
            continue;
        }
        let Some(dom) = c.parent_deformer.and_then(|p| domains.get(p as usize).copied().flatten())
        else {
            continue;
        };
        match &mut c.kind {
            DeformerKind::Warp(w) => {
                dom.map_pairs(&mut w.positions);
                for s in &mut c.blend_shapes {
                    dom.map_pairs(&mut s.positions);
                }
            }
            DeformerKind::Rotation(r) => {
                for (x, y) in r.origins_x.iter_mut().zip(r.origins_y.iter_mut()) {
                    (*x, *y) = dom.to_unit(*x, *y);
                }
                for s in &mut c.blend_shapes {
                    for (x, y) in s.origins_x.iter_mut().zip(s.origins_y.iter_mut()) {
                        (*x, *y) = dom.to_unit(*x, *y);
                    }
                }
            }
            // a bone under a bone baked to a warp is a warp too
            DeformerKind::Bone(_) => {}
        }
    }

    // a rotation deformer multiplies its scale by its parent's, and a warp
    // passes on the scale above it: one that had a bone baked to a warp
    // above it (through warps alone) gets the bone's scale back
    set_pose(&mut orig, &defaults, &[]);
    let scale_now = |out: &Model, d: Option<usize>| {
        let mut at = d;
        for _ in 0..=n {
            let Some(x) = at else { return 1.0 };
            match out.deformers.get(x).map(|o| &o.kind) {
                Some(DeformerKind::Warp(_)) => {
                    at = out.deformers.get(x).and_then(|o| o.parent_deformer).map(|p| p as usize);
                }
                Some(_) => return orig.def_scale.get(x).copied().unwrap_or(1.0),
                None => return 1.0,
            }
        }
        1.0
    };
    let ratios: Vec<f32> = (0..n)
        .map(|i| {
            let Some(o) = out.deformers.get(i) else { return 1.0 };
            if o.kind.rotation().is_none() {
                return 1.0;
            }
            let parent = o.parent_deformer.map(|p| p as usize);
            let was = parent.map_or(1.0, |p| orig.def_scale.get(p).copied().unwrap_or(1.0));
            let now = scale_now(&out, parent);
            if now.abs() > 1e-9 && was.is_finite() { was / now } else { 1.0 }
        })
        .collect();
    for (o, &ratio) in out.deformers.iter_mut().zip(&ratios) {
        if (ratio - 1.0).abs() > 1e-6
            && let Some(r) = o.kind.rotation_mut()
        {
            for s in &mut r.scales {
                *s *= ratio;
            }
        }
    }

    // weighted meshes: keyforms at every key of the bones below the
    // deepest deformer their bones share, in that deformer's space
    for (mi, m) in model.art_meshes.iter().enumerate() {
        if !baked_mesh(mi) {
            continue;
        }
        let Some(skin) = &m.skin else { continue };
        let used = orig.skinning.mesh_bones.get(mi).cloned().unwrap_or_default();
        let paths: Vec<Vec<u32>> =
            used.iter().filter_map(|&b| bones::path_to(model, b, m.parent_deformer)).collect();
        let mut depth = 0;
        while let Some(&x) = paths.first().and_then(|p| p.get(depth))
            && paths.iter().all(|p| p.get(depth) == Some(&x))
        {
            depth += 1;
        }
        // weight left over follows the mesh's own parent, not a shared bone
        let whole = skin.offsets.windows(2).all(|s| {
            let (a, b) =
                (s.first().copied().unwrap_or(0) as usize, s.get(1).copied().unwrap_or(0) as usize);
            let w: f32 = skin.weights.get(a..b).unwrap_or(&[]).iter().sum();
            (w - 1.0).abs() < 1e-4
        });
        let shared = if whole && depth > 0 {
            paths.first().and_then(|p| p.get(depth - 1)).map(|&a| a as usize)
        } else {
            None
        };
        let (parent, below) = match shared {
            Some(a) => (Some(a), depth),
            None => (m.parent_deformer.map(|p| p as usize), 0),
        };
        let mut want = Vec::new();
        for p in &paths {
            for &e in p.get(below..).unwrap_or(&[]) {
                merge(&mut want, &driving(model, &plans, e as usize), true);
            }
        }
        let target = Target::ArtMesh(mi);
        if widen(model, &mut out, target, &m.id, &want, false, SAMPLING)? {
            report.notes.push(format!("{}: too many keys for the curve on some axes", m.id));
        }
        let (binding, count) = match out.art_meshes.get(mi) {
            Some(om) => (om.binding.clone(), om.binding.keyform_count()),
            None => continue,
        };
        merge(&mut touched, &own_axes(&m.binding), false);
        let len = (m.vertex_count as usize).saturating_mul(2);
        let mut positions = Vec::with_capacity(count.saturating_mul(len));
        for k in 0..count {
            set_pose(&mut orig, &defaults, &key_pose(&binding, k));
            let world = mesh_world(&orig, mi).unwrap_or_else(|| vec![0.0; len]);
            let (pairs, _) = world.as_chunks::<2>();
            for q in pairs {
                let (x, y) = into_parent(&orig, &domains, parent, (q[0], q[1]));
                positions.extend_from_slice(&[x, y]);
            }
        }
        // blend shape forms go across at rest
        set_pose(&mut orig, &defaults, &[]);
        let mut shapes = m.blend_shapes.clone();
        for s in &mut shapes {
            for block in s.positions.chunks_mut(len.max(1)) {
                if block.len() != len {
                    continue;
                }
                orig.apply_skin(mi, block);
                if let Some(pd) = m.parent_deformer {
                    orig.apply_deformer(pd as usize, block);
                }
                let (pairs, _) = block.as_chunks_mut::<2>();
                for q in pairs {
                    let (x, y) = into_parent(&orig, &domains, parent, (q[0], q[1]));
                    *q = [x, y];
                }
            }
        }
        if !m.blend_shapes.is_empty() {
            report
                .notes
                .push(format!("{}: its blend shapes follow its bones as they sit at rest", m.id));
        }
        // the blend shapes of the bones below the new parent move the mesh
        // too: each parameter they key becomes a blend shape of the mesh,
        // its forms the mesh posed at every key the bones (and the mesh's
        // own shape there) hold, so the runtime adds the same motion
        let mut bone_shapes: Vec<(u32, Vec<f32>, Vec<WeightLimit>)> = Vec::new();
        for p in &paths {
            for &e in p.get(below..).unwrap_or(&[]) {
                let Some(d) = model.deformers.get(e as usize) else { continue };
                for s in &d.blend_shapes {
                    match bone_shapes.iter_mut().find(|b| b.0 == s.parameter) {
                        Some(b) => b.1.extend_from_slice(&s.keys),
                        None => bone_shapes.push((s.parameter, s.keys.clone(), s.limits.clone())),
                    }
                }
            }
        }
        for (param, mut keys, limits) in bone_shapes {
            let own = shapes.iter().position(|s| s.parameter == param);
            if let Some(s) = own.and_then(|o| m.blend_shapes.get(o)) {
                keys.extend_from_slice(&s.keys);
            }
            keys.push(0.0);
            keys.sort_by(f32::total_cmp);
            keys.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
            // a blend shape goes straight between its keys where the bones
            // turn on an arc: keys inside every span, the base key's too,
            // keep the chord close
            let mut keys: Vec<f32> = keys
                .windows(2)
                .flat_map(|w| {
                    let (lo, hi) =
                        (w.first().copied().unwrap_or(0.0), w.get(1).copied().unwrap_or(0.0));
                    (0..SHAPE_SPLIT).map(move |i| lo + (hi - lo) * i as f32 / SHAPE_SPLIT as f32)
                })
                .chain(keys.last().copied())
                .collect();
            keys.retain(|k| k.abs() > 1e-6);
            if keys.is_empty() {
                continue;
            }
            let mut forms = Vec::with_capacity(keys.len().saturating_mul(len));
            for &v in &keys {
                set_pose(&mut orig, &defaults, &[(param, v)]);
                let world = mesh_world(&orig, mi).unwrap_or_else(|| vec![0.0; len]);
                let (pairs, _) = world.as_chunks::<2>();
                for q in pairs {
                    let (x, y) = into_parent(&orig, &domains, parent, (q[0], q[1]));
                    forms.extend_from_slice(&[x, y]);
                }
            }
            match own.and_then(|o| shapes.get_mut(o)) {
                // the mesh's own shape on the same parameter is in the forms
                // already; its other channels stay when its keys did
                Some(s) if s.keys == keys => s.positions = forms,
                Some(s) => {
                    report.notes.push(format!(
                        "{}: its own blend shape keys moved to its bones', its opacity and colour there dropped",
                        m.id
                    ));
                    *s = BlendShape {
                        parameter: param,
                        keys,
                        positions: forms,
                        limits,
                        ..BlendShape::default()
                    };
                }
                None => shapes.push(BlendShape {
                    parameter: param,
                    keys,
                    positions: forms,
                    limits,
                    ..BlendShape::default()
                }),
            }
        }
        set_pose(&mut orig, &defaults, &[]);
        if let Some(om) = out.art_meshes.get_mut(mi) {
            om.positions = positions;
            om.blend_shapes = shapes;
            om.parent_deformer = parent.map(|p| p as u32);
        }
        report.meshes += 1;
    }

    for m in &mut out.art_meshes {
        m.skin = None;
    }
    out.ik.clear();

    let (at_keys, between_keys) = sweep(model, &out, &touched);
    report.at_keys = at_keys;
    report.between_keys = between_keys;
    Ok((out, report))
}

/// The largest vertex distance between `a` and `b`, over poses at the keys
/// of `axes` and between them: each axis alone, then a few spread over
/// all of them at once.
fn sweep(a: &Model, b: &Model, axes: &Axes) -> (f32, f32) {
    let defaults: Vec<f32> = a.parameters.iter().map(|p| p.default).collect();
    let (mut pa, mut pb) = (Puppet::new(a.clone()), Puppet::new(b.clone()));
    let mut worst = |pose: &[(u32, f32)]| {
        set_pose(&mut pa, &defaults, pose);
        set_pose(&mut pb, &defaults, pose);
        let mut d = 0.0f32;
        for mi in 0..a.art_meshes.len() {
            if !pa.mesh_visible.get(mi).copied().unwrap_or(false) {
                continue;
            }
            let (Some(x), Some(y)) = (pa.mesh_drawn_positions(mi), pb.mesh_drawn_positions(mi))
            else {
                continue;
            };
            for (u, v) in x.iter().zip(&y) {
                d = d.max((u - v).abs());
            }
        }
        d
    };
    let mids = |keys: &[f32]| -> Vec<f32> {
        keys.windows(2)
            .flat_map(|w| {
                let (lo, hi) =
                    (w.first().copied().unwrap_or(0.0), w.get(1).copied().unwrap_or(0.0));
                [lo + (hi - lo) * 0.3, lo + (hi - lo) * 0.45, lo + (hi - lo) * 0.7]
            })
            .collect()
    };
    let (mut at, mut between) = (0.0f32, 0.0f32);
    for (p, keys, _) in axes {
        for &k in keys {
            at = at.max(worst(&[(*p, k)]));
        }
        for k in mids(keys) {
            between = between.max(worst(&[(*p, k)]));
        }
    }
    // a small fixed generator, so the sweep is the same every time
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut pick = |len: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 33) as usize % len.max(1)
    };
    for _ in 0..48 {
        let keyed: Vec<(u32, f32)> = axes
            .iter()
            .map(|(p, keys, _)| (*p, keys.get(pick(keys.len())).copied().unwrap_or(0.0)))
            .collect();
        at = at.max(worst(&keyed));
        let off: Vec<(u32, f32)> = axes
            .iter()
            .map(|(p, keys, _)| {
                let m = mids(keys);
                (*p, m.get(pick(m.len())).copied().unwrap_or(0.0))
            })
            .collect();
        between = between.max(worst(&off));
    }
    (at, between)
}

#[cfg(test)]
mod tests {
    use ayatsuri_format::model::{
        ArtMesh, Binding, BindingAxis, BlendShape, BoneData, Deformer, DeformerKind, IkConstraint,
        Model, Parameter, RotationData, Skin, WarpData,
    };

    use super::bake_bones;
    use crate::Puppet;

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

    fn axis(param: u32, keys: &[f32]) -> Binding {
        Binding {
            axes: vec![BindingAxis { parameter: param, keys: keys.to_vec() }],
            curves: Vec::new(),
        }
    }

    /// A bone at `origin` under `parent`, turning `turn` degrees per unit
    /// of `param` at each of `keys`.
    fn bone(
        parent: Option<u32>,
        param: u32,
        keys: &[f32],
        origin: (f32, f32),
        turn: f32,
    ) -> Deformer {
        let n = keys.len();
        let rotation = RotationData {
            base_angle: 0.0,
            opacities: vec![1.0; n],
            angles: keys.iter().map(|k| k * turn).collect(),
            origins_x: vec![origin.0; n],
            origins_y: vec![origin.1; n],
            scales: vec![1.0; n],
            reflect_x: vec![false; n],
            reflect_y: vec![false; n],
        };
        Deformer {
            id: format!("B{param}_{}", origin.0),
            parent_part: None,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            binding: axis(param, keys),
            kind: DeformerKind::Bone(BoneData::new(rotation, 1.0)),
            multiply: Vec::new(),
            screen: Vec::new(),
            blend_shapes: Vec::new(),
        }
    }

    /// A 1x1 warp over [0,2]^2 whose top edge shears sideways on `param`.
    fn warp(param: u32) -> Deformer {
        let mut positions = Vec::new();
        for shear in [-0.5f32, 0.0, 0.5] {
            positions.extend_from_slice(&[0.0, 0.0, 2.0, 0.0, shear, 2.0, 2.0 + shear, 2.0]);
        }
        Deformer {
            id: "W".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding: axis(param, &[-1.0, 0.0, 1.0]),
            kind: DeformerKind::Warp(WarpData {
                rows: 1,
                cols: 1,
                quad: true,
                opacities: vec![1.0; 3],
                positions,
            }),
            multiply: Vec::new(),
            screen: Vec::new(),
            blend_shapes: Vec::new(),
        }
    }

    fn mesh(parent: Option<u32>, positions: Vec<f32>, skin: Option<Skin>) -> ArtMesh {
        let n = positions.len() / 2;
        ArtMesh {
            id: format!("M{}", positions.first().copied().unwrap_or(0.0)),
            parent_part: None,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: n as u32,
            uvs: vec![0.0; n * 2],
            indices: vec![0, 1, 2],
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
            skin,
        }
    }

    fn model(deformers: Vec<Deformer>, meshes: Vec<ArtMesh>) -> Model {
        let mut m = Model::new();
        m.parameters = (0..3).map(|i| param(&format!("P{i}"))).collect();
        m.deformers = deformers;
        m.art_meshes = meshes;
        m
    }

    const TRI: [f32; 6] = [0.1, 0.2, 0.6, -0.3, -0.4, 0.5];

    /// Every pose with each parameter at -1, 0 or 1.
    fn at_keys() -> Vec<[f32; 3]> {
        const V: [f32; 3] = [-1.0, 0.0, 1.0];
        let mut out = Vec::new();
        for a in V {
            for b in V {
                for c in V {
                    out.push([a, b, c]);
                }
            }
        }
        out
    }

    /// Poses off the keys (the bake keys two-key axes at their middle too).
    fn between() -> Vec<[f32; 3]> {
        const V: [f32; 4] = [-0.75, -0.3, 0.25, 0.6];
        let mut out = Vec::new();
        for a in V {
            for b in V {
                for c in V {
                    out.push([a, b, c]);
                }
            }
        }
        out
    }

    /// The worst vertex distance between `a` and its bake over `poses`.
    fn worst(a: &Model, b: &Model, poses: &[[f32; 3]]) -> f32 {
        let (mut pa, mut pb) = (Puppet::new(a.clone()), Puppet::new(b.clone()));
        let mut d = 0.0f32;
        for v in poses {
            for (i, &x) in v.iter().enumerate() {
                pa.set_value(i, x);
                pb.set_value(i, x);
            }
            pa.update();
            pb.update();
            for mi in 0..a.art_meshes.len() {
                let (x, y) =
                    (pa.mesh_drawn_positions(mi).unwrap(), pb.mesh_drawn_positions(mi).unwrap());
                for (u, w) in x.iter().zip(&y) {
                    d = d.max((u - w).abs());
                }
            }
        }
        d
    }

    /// Bake, check nothing of the bones is left, and give the worst error
    /// at keys and between them.
    fn bake(name: &str, m: &Model) -> (Model, f32, f32) {
        let (out, report) = bake_bones(m).unwrap();
        assert!(out.deformers.iter().all(|d| !d.kind.is_bone()), "{name}: a bone is left");
        assert!(out.art_meshes.iter().all(|m| m.skin.is_none()) && out.ik.is_empty());
        assert_eq!(out.deformers.len(), m.deformers.len());
        let (k, b) = (worst(m, &out, &at_keys()), worst(m, &out, &between()));
        eprintln!(
            "{name}: at keys {k:.2e}, between {b:.2e}; report {} / {} ({:?})",
            report.at_keys, report.between_keys, report.notes
        );
        (out, k, b)
    }

    fn shear(d: &mut Deformer, by: f32) {
        let DeformerKind::Bone(b) = &mut d.kind else { panic!("bone") };
        let n = b.rotation.angles.len();
        b.shears_x = vec![by; n];
        b.scales_y = vec![0.6; n];
    }

    #[test]
    fn a_plain_bone_is_a_rotation_deformer_exactly() {
        let m = model(
            vec![bone(None, 0, &[-1.0, 0.0, 1.0], (0.2, 0.1), 30.0)],
            vec![mesh(Some(0), TRI.to_vec(), None)],
        );
        let (out, k, b) = bake("rigid bone", &m);
        assert!(matches!(out.deformers[0].kind, DeformerKind::Rotation(_)));
        assert_eq!((k, b), (0.0, 0.0));
    }

    #[test]
    fn a_chain_of_three_bakes_exactly() {
        let m = model(
            vec![
                bone(None, 0, &[-1.0, 0.0, 1.0], (0.2, 0.1), 30.0),
                bone(Some(0), 1, &[-1.0, 1.0], (0.0, 1.0), 45.0),
                bone(Some(1), 2, &[-1.0, 0.0, 1.0], (0.0, 0.8), 20.0),
            ],
            vec![mesh(Some(2), TRI.to_vec(), None), mesh(Some(1), TRI.to_vec(), None)],
        );
        let (_, k, b) = bake("3-bone chain", &m);
        assert_eq!((k, b), (0.0, 0.0));
    }

    #[test]
    fn a_bone_under_a_warp_bakes_exactly() {
        let m = model(
            vec![warp(2), bone(Some(0), 0, &[-1.0, 0.0, 1.0], (0.5, 0.5), 30.0)],
            vec![mesh(Some(1), TRI.to_vec(), None)],
        );
        let (_, k, b) = bake("bone under a warp", &m);
        assert_eq!((k, b), (0.0, 0.0));
    }

    #[test]
    fn a_sheared_bone_becomes_a_warp_exact_at_keys() {
        let mut parent = bone(None, 0, &[-1.0, 1.0], (0.3, -0.2), 30.0);
        shear(&mut parent, 15.0);
        // a plain bone and a rotation deformer under it come along
        let child = bone(Some(0), 1, &[-1.0, 0.0, 1.0], (0.5, 1.0), 45.0);
        let mut rot = bone(Some(0), 2, &[-1.0, 0.0, 1.0], (-0.4, 0.6), 25.0);
        if let DeformerKind::Bone(b) = &rot.kind {
            rot.kind = DeformerKind::Rotation(b.rotation.clone());
        }
        let m = model(
            vec![parent, child, rot],
            vec![
                mesh(Some(0), TRI.to_vec(), None),
                mesh(Some(1), TRI.to_vec(), None),
                mesh(Some(2), TRI.to_vec(), None),
            ],
        );
        let (out, k, b) = bake("sheared bone", &m);
        assert!(matches!(out.deformers[0].kind, DeformerKind::Warp(_)));
        assert!(matches!(out.deformers[1].kind, DeformerKind::Warp(_)));
        assert!(matches!(out.deformers[2].kind, DeformerKind::Rotation(_)));
        assert!(k < 1e-4, "{k}");
        assert!(b < 2e-2, "{b}");
    }

    fn skin(per_vertex: &[&[(u32, f32)]]) -> Skin {
        Skin::from_pairs(per_vertex.iter().copied())
    }

    #[test]
    fn a_mesh_weighted_across_a_parent_and_child_bone() {
        let m = model(
            vec![
                bone(None, 0, &[-1.0, 0.0, 1.0], (0.2, 0.1), 30.0),
                bone(Some(0), 1, &[-1.0, 1.0], (0.0, 1.0), 45.0),
            ],
            vec![mesh(
                None,
                TRI.to_vec(),
                Some(skin(&[&[(1, 1.0)], &[(0, 0.5), (1, 0.5)], &[(0, 1.0)]])),
            )],
        );
        let (out, k, b) = bake("parent and child weights", &m);
        // under the shared bone, keyed on the child's parameter only
        assert_eq!(out.art_meshes[0].parent_deformer, Some(0));
        assert_eq!(out.art_meshes[0].binding.axes.len(), 1);
        assert!(k < 1e-4, "{k}");
        assert!(b < 1e-2, "{b}");
    }

    #[test]
    fn a_mesh_weighted_to_two_sibling_bones() {
        let m = model(
            vec![
                bone(None, 0, &[-1.0, 0.0, 1.0], (0.2, 0.1), 30.0),
                bone(None, 1, &[-1.0, 0.0, 1.0], (-0.3, 0.4), 40.0),
            ],
            vec![mesh(
                None,
                TRI.to_vec(),
                Some(skin(&[&[(1, 1.0)], &[(0, 0.5), (1, 0.5)], &[(0, 0.7), (1, 0.3)]])),
            )],
        );
        let (out, k, b) = bake("sibling weights", &m);
        assert_eq!(out.art_meshes[0].parent_deformer, None);
        assert!(k < 1e-4, "{k}");
        assert!(b < 1e-2, "{b}");
    }

    /// A bone keyed on a blend shape parameter only: no binding, a blend
    /// shape turning it.
    fn shaped_bone(
        parent: Option<u32>,
        origin: (f32, f32),
        keys: &[f32],
        turns: &[f32],
    ) -> Deformer {
        let mut d = bone(parent, 0, &[0.0], origin, 0.0);
        d.binding = Binding::default();
        let n = keys.len();
        d.blend_shapes = vec![BlendShape {
            parameter: 2,
            keys: keys.to_vec(),
            opacities: vec![1.0; n],
            angles: turns.to_vec(),
            origins_x: vec![origin.0; n],
            origins_y: vec![origin.1; n],
            scales: vec![1.0; n],
            ..BlendShape::default()
        }];
        d
    }

    #[test]
    fn bones_on_a_blend_shape_move_their_weighted_mesh() {
        let mut m = model(
            vec![
                shaped_bone(None, (0.2, 0.1), &[-1.0, 0.5, 1.0], &[10.0, -6.0, 4.0]),
                shaped_bone(Some(0), (0.0, 1.0), &[-1.0, 0.5, 1.0], &[30.0, -20.0, -45.0]),
            ],
            vec![mesh(
                None,
                TRI.to_vec(),
                Some(skin(&[&[(1, 1.0)], &[(0, 0.5), (1, 0.5)], &[(0, 1.0)]])),
            )],
        );
        m.parameters[2].blend_shape = true;
        let (out, report) = bake_bones(&m).unwrap();
        let mesh = &out.art_meshes[0];
        assert_eq!(mesh.parent_deformer, Some(0));
        assert!(mesh.blend_shapes.iter().any(|s| s.parameter == 2), "the turn reaches the mesh");
        // the shaped parameter alone, at the bones' keys and off them
        let along = |vs: &[f32]| -> Vec<[f32; 3]> { vs.iter().map(|&v| [0.0, 0.0, v]).collect() };
        let k = worst(&m, &out, &along(&[-1.0, 0.0, 0.5, 1.0]));
        let b = worst(&m, &out, &along(&[-0.6, -0.2, 0.25, 0.7, 0.9]));
        eprintln!(
            "shaped bones: at keys {k:.2e}, between {b:.2e}; report {} / {}",
            report.at_keys, report.between_keys
        );
        assert!(k < 1e-4, "{k}");
        assert!(b < 1e-2, "{b}");
        // the sweep sees the shaped parameter too
        assert!(report.between_keys > 0.0 || b == 0.0);
    }

    #[test]
    fn an_ik_chain_bakes_its_solved_turns() {
        // the target (0) moves sideways on P2; the arm (1, 2) reaches for it
        let mut target = bone(None, 2, &[-1.0, 0.0, 1.0], (1.2, 0.5), 0.0);
        if let DeformerKind::Bone(b) = &mut target.kind {
            b.rotation.origins_x = vec![0.4, 1.2, 1.4];
        }
        let mut m = model(
            vec![
                target,
                bone(None, 0, &[-1.0, 0.0, 1.0], (0.0, 0.0), 10.0),
                bone(Some(1), 1, &[-1.0, 0.0, 1.0], (0.0, 1.0), 10.0),
            ],
            vec![
                mesh(Some(1), vec![0.0, 1.0, 0.5, 0.0, 0.0, 0.5], None),
                mesh(Some(2), vec![0.0, 1.0, 0.5, 0.0, 0.0, 0.5], None),
            ],
        );
        m.ik.push(IkConstraint {
            id: "IK".into(),
            bones: vec![1, 2],
            target: 0,
            binding: Binding::default(),
            mix: vec![1.0],
            bend_positive: vec![true],
        });
        let (_, k, b) = bake("IK chain", &m);
        assert!(k < 1e-4, "{k}");
        assert!(b < 5e-2, "{b}");
    }

    #[test]
    fn a_mesh_with_its_own_keyforms_and_weights() {
        let mut weighted = mesh(
            None,
            TRI.to_vec(),
            Some(skin(&[&[(1, 1.0)], &[(0, 0.4), (1, 0.6)], &[(0, 1.0)]])),
        );
        // the mesh stretches on P2 by itself
        weighted.binding = axis(2, &[-1.0, 1.0]);
        let stretched: Vec<f32> = TRI.iter().map(|v| v * 1.3).collect();
        weighted.positions = [TRI.to_vec(), stretched].concat();
        weighted.opacities = vec![1.0; 2];
        weighted.draw_orders = vec![500.0; 2];
        let m = model(
            vec![
                bone(None, 0, &[-1.0, 0.0, 1.0], (0.2, 0.1), 30.0),
                bone(Some(0), 1, &[-1.0, 0.0, 1.0], (0.0, 1.0), 45.0),
            ],
            vec![weighted],
        );
        let (_, k, b) = bake("own keyforms plus weights", &m);
        assert!(k < 1e-4, "{k}");
        assert!(b < 1e-2, "{b}");
    }
}
