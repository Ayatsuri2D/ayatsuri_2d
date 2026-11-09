//! Bone weight skinning: the joints weighted meshes read, built at load.
//!
//! A joint is one bone as seen from one mesh parent: the chain of rotation
//! deformers and bones from just under that parent down to the bone, and
//! the inverse of where the chain sits at rest (every parameter at its
//! default). Each frame the chain is resolved again from its members'
//! local states the way the evaluator resolves a rotation under a rotation,
//! and the joint's map is "now after inverse rest". A mesh's weight slots
//! point at joints; a mesh whose weights do not check out is not skinned.

use ayatsuri_format::model::{Model, SKIN_SLOTS};

use crate::deform::{Affine, Bone, Parent, Resolved, Vec2, resolve_bone, resolve_under};

/// Every joint any mesh reads, and which joint each weight slot reads.
#[derive(Default)]
pub(crate) struct Skinning {
    /// Per joint: the bone (a deformer index).
    pub bone: Vec<u32>,
    /// Per joint: its chain as (offset, length) into `path`.
    pub span: Vec<(usize, usize)>,
    /// Chains of deformer indices, outermost first, the bone last.
    pub path: Vec<u32>,
    /// Per joint: the inverse of the chain's map at rest.
    pub rest_inv: Vec<Affine>,
    /// Per joint, this frame: the map from rest to now.
    pub now: Vec<Affine>,
    /// Per mesh: the joint each weight slot reads (`u32::MAX` for an empty
    /// slot); empty when the mesh is not skinned.
    pub slot_joints: Vec<Vec<u32>>,
    /// Per mesh: the bones it reads, so the cache redoes it when one moves.
    pub mesh_bones: Vec<Vec<u32>>,
}

/// A chain of rotation deformers and bones, outermost first, each in the
/// space of the one before, brought out into the first one's parent space
/// the way the evaluator resolves them; `local` gives each one's own state
/// (None when it is hidden). Allocates nothing.
pub(crate) fn chain(
    model: &Model,
    path: &[u32],
    local: impl Fn(usize) -> Option<Bone>,
) -> Option<Affine> {
    // the last link: whether it is a bone, and how it resolved
    let mut prev: Option<(bool, Resolved)> = None;
    for &di in path {
        let own = local(di as usize)?;
        let is_bone = model.deformers.get(di as usize)?.kind.is_bone();
        let above = prev.map(|(_, r)| r);
        let map = |q: Vec2| above.unwrap_or_default().map.apply_v(q);
        let res = if is_bone {
            let parent = match prev {
                None => Parent::Root,
                Some((true, r)) => Parent::Bone { world: r.map, plain: r.plain, scale: r.scale },
                Some((false, r)) => Parent::Probe { map: &map, dir: -10.0, scale: r.scale },
            };
            resolve_bone(&own, parent)
        } else {
            let mut rot = own.rotation();
            if let Some(r) = above {
                resolve_under(&mut rot, -10.0, r.scale, map);
            }
            Resolved { map: Affine::of(&rot), plain: false, scale: rot.scale, rotation: rot }
        };
        prev = Some((is_bone, res));
    }
    prev.map(|(_, r)| r.map)
}

/// The deformers from just under `top` (None: the root) down to `bone`,
/// outermost first, when `bone` is a bone under `top` with only rotation
/// deformers and bones between.
fn path_to(model: &Model, bone: u32, top: Option<u32>) -> Option<Vec<u32>> {
    if !model.deformers.get(bone as usize)?.kind.is_bone() {
        return None;
    }
    let mut path = Vec::new();
    let mut at = Some(bone);
    // bounded, so a parent loop in a broken file cannot spin
    for _ in 0..=model.deformers.len() {
        if at == top {
            path.reverse();
            return Some(path);
        }
        let i = at?;
        let d = model.deformers.get(i as usize)?;
        d.kind.rotation()?;
        path.push(i);
        at = d.parent_deformer;
    }
    None
}

