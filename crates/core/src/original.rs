//! Revert to Original (Cubism 5.2): an art mesh or deformer goes back to a
//! shape recorded on the canvas earlier, whatever its parents have done
//! since. A deformer's shape is taken when it is made and, while its Auto
//! Update is on, each time a key is added; a mesh's when its first key is
//! added, which is the art as imported. Update Original Shape takes either
//! by hand. An object with no shape taken goes back to how it is drawn at
//! every parameter's default.

use ayatsuri_format::Model;
use ayatsuri_format::model::{DeformerKind, OriginalHow, OriginalShape};

use crate::Puppet;
use crate::edit::Target;

fn key(model: &Model, target: Target) -> Option<(bool, &str)> {
    match target {
        Target::ArtMesh(i) => model.art_meshes.get(i).map(|m| (false, m.id.as_str())),
        Target::Deformer(i) => model.deformers.get(i).map(|d| (true, d.id.as_str())),
        Target::Part(_) => None,
    }
}

/// The object's shape on the canvas now, as an original shape keeps it.
pub fn shape_now(puppet: &Puppet, target: Target) -> Option<Vec<f32>> {
    match target {
        Target::ArtMesh(i) => puppet.mesh_drawn_positions(i),
        Target::Deformer(i) => match &puppet.model().deformers.get(i)?.kind {
            DeformerKind::Warp(_) => puppet.warp_grid(i).map(|(g, _, _)| g.to_vec()),
            DeformerKind::Rotation(_) => {
                let r = puppet.rotation_state(i)?;
                Some(vec![r.origin.x, r.origin.y, r.angle, r.scale])
            }
        },
        Target::Part(_) => None,
    }
}

/// The object's recorded original shape, also when its point count no
/// longer fits.
pub fn find(model: &Model, target: Target) -> Option<&OriginalShape> {
    let (deformer, id) = key(model, target)?;
    model.original_shapes.iter().find(|o| o.deformer == deformer && o.id == id)
}

/// Whether taking a shape `how` would record anything. As in Cubism, a key
/// added takes it only on an object with no keys yet (its first key), and
/// leaves alone a deformer with Auto Update off and a mesh that has one.
pub fn takes(model: &Model, target: Target, how: OriginalHow) -> bool {
    if how != OriginalHow::KeyAdded {
        return key(model, target).is_some();
    }
    if crate::shapes::normal_count(model, target) > 1 {
        return false;
    }
    match (target, find(model, target)) {
        (Target::ArtMesh(_), Some(_)) => false,
        (Target::Deformer(_), Some(o)) => o.auto,
        (Target::Part(_), _) => false,
        _ => true,
    }
}

/// Record `points` (from [`shape_now`]) as the object's original shape. A
/// shape taken by hand turns Auto Update off, as in Cubism.
pub fn record(
    model: &mut Model,
    target: Target,
    points: Vec<f32>,
    how: OriginalHow,
    time: u64,
) -> bool {
    if !takes(model, target, how) {
        return false;
    }
    let Some((deformer, id)) = key(model, target).map(|(d, id)| (d, id.to_string())) else {
        return false;
    };
    prune(model);
    let auto = how != OriginalHow::Manual && find(model, target).is_none_or(|o| o.auto);
    let shape = OriginalShape { id, deformer, points, how, time, auto };
    match model.original_shapes.iter_mut().find(|o| o.deformer == deformer && o.id == shape.id) {
        Some(o) => *o = shape,
        None => model.original_shapes.push(shape),
    }
    true
}

/// Turn a deformer's Auto Update on or off. Off with no shape taken yet
/// records nothing to switch, so it is refused.
pub fn set_auto(model: &mut Model, target: Target, on: bool) -> bool {
    let Some((deformer, id)) = key(model, target).map(|(d, id)| (d, id.to_string())) else {
        return false;
    };
    match model.original_shapes.iter_mut().find(|o| o.deformer == deformer && o.id == id) {
        Some(o) if o.auto != on => {
            o.auto = on;
            true
        }
        _ => false,
    }
}

/// Delete Original Shape of All Deformers: meshes keep theirs.
pub fn clear_deformers(model: &mut Model) -> bool {
    let before = model.original_shapes.len();
    model.original_shapes.retain(|o| !o.deformer);
    model.original_shapes.len() != before
}

/// Drop shapes whose object is gone.
fn prune(model: &mut Model) {
    let Model { original_shapes, art_meshes, deformers, .. } = model;
    original_shapes.retain(|o| {
        if o.deformer {
            deformers.iter().any(|d| d.id == o.id)
        } else {
            art_meshes.iter().any(|m| m.id == o.id)
        }
    });
}

