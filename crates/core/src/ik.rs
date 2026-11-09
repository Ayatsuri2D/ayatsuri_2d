//! IK: a chain of one or two bones turned so it reaches a target bone's
//! joint, solved analytically each frame (aim for one bone, the law of
//! cosines for two). The constraints that check out are planned at load.

use ayatsuri_format::model::{DeformerKind, Model};

use crate::deform::Vec2;

/// One constraint that checks out, by deformer index.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Plan {
    pub constraint: usize,
    pub root: usize,
    pub child: Option<usize>,
    pub target: usize,
}

/// The constraints that can be solved: one or two bones, the second the
/// first's child, a bone target placed before the chain, keyforms that
/// match the binding. A bone already in an earlier chain is not taken
/// again.
pub(crate) fn plans(model: &Model) -> Vec<Plan> {
    let is_bone =
        |i: usize| model.deformers.get(i).is_some_and(|d| matches!(d.kind, DeformerKind::Bone(_)));
    let mut out: Vec<Plan> = Vec::new();
    for (ci, c) in model.ik.iter().enumerate() {
        let (root, child) = match c.bones[..] {
            [r] => (r as usize, None),
            [r, k] => (r as usize, Some(k as usize)),
            _ => continue,
        };
        let target = c.target as usize;
        let n = c.binding.keyform_count();
        let ok = is_bone(root)
            && child.is_none_or(|k| {
                is_bone(k)
                    && model.deformers.get(k).and_then(|d| d.parent_deformer) == Some(root as u32)
            })
            && is_bone(target)
            && target < root
            && c.mix.len() == n
            && c.bend_positive.len() == n
            && !out.iter().any(|p| {
                [Some(p.root), p.child].contains(&Some(root))
                    || child.is_some_and(|k| [Some(p.root), p.child].contains(&Some(k)))
            });
        if ok {
            out.push(Plan { constraint: ci, root, child, target });
        }
    }
    out
}

/// `a` wrapped to [-pi, pi].
fn wrap(a: f32) -> f32 {
    let tau = core::f32::consts::TAU;
    a - tau * (a / tau).round()
}

fn angle(v: Vec2) -> f32 {
    v.y.atan2(v.x)
}

fn sub(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 { x: a.x - b.x, y: a.y - b.y }
}

fn len(v: Vec2) -> f32 {
    v.x.hypot(v.y)
}

/// The turn (radians, counterclockwise) that points `dir`, from `joint`,
/// at `target`.
pub(crate) fn aim(joint: Vec2, dir: Vec2, target: Vec2) -> Option<f32> {
    let to = sub(target, joint);
    if len(to) < 1e-9 || len(dir) < 1e-9 {
        return None;
    }
    Some(wrap(angle(to) - angle(dir)))
}

/// The turns (radians, counterclockwise) of a two-bone chain, joints `j0`
/// and `j1`, tip `tip`, so the tip reaches `target`: the root's, then the
/// child's on top of it. Out of reach, both point at the target. The
/// child's turn counts the root's when `child_follows` (it inherits its
/// parent's turn).
pub(crate) fn two_bone(
    j0: Vec2,
    j1: Vec2,
    tip: Vec2,
    target: Vec2,
    bend_positive: bool,
    child_follows: bool,
) -> Option<(f32, f32)> {
    let (v1, v2, to) = (sub(j1, j0), sub(tip, j1), sub(target, j0));
    let (l1, l2, d) = (len(v1), len(v2), len(to));
    if l1 < 1e-9 || d < 1e-9 {
        return None;
    }
    let a1 = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0).acos();
    // the turn at the elbow, 0 when straight
    let bend = if l2 < 1e-9 {
        0.0
    } else {
        core::f32::consts::PI
            - ((l1 * l1 + l2 * l2 - d * d) / (2.0 * l1 * l2)).clamp(-1.0, 1.0).acos()
    };
    let s = if bend_positive { 1.0 } else { -1.0 };
    let first = angle(to) - s * a1;
    let second = first + s * bend;
    let root = wrap(first - angle(v1));
    let carried = if child_follows { root } else { 0.0 };
    let child = if l2 < 1e-9 { 0.0 } else { wrap(second - (angle(v2) + carried)) };
    Some((root, child))
}