impl Skinning {
    /// Check every mesh's weights and make the joints they read. `rest`
    /// gives a rotation deformer's or bone's local state at rest.
    pub(crate) fn new(model: &Model, rest: impl Fn(usize) -> Option<Bone>) -> Skinning {
        let mut s = Skinning::default();
        // (bone, mesh parent) of each joint, to share joints between meshes
        let mut keys: Vec<(u32, Option<u32>)> = Vec::new();
        for m in &model.art_meshes {
            let made = (|| {
                let skin = m.skin.as_ref()?;
                let len = (m.vertex_count as usize).checked_mul(SKIN_SLOTS)?;
                if skin.bones.len() != len || skin.weights.len() != len {
                    return None;
                }
                let mut slots = Vec::with_capacity(len);
                let mut bones: Vec<u32> = Vec::new();
                for (&b, &w) in skin.bones.iter().zip(&skin.weights) {
                    if !w.is_finite() {
                        return None;
                    }
                    if w == 0.0 {
                        slots.push(u32::MAX);
                        continue;
                    }
                    let key = (b, m.parent_deformer);
                    let j = match keys.iter().position(|k| *k == key) {
                        Some(j) => j,
                        None => {
                            let path = path_to(model, b, m.parent_deformer)?;
                            let rest_inv = chain(model, &path, &rest)?.inverse()?;
                            keys.push(key);
                            s.bone.push(b);
                            s.span.push((s.path.len(), path.len()));
                            s.path.extend_from_slice(&path);
                            s.rest_inv.push(rest_inv);
                            s.now.push(Affine::IDENTITY);
                            keys.len() - 1
                        }
                    };
                    slots.push(j as u32);
                    if !bones.contains(&b) {
                        bones.push(b);
                    }
                }
                Some((slots, bones))
            })();
            let (slots, bones) = made.unwrap_or_default();
            s.slot_joints.push(slots);
            s.mesh_bones.push(bones);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use ayatsuri_format::model::{
        ArtMesh, Binding, BindingAxis, BoneData, Deformer, DeformerKind, Model, Parameter,
        RotationData, SKIN_SLOTS, Skin, WarpData,
    };

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

    /// A rotation deformer or bone at `origin`, turning -`turn` to `turn`
    /// degrees as `param` goes -1 to 1.
    fn turner(
        bone: bool,
        parent: Option<u32>,
        param: u32,
        origin: (f32, f32),
        turn: f32,
    ) -> Deformer {
        let rotation = RotationData {
            base_angle: 0.0,
            opacities: vec![1.0; 3],
            angles: vec![-turn, 0.0, turn],
            origins_x: vec![origin.0; 3],
            origins_y: vec![origin.1; 3],
            scales: vec![1.0; 3],
            reflect_x: vec![false; 3],
            reflect_y: vec![false; 3],
        };
        Deformer {
            id: format!("D{param}"),
            parent_part: None,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            binding: Binding {
                axes: vec![BindingAxis { parameter: param, keys: vec![-1.0, 0.0, 1.0] }],
                curves: Vec::new(),
            },
            kind: if bone {
                DeformerKind::Bone(BoneData::new(rotation, 1.0))
            } else {
                DeformerKind::Rotation(rotation)
            },
            multiply: Vec::new(),
            screen: Vec::new(),
            blend_shapes: Vec::new(),
        }
    }

    /// A 1x1 warp over [0,2]^2 whose top edge shears sideways as `param`
    /// moves.
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
            binding: Binding {
                axes: vec![BindingAxis { parameter: param, keys: vec![-1.0, 0.0, 1.0] }],
                curves: Vec::new(),
            },
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

    /// One (bone, weight) list per vertex, packed into slots.
    fn skin(per_vertex: &[&[(u32, f32)]]) -> Skin {
        let mut s = Skin {
            bones: vec![0; per_vertex.len() * SKIN_SLOTS],
            weights: vec![0.0; per_vertex.len() * SKIN_SLOTS],
        };
        for (v, pairs) in per_vertex.iter().enumerate() {
            for (k, &(b, w)) in pairs.iter().enumerate() {
                s.bones[v * SKIN_SLOTS + k] = b;
                s.weights[v * SKIN_SLOTS + k] = w;
            }
        }
        s
    }

    fn model(deformers: Vec<Deformer>, meshes: Vec<ArtMesh>) -> Model {
        let mut m = Model::new();
        m.parameters = (0..3).map(|i| param(&format!("P{i}"))).collect();
        m.deformers = deformers;
        m.art_meshes = meshes;
        m
    }

    fn drawn(p: &Puppet, mesh: usize) -> Vec<f32> {
        p.mesh_drawn_positions(mesh).unwrap()
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    fn same_bits(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
    }

    const TRI: [f32; 6] = [0.1, 0.2, 0.6, -0.3, -0.4, 0.5];

    /// Poses every test runs through, one value per parameter.
    const POSES: [[f32; 3]; 5] =
        [[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [0.0, -1.0, 0.0], [0.7, 0.3, 1.0], [-1.0, 1.0, -0.6]];

    fn pose(p: &mut Puppet, v: [f32; 3]) {
        for (i, x) in v.into_iter().enumerate() {
            p.set_value(i, x);
        }
        p.update();
    }

    #[test]
    fn an_unweighted_bone_moves_its_children_as_a_rotation_does() {
        // a chain of two and a mesh under the second
        let chain = |bone: bool| {
            model(
                vec![
                    turner(bone, None, 0, (0.2, 0.1), 30.0),
                    turner(bone, Some(0), 1, (0.0, 1.0), 45.0),
                ],
                vec![mesh(Some(1), TRI.to_vec(), None)],
            )
        };
        let (mut bones, mut rotations) = (Puppet::new(chain(true)), Puppet::new(chain(false)));
        for v in POSES {
            pose(&mut bones, v);
            pose(&mut rotations, v);
            assert!(same_bits(&drawn(&bones, 0), &drawn(&rotations, 0)), "{v:?}");
        }
    }

    #[test]
    fn a_bone_under_a_warp_follows_it_as_a_rotation_does() {
        let made = |bone: bool| {
            model(
                vec![warp(2), turner(bone, Some(0), 0, (0.5, 0.5), 30.0)],
                vec![mesh(Some(1), TRI.to_vec(), None)],
            )
        };
        let (mut bones, mut rotations) = (Puppet::new(made(true)), Puppet::new(made(false)));
        for v in POSES {
            pose(&mut bones, v);
            pose(&mut rotations, v);
            assert!(same_bits(&drawn(&bones, 0), &drawn(&rotations, 0)), "{v:?}");
        }
    }

    /// Bones A (root) and B (under A); mesh 0 at the root weighted to
    /// them, meshes 1 and 2 the same points as rigid children of B and of
    /// A, placed so that at rest all three sit in the same spot.
    fn weighted(weights: Skin) -> Model {
        let (a, b) = ((0.2, 0.1), (0.0, 1.0));
        let under_a: Vec<f32> = TRI.chunks(2).flat_map(|p| [p[0] - a.0, p[1] - a.1]).collect();
        let under_b: Vec<f32> =
            TRI.chunks(2).flat_map(|p| [p[0] - a.0 - b.0, p[1] - a.1 - b.1]).collect();
        model(
            vec![turner(true, None, 0, a, 30.0), turner(true, Some(0), 1, b, 45.0)],
            vec![
                mesh(None, TRI.to_vec(), Some(weights)),
                mesh(Some(1), under_b, None),
                mesh(Some(0), under_a, None),
            ],
        )
    }

    #[test]
    fn a_mesh_weighted_to_one_bone_moves_rigidly_with_it() {
        let mut p = Puppet::new(weighted(skin(&[&[(1, 1.0)], &[(1, 1.0)], &[(1, 1.0)]])));
        for v in POSES {
            pose(&mut p, v);
            assert!(close(&drawn(&p, 0), &drawn(&p, 1)), "{v:?}");
        }
    }

    #[test]
    fn half_weights_land_halfway_between_the_bones() {
        let mut p = Puppet::new(weighted(skin(&[&[(1, 1.0)], &[(0, 0.5), (1, 0.5)], &[(0, 1.0)]])));
        for v in POSES {
            pose(&mut p, v);
            let (m, b, a) = (drawn(&p, 0), drawn(&p, 1), drawn(&p, 2));
            let want = [b[0], b[1], (a[2] + b[2]) * 0.5, (a[3] + b[3]) * 0.5, a[4], a[5]];
            assert!(close(&m, &want), "{v:?}: {m:?} vs {want:?}");
        }
    }

    #[test]
    fn a_weighted_mesh_at_rest_is_where_its_keyforms_put_it() {
        let mut p =
            Puppet::new(weighted(skin(&[&[(1, 1.0)], &[(0, 0.25), (1, 0.75)], &[(0, 1.0)]])));
        p.update();
        assert!(close(&drawn(&p, 0), &TRI));
        // and back again after a pose
        pose(&mut p, [0.8, -0.4, 0.0]);
        assert!(!close(&drawn(&p, 0), &TRI));
        pose(&mut p, [0.0, 0.0, 0.0]);
        assert!(close(&drawn(&p, 0), &TRI));
    }

    #[test]
    fn a_skinned_mesh_redoes_when_only_its_bone_moves() {
        let mut p = Puppet::new(weighted(skin(&[&[(1, 1.0)], &[(0, 0.5), (1, 0.5)], &[(0, 1.0)]])));
        pose(&mut p, [0.3, 0.0, 0.0]);
        // only the child bone turns: the mesh's own weights are the same
        pose(&mut p, [0.3, 0.9, 0.0]);
        let cached = drawn(&p, 0);
        p.invalidate_cache();
        p.update();
        assert_eq!(cached, drawn(&p, 0));
    }

    #[test]
    fn weights_that_do_not_check_out_leave_the_mesh_unskinned() {
        // a rotation deformer is not a bone; a short weight list; a bone
        // outside the mesh's parent
        for (weights, parent) in [
            (skin(&[&[(0, 1.0)], &[(0, 1.0)], &[(0, 1.0)]]), None),
            (Skin { bones: vec![1; 4], weights: vec![1.0; 4] }, None),
            (skin(&[&[(1, 1.0)], &[(1, 1.0)], &[(1, 1.0)]]), Some(2)),
        ] {
            let m = model(
                vec![
                    turner(false, None, 0, (0.0, 0.0), 30.0),
                    turner(true, None, 1, (0.0, 0.0), 30.0),
                    turner(false, None, 2, (0.0, 0.0), 0.0),
                ],
                vec![mesh(parent, TRI.to_vec(), Some(weights))],
            );
            let mut plain = m.clone();
            plain.art_meshes[0].skin = None;
            let (mut p, mut q) = (Puppet::new(m), Puppet::new(plain));
            pose(&mut p, [0.5, 0.5, 0.5]);
            pose(&mut q, [0.5, 0.5, 0.5]);
            assert_eq!(drawn(&p, 0), drawn(&q, 0));
        }
    }

    /// Set a bone's y scale and shears in every keyform.
    fn shape_bone(d: &mut Deformer, scale_y: f32, shear_x: f32, shear_y: f32) {
        let DeformerKind::Bone(b) = &mut d.kind else { panic!("bone") };
        let n = b.rotation.scales.len();
        b.scales_y = vec![scale_y; n];
        b.shears_x = vec![shear_x; n];
        b.shears_y = vec![shear_y; n];
    }

    #[test]
    fn a_bone_scales_each_axis_and_shears() {
        let mut bone = turner(true, None, 0, (1.0, 2.0), 90.0);
        shape_bone(&mut bone, 0.5, 0.0, 0.0);
        let mut p = Puppet::new(model(vec![bone.clone()], vec![mesh(Some(0), TRI.to_vec(), None)]));
        p.update();
        // at rest: x as is, y halved, then moved to the origin
        let want: Vec<f32> = TRI.chunks(2).flat_map(|v| [1.0 + v[0], 2.0 + 0.5 * v[1]]).collect();
        assert!(close(&drawn(&p, 0), &want));

        // a 90 degree shear of x leaves the x axis along y
        shape_bone(&mut bone, 1.0, 90.0, 0.0);
        let mut p = Puppet::new(model(vec![bone], vec![mesh(Some(0), TRI.to_vec(), None)]));
        p.update();
        let want: Vec<f32> = TRI.chunks(2).flat_map(|v| [1.0, 2.0 + v[0] + v[1]]).collect();
        assert!(close(&drawn(&p, 0), &want), "{:?}", drawn(&p, 0));
    }

    #[test]
    fn a_bone_under_a_sheared_bone_takes_its_whole_map() {
        let mut parent = turner(true, None, 0, (0.3, -0.2), 30.0);
        shape_bone(&mut parent, 0.6, 15.0, -10.0);
        let child = turner(true, Some(0), 1, (0.5, 1.0), 45.0);
        let m = model(
            vec![parent, child],
            vec![mesh(Some(1), TRI.to_vec(), None), mesh(Some(0), TRI.to_vec(), None)],
        );
        let mut p = Puppet::new(m);
        for v in POSES {
            pose(&mut p, v);
            // the child's own map at this pose, then the parent's
            let turn = 45.0 * v[1];
            let (s, c) = turn.to_radians().sin_cos();
            let local: Vec<f32> = TRI
                .chunks(2)
                .flat_map(|q| [0.5 + c * q[0] - s * q[1], 1.0 + s * q[0] + c * q[1]])
                .collect();
            // mesh 1 is TRI through the parent: read the parent's map off it
            let m = fit(&TRI, &drawn(&p, 1));
            let want: Vec<f32> = local
                .chunks(2)
                .flat_map(|q| [m[0] * q[0] + m[1] * q[1] + m[4], m[2] * q[0] + m[3] * q[1] + m[5]])
                .collect();
            assert!(close(&drawn(&p, 0), &want), "{v:?}");
        }
    }

    /// The affine map taking three points `from` to `to`: m00, m01, m10,
    /// m11, tx, ty.
    fn fit(from: &[f32], to: &[f32]) -> [f32; 6] {
        let (x0, y0, x1, y1, x2, y2) = (from[0], from[1], from[2], from[3], from[4], from[5]);
        let (ax, ay, bx, by) = (x1 - x0, y1 - y0, x2 - x0, y2 - y0);
        let det = ax * by - ay * bx;
        let row = |u0: f32, u1: f32, u2: f32| {
            let (du, dv) = (u1 - u0, u2 - u0);
            let m0 = (du * by - dv * ay) / det;
            let m1 = (dv * ax - du * bx) / det;
            (m0, m1, u0 - m0 * x0 - m1 * y0)
        };
        let (m00, m01, tx) = row(to[0], to[2], to[4]);
        let (m10, m11, ty) = row(to[1], to[3], to[5]);
        [m00, m01, m10, m11, tx, ty]
    }

    #[test]
    fn a_plain_bone_still_matches_a_rotation_with_empty_or_even_pools() {
        // pools saying scale y = scale and no shear are a rotation deformer
        let made = |bone: bool| {
            let mut d = vec![
                turner(bone, None, 0, (0.2, 0.1), 30.0),
                turner(bone, Some(0), 1, (0.0, 1.0), 45.0),
            ];
            if bone {
                shape_bone(&mut d[1], 1.0, 0.0, 0.0);
            }
            model(d, vec![mesh(Some(1), TRI.to_vec(), None)])
        };
        let (mut bones, mut rotations) = (Puppet::new(made(true)), Puppet::new(made(false)));
        for v in POSES {
            pose(&mut bones, v);
            pose(&mut rotations, v);
            assert!(same_bits(&drawn(&bones, 0), &drawn(&rotations, 0)), "{v:?}");
        }
    }

    #[test]
    fn adding_a_key_keeps_a_bones_own_pools_in_step() {
        use crate::edit::{self, Target};
        let mut bone = turner(true, None, 0, (0.0, 0.0), 30.0);
        shape_bone(&mut bone, 0.5, 10.0, -10.0);
        let mut m = model(vec![bone], vec![mesh(Some(0), TRI.to_vec(), None)]);
        assert!(edit::add_keys(&mut m, Target::Deformer(0), 2, &[-1.0, 1.0]));
        let DeformerKind::Bone(b) = &m.deformers[0].kind else { panic!("bone") };
        let n = b.rotation.angles.len();
        assert_eq!(n, 6);
        assert_eq!((b.scales_y.len(), b.shears_x.len(), b.shears_y.len()), (n, n, n));
    }
}