/// A rename carries the object's shape along.
pub fn rename(shapes: &mut [OriginalShape], deformer: bool, from: &str, to: &str) {
    for o in shapes.iter_mut().filter(|o| o.deformer == deformer && o.id == from) {
        o.id = to.to_string();
    }
}

/// What reverting does to the keyform the parameters sit on, worked out
/// against the puppet as it stands, so the model can be edited after.
#[derive(Clone, Debug, PartialEq)]
pub enum Revert {
    /// Per point: index and its move in the parent's space.
    Mesh(Vec<(usize, f32, f32)>),
    Warp(Vec<(usize, f32, f32)>),
    /// Origin move in the parent's space, degrees to add, scale factor.
    Rotation {
        dx: f32,
        dy: f32,
        angle: f32,
        scale: f32,
    },
}

/// The point in `parent`'s space that lands on `goal` on the canvas,
/// searched from `start` (Newton's method on the parent's map; no parent
/// is the canvas itself).
fn local_of(
    puppet: &Puppet,
    parent: Option<u32>,
    start: (f32, f32),
    goal: (f32, f32),
) -> (f32, f32) {
    let Some(pd) = parent else { return goal };
    let map = |x: f32, y: f32| puppet.deformer_point(pd as usize, x, y);
    let (mut x, mut y) = start;
    for _ in 0..24 {
        let (fx, fy) = map(x, y);
        let (ex, ey) = (goal.0 - fx, goal.1 - fy);
        if ex.abs() + ey.abs() < 1e-6 {
            break;
        }
        let e = 1e-3;
        let (ax, ay) = map(x + e, y);
        let (bx, by) = map(x, y + e);
        let (j00, j10, j01, j11) = ((ax - fx) / e, (ay - fy) / e, (bx - fx) / e, (by - fy) / e);
        let det = j00 * j11 - j01 * j10;
        if det.abs() < 1e-12 || !det.is_finite() {
            break;
        }
        x += (j11 * ex - j01 * ey) / det;
        y += (j00 * ey - j10 * ex) / det;
    }
    (x, y)
}

/// The shape to go back to: the recorded one when it still fits, else the
/// object as drawn at every parameter's default.
pub fn goal(puppet: &Puppet, target: Target) -> Option<Vec<f32>> {
    let len = shape_now(puppet, target)?.len();
    if let Some(o) = find(puppet.model(), target).filter(|o| o.points.len() == len) {
        return Some(o.points.clone());
    }
    // a deformer with none recorded is Uncreated in Cubism: nothing to go to
    if matches!(target, Target::Deformer(_)) {
        return None;
    }
    let mut rest = Puppet::new(puppet.model().clone());
    rest.update();
    shape_now(&rest, target).filter(|p| p.len() == len)
}

/// Work out Revert to Original for one object on the puppet as it stands.
pub fn revert(puppet: &Puppet, target: Target) -> Option<Revert> {
    let goal = goal(puppet, target)?;
    let model = puppet.model();
    let moves = |parent: Option<u32>, local: Vec<f32>| -> Vec<(usize, f32, f32)> {
        local
            .as_chunks::<2>()
            .0
            .iter()
            .zip(goal.as_chunks::<2>().0)
            .enumerate()
            .filter_map(|(i, (&[lx, ly], &[gx, gy]))| {
                let (x, y) = local_of(puppet, parent, (lx, ly), (gx, gy));
                let (dx, dy) = (x - lx, y - ly);
                (dx.is_finite() && dy.is_finite() && (dx != 0.0 || dy != 0.0))
                    .then_some((i, dx, dy))
            })
            .collect()
    };
    match target {
        Target::ArtMesh(i) => {
            let parent = model.art_meshes.get(i)?.parent_deformer;
            Some(Revert::Mesh(moves(parent, puppet.mesh_local_positions(i)?)))
        }
        Target::Deformer(i) => {
            let d = model.deformers.get(i)?;
            match d.kind {
                DeformerKind::Warp(_) => {
                    Some(Revert::Warp(moves(d.parent_deformer, puppet.warp_local_grid(i)?)))
                }
                DeformerKind::Rotation(_) => {
                    let now = puppet.rotation_state(i)?;
                    let (lx, ly) = puppet.rotation_local_origin(i)?;
                    let [gx, gy, ga, gs] = goal[..] else { return None };
                    let (x, y) = local_of(puppet, d.parent_deformer, (lx, ly), (gx, gy));
                    let scale = if now.scale.abs() > 1e-9 { gs / now.scale } else { 1.0 };
                    Some(Revert::Rotation {
                        dx: x - lx,
                        dy: y - ly,
                        angle: ga - now.angle,
                        scale: if scale.is_finite() && scale > 0.0 { scale } else { 1.0 },
                    })
                }
            }
        }
        Target::Part(_) => None,
    }
}

/// Write a worked-out revert into one keyform.
pub fn apply(model: &mut Model, target: Target, keyform: usize, revert: &Revert) -> bool {
    match (target, revert) {
        (Target::ArtMesh(i), Revert::Mesh(moves)) => {
            !moves.is_empty() && crate::edit::move_vertices_in(model, i, Some(keyform), moves)
        }
        (Target::Deformer(i), Revert::Warp(moves)) => {
            !moves.is_empty() && crate::edit::move_warp_points(model, i, Some(keyform), moves)
        }
        (Target::Deformer(i), &Revert::Rotation { dx, dy, angle, scale }) => {
            let Some(DeformerKind::Rotation(r)) = model.deformers.get_mut(i).map(|d| &mut d.kind)
            else {
                return false;
            };
            let (Some(ox), Some(oy), Some(a), Some(s)) = (
                r.origins_x.get_mut(keyform),
                r.origins_y.get_mut(keyform),
                r.angles.get_mut(keyform),
                r.scales.get_mut(keyform),
            ) else {
                return false;
            };
            let before = (*ox, *oy, *a, *s);
            *ox += dx;
            *oy += dy;
            *a += angle;
            *s *= scale;
            before != (*ox, *oy, *a, *s)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{self, NewDeformer};

    fn puppet_with_warp_and_rotation() -> (Model, usize, usize) {
        let mut m = Model::new();
        m.parameters.push(ayatsuri_format::model::Parameter {
            id: "P".into(),
            min: -1.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape: false,
        });
        let warp = edit::create_deformer(
            &mut m,
            NewDeformer::Warp { rows: 2, cols: 2 },
            &[],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "W".into(),
        )
        .unwrap();
        let rot = edit::create_deformer(
            &mut m,
            NewDeformer::Rotation { origin: Some((0.5, 0.5)) },
            &[],
            Some(warp as u32),
            (0.25, 0.25, 0.75, 0.75),
            None,
            "R".into(),
        )
        .unwrap();
        (m, warp, rot)
    }

    /// The mesh points as drawn, after reverting `mesh` on `m`.
    fn reverted(m: &mut Model, mesh: usize) -> Vec<f32> {
        let mut p = Puppet::new(m.clone());
        p.update();
        let rv = revert(&p, Target::ArtMesh(mesh)).unwrap();
        apply(m, Target::ArtMesh(mesh), 0, &rv);
        let mut p = Puppet::new(m.clone());
        p.update();
        p.mesh_drawn_positions(mesh).unwrap()
    }

    #[test]
    fn a_point_the_parent_cannot_carry_back_stays_put_instead_of_flying_off() {
        let (mut m, w, _) = puppet_with_warp_and_rotation();
        // a 5 x 5 mesh in the warp
        let n = 5;
        let mut uvs = Vec::new();
        for j in 0..n {
            for i in 0..n {
                uvs.extend([0.1 + 0.8 * i as f32 / 4.0, 0.1 + 0.8 * j as f32 / 4.0]);
            }
        }
        let mut indices = Vec::new();
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u16;
                indices.extend([a, a + 1, a + n as u16, a + 1, a + n as u16 + 1, a + n as u16]);
            }
        }
        m.art_meshes.push(ayatsuri_format::model::ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: (n * n) as u32,
            positions: uvs.clone(),
            uvs,
            indices,
            masks: Vec::new(),
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        });
        let mi = m.art_meshes.len() - 1;
        assert!(edit::set_parent(&mut m, edit::Child::Mesh(mi), Some(w)).is_some());
        let mut p = Puppet::new(m.clone());
        p.update();
        let want = p.mesh_drawn_positions(mi).unwrap();
        assert!(record(&mut m, Target::ArtMesh(mi), want.clone(), OriginalHow::Created, 1));
        // the warp folded over itself: its middle pulled past a corner
        assert!(edit::move_warp_points(&mut m, w, Some(0), &[(4, 0.9, 0.8), (8, -0.6, -0.7)]));
        let got = reverted(&mut m, mi);
        for (g, w) in got.as_chunks::<2>().0.iter().zip(want.as_chunks::<2>().0) {
            let off = ((g[0] - w[0]).powi(2) + (g[1] - w[1]).powi(2)).sqrt();
            assert!(off < 1.5, "a point flew off: {g:?}, its original {w:?}");
        }
    }

    #[test]
    fn a_reverted_warp_and_rotation_land_on_their_recorded_shapes() {
        let (mut m, w, r) = puppet_with_warp_and_rotation();
        let mut p = Puppet::new(m.clone());
        p.update();
        for t in [Target::Deformer(w), Target::Deformer(r)] {
            let now = shape_now(&p, t).unwrap();
            assert!(record(&mut m, t, now, OriginalHow::Created, 1));
        }
        let want_w = find(&m, Target::Deformer(w)).unwrap().points.clone();
        let want_r = find(&m, Target::Deformer(r)).unwrap().points.clone();
        // shear the warp and turn the rotation
        assert!(edit::move_warp_points(&mut m, w, Some(0), &[(0, 0.2, 0.1), (4, -0.1, 0.15)]));
        if let DeformerKind::Rotation(rd) = &mut m.deformers[r].kind {
            rd.angles[0] += 30.0;
            rd.scales[0] *= 1.5;
            rd.origins_x[0] += 0.1;
        }
        for t in [Target::Deformer(w), Target::Deformer(r)] {
            let mut p = Puppet::new(m.clone());
            p.update();
            let rv = revert(&p, t).unwrap();
            assert!(apply(&mut m, t, 0, &rv));
        }
        let mut p = Puppet::new(m.clone());
        p.update();
        for (t, want) in [(Target::Deformer(w), want_w), (Target::Deformer(r), want_r)] {
            let got = shape_now(&p, t).unwrap();
            for (a, b) in got.iter().zip(&want) {
                assert!((a - b).abs() < 1e-3, "{t:?}: {got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn auto_update_and_manual_takes() {
        let (mut m, w, _) = puppet_with_warp_and_rotation();
        let t = Target::Deformer(w);
        assert!(record(&mut m, t, vec![0.0; 18], OriginalHow::Created, 1));
        assert!(find(&m, t).unwrap().auto);
        assert!(record(&mut m, t, vec![1.0; 18], OriginalHow::KeyAdded, 2));
        assert!(record(&mut m, t, vec![2.0; 18], OriginalHow::Manual, 3));
        assert!(!find(&m, t).unwrap().auto);
        // auto off: a key added leaves it
        assert!(!record(&mut m, t, vec![3.0; 18], OriginalHow::KeyAdded, 4));
        assert_eq!(find(&m, t).unwrap().points[0], 2.0);
        assert!(set_auto(&mut m, t, true));
        assert!(record(&mut m, t, vec![3.0; 18], OriginalHow::KeyAdded, 4));
        // renames follow; clearing deformers keeps meshes
        assert!(edit::rename_deformer(&mut m, w, "W2"));
        assert_eq!(find(&m, t).unwrap().id, "W2");
        assert!(clear_deformers(&mut m));
        assert!(find(&m, t).is_none());
    }

    #[test]
    fn a_deformer_with_nothing_recorded_is_left_alone() {
        let (mut m, w, _) = puppet_with_warp_and_rotation();
        assert!(edit::move_warp_points(&mut m, w, Some(0), &[(4, 0.3, 0.3)]));
        let mut p = Puppet::new(m.clone());
        p.update();
        // Uncreated, as Cubism calls it: nothing to go back to
        assert_eq!(revert(&p, Target::Deformer(w)), None);
    }

    #[test]
    fn only_the_first_key_takes_the_shape() {
        let (mut m, w, _) = puppet_with_warp_and_rotation();
        let t = Target::Deformer(w);
        assert!(record(&mut m, t, vec![0.0; 18], OriginalHow::KeyAdded, 1));
        assert!(edit::add_keys(&mut m, t, 0, &[-1.0, 0.0, 1.0]));
        // keyed now: a further key leaves the shape, Auto Update or not
        assert!(find(&m, t).unwrap().auto);
        assert!(!record(&mut m, t, vec![5.0; 18], OriginalHow::KeyAdded, 2));
        assert_eq!(find(&m, t).unwrap().points[0], 0.0);
        // taking it by hand still works
        assert!(record(&mut m, t, vec![5.0; 18], OriginalHow::Manual, 3));
    }
}
