// Sphere warp: write a warp deformer's keyforms on two parameters so the
// grid behaves like the visible face of a sphere that turns with them. Each
// grid point at rest is read as a point (u, v) in -1..1 across the grid,
// lifted onto the hemisphere z = sqrt(1 - u^2 - v^2) scaled by a depth,
// turned by the yaw and pitch the key's parameter values stand for, and
// projected straight back onto the plane (z dropped). The result is plain
// warp keyforms, so it exports to every moc3 version and the rigger can
// hand-edit any key afterwards. Eyes, a ball and a chest turning with the
// head are the uses this was asked for.

use ayatsuri_format::model::{DeformerKind, Model};

use crate::edit::{Target, add_keys, axis_lens};

/// What `sphere_warp` writes.
#[derive(Clone, Debug, PartialEq)]
pub struct SphereParams {
    /// The parameter that turns the sphere left and right: its maximum
    /// turns the face toward +x.
    pub param_x: usize,
    /// The parameter that turns the sphere down and up: its maximum turns
    /// the face up, toward -y as the screen draws it.
    pub param_y: usize,
    /// The keys to write on each parameter. The parameter's default is
    /// always added, since that key is the front. Empty means the
    /// parameter's minimum, default and maximum.
    pub keys_x: Vec<f32>,
    pub keys_y: Vec<f32>,
    /// Half the sphere's width and height in the warp's parent space. Zero
    /// or less takes the rest grid's own half extent, so the grid's edge
    /// is the sphere's rim.
    pub radius_x: f32,
    pub radius_y: f32,
    /// How far the grid is lifted: 1 is a full hemisphere, 0 stays flat and
    /// only foreshortens.
    pub depth: f32,
    /// The turn in degrees at the key farthest from the parameter's
    /// default, as Cubism's angle range reads it. A parameter running from
    /// -30 to 30 with its default at 0 turns from -max to +max.
    pub max_angle_deg: f32,
}

/// The turn, in degrees, a parameter value stands for: the default is the
/// front, and the end farther from the default reaches `max_angle_deg`.
pub fn turn_angle(min: f32, max: f32, default: f32, value: f32, max_angle_deg: f32) -> f32 {
    let far = (max - default).abs().max((default - min).abs());
    if far <= 1e-9 {
        return 0.0;
    }
    max_angle_deg * (value - default) / far
}

/// A keyform's index from its coordinate on each axis, first axis fastest.
fn index_of(lens: &[usize], coords: &[usize]) -> usize {
    let mut idx = 0usize;
    let mut stride = 1usize;
    for (i, &len) in lens.iter().enumerate() {
        idx += coords.get(i).copied().unwrap_or(0).min(len.saturating_sub(1)) * stride;
        stride *= len.max(1);
    }
    idx
}

/// The key nearest `v`, or 0 on an empty axis.
fn nearest_key(keys: &[f32], v: f32) -> usize {
    keys.iter()
        .enumerate()
        .min_by(|a, b| (a.1 - v).abs().total_cmp(&(b.1 - v).abs()))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// The keys to write on a parameter: the ones asked for plus the default.
fn keys_with_default(asked: &[f32], min: f32, max: f32, default: f32) -> Vec<f32> {
    let mut keys: Vec<f32> = if asked.is_empty() {
        vec![min, default, max]
    } else {
        asked.iter().copied().filter(|k| k.is_finite()).collect()
    };
    keys.push(default);
    keys.sort_by(f32::total_cmp);
    keys.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    keys
}

/// Write `deformer`'s keyforms on `p.param_x` and `p.param_y` as a turning
/// sphere. The keys are added to the warp's binding first, so the warp may
/// be unkeyed on them, or keyed already: every key on the two axes is
/// written, and nothing else changes. Keyforms where any other axis sits
/// off its default key are left as they are. The keyform at both defaults
/// equals the rest grid exactly. Returns false, with the model untouched,
/// when the deformer is not a warp, a parameter is missing, both are the
/// same, one drives blend shapes, or the rest grid has no extent.
pub fn sphere_warp(model: &mut Model, deformer: usize, p: SphereParams) -> bool {
    if p.param_x == p.param_y || !p.depth.is_finite() || !p.max_angle_deg.is_finite() {
        return false;
    }
    let Some(px) = model.parameters.get(p.param_x).cloned() else { return false };
    let Some(py) = model.parameters.get(p.param_y).cloned() else { return false };
    if px.blend_shape || py.blend_shape {
        return false;
    }
    let is_warp =
        model.deformers.get(deformer).is_some_and(|d| matches!(d.kind, DeformerKind::Warp(_)));
    if !is_warp {
        return false;
    }
    // add the keys first: a warp unkeyed on a parameter tiles its grid
    // across the new axis, so the rest grid read afterwards is unchanged
    let keys_x = keys_with_default(&p.keys_x, px.min, px.max, px.default);
    let keys_y = keys_with_default(&p.keys_y, py.min, py.max, py.default);
    let target = Target::Deformer(deformer);
    add_keys(model, target, p.param_x, &keys_x);
    add_keys(model, target, p.param_y, &keys_y);

    let Some(d) = model.deformers.get_mut(deformer) else { return false };
    let binding = d.binding.clone();
    let DeformerKind::Warp(w) = &mut d.kind else { return false };
    let stride = w.grid_points() * 2;
    let lens = axis_lens(&binding);
    let axis_pos = |param: usize| binding.axes.iter().position(|a| a.parameter as usize == param);
    let (Some(ax), Some(ay)) = (axis_pos(p.param_x), axis_pos(p.param_y)) else { return false };
    let defaults: Vec<usize> = binding
        .axes
        .iter()
        .map(|a| {
            let default = model.parameters.get(a.parameter as usize).map_or(0.0, |q| q.default);
            nearest_key(&a.keys, default)
        })
        .collect();
    let total = lens.iter().product::<usize>().max(1);
    if stride == 0 || w.positions.len() < total * stride {
        return false;
    }
    let rest_at = index_of(&lens, &defaults) * stride;
    let Some(rest) = w.positions.get(rest_at..rest_at + stride).map(<[f32]>::to_vec) else {
        return false;
    };
    let (rest_pts, _) = rest.as_chunks::<2>();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for q in rest_pts {
        x0 = x0.min(q[0]);
        y0 = y0.min(q[1]);
        x1 = x1.max(q[0]);
        y1 = y1.max(q[1]);
    }
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let rx = if p.radius_x > 0.0 { p.radius_x } else { (x1 - x0) * 0.5 };
    let ry = if p.radius_y > 0.0 { p.radius_y } else { (y1 - y0) * 0.5 };
    if !(rx > 1e-9 && ry > 1e-9) || !cx.is_finite() || !cy.is_finite() {
        return false;
    }
    let depth = p.depth.max(0.0);

    let (Some(axis_x), Some(axis_y)) = (binding.axes.get(ax), binding.axes.get(ay)) else {
        return false;
    };
    let mut coords = defaults.clone();
    for (ix, &kx) in axis_x.keys.iter().enumerate() {
        let yaw = turn_angle(px.min, px.max, px.default, kx, p.max_angle_deg).to_radians();
        for (iy, &ky) in axis_y.keys.iter().enumerate() {
            let pitch = turn_angle(py.min, py.max, py.default, ky, p.max_angle_deg).to_radians();
            if let Some(c) = coords.get_mut(ax) {
                *c = ix;
            }
            if let Some(c) = coords.get_mut(ay) {
                *c = iy;
            }
            let at = index_of(&lens, &coords) * stride;
            let Some(form) = w.positions.get_mut(at..at + stride) else { continue };
            if yaw == 0.0 && pitch == 0.0 {
                // the front is the rest grid, bit for bit
                form.copy_from_slice(&rest);
                continue;
            }
            let (sy, cyaw) = yaw.sin_cos();
            let (sp, cp) = pitch.sin_cos();
            let (out, _) = form.as_chunks_mut::<2>();
            for (dst, q) in out.iter_mut().zip(rest_pts) {
                let u = (q[0] - cx) / rx;
                // up is -y on screen, so the lift reads v upward
                let up = (cy - q[1]) / ry;
                let z = depth * (1.0 - u * u - up * up).max(0.0).sqrt();
                // yaw about the vertical axis, then pitch about the
                // horizontal one; z is dropped (orthographic)
                let x_turned = u * cyaw + z * sy;
                let z_turned = z * cyaw - u * sy;
                let up_turned = up * cp + z_turned * sp;
                dst[0] = cx + x_turned * rx;
                dst[1] = cy - up_turned * ry;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use ayatsuri_format::model::{
        Binding, BindingAxis, Deformer, DeformerKind, Model, Parameter, WarpData,
    };

    use super::*;

    const COLS: u32 = 4;
    const ROWS: u32 = 4;

    fn param(id: &str, min: f32, max: f32, default: f32) -> Parameter {
        Parameter {
            id: id.into(),
            min,
            max,
            default,
            repeat: false,
            decimals: 1,
            blend_shape: false,
        }
    }

    /// A 4x4 grid over [0,100]^2, shifted by `dx`.
    fn grid(dx: f32) -> Vec<f32> {
        let mut v = Vec::new();
        for r in 0..=ROWS {
            for c in 0..=COLS {
                v.push(c as f32 * 25.0 + dx);
                v.push(r as f32 * 25.0);
            }
        }
        v
    }

    fn warp(binding: Binding, positions: Vec<f32>, keyforms: usize) -> Deformer {
        Deformer {
            id: "W".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding,
            kind: DeformerKind::Warp(WarpData {
                rows: ROWS,
                cols: COLS,
                quad: true,
                opacities: vec![1.0; keyforms],
                positions,
            }),
            multiply: Vec::new(),
            screen: Vec::new(),
            blend_shapes: Vec::new(),
        }
    }

    /// Angle X, Angle Y and an unkeyed warp.
    fn model() -> Model {
        Model {
            parameters: vec![
                param("ParamAngleX", -30.0, 30.0, 0.0),
                param("ParamAngleY", -30.0, 30.0, 0.0),
            ],
            deformers: vec![warp(Binding::default(), grid(0.0), 1)],
            ..Default::default()
        }
    }

    /// The same, with a third parameter already keyed at 0 and 1: at 1 the
    /// grid sits 10 units to the right.
    fn model_with_third_axis() -> Model {
        let mut m = model();
        m.parameters.push(param("C", 0.0, 1.0, 0.0));
        let mut positions = grid(0.0);
        positions.extend(grid(10.0));
        m.deformers = vec![warp(
            Binding {
                axes: vec![BindingAxis { parameter: 2, keys: vec![0.0, 1.0] }],
                curves: vec![],
            },
            positions,
            2,
        )];
        m
    }

    fn params(max_angle_deg: f32, depth: f32) -> SphereParams {
        SphereParams {
            param_x: 0,
            param_y: 1,
            keys_x: vec![-30.0, 0.0, 30.0],
            keys_y: vec![-30.0, 0.0, 30.0],
            radius_x: 0.0,
            radius_y: 0.0,
            depth,
            max_angle_deg,
        }
    }

    /// The grid of the keyform at key index `ix` on X, `iy` on Y and `ic`
    /// on the third axis, when there is one.
    fn form(m: &Model, ix: usize, iy: usize, ic: usize) -> Vec<f32> {
        let d = m.deformers.first().expect("warp");
        let DeformerKind::Warp(w) = &d.kind else { panic!("warp") };
        let lens = axis_lens(&d.binding);
        let pos = |p: usize| d.binding.axes.iter().position(|a| a.parameter as usize == p);
        let mut coords = vec![0usize; lens.len()];
        for (p, i) in [(0, ix), (1, iy), (2, ic)] {
            if let Some(a) = pos(p) {
                coords[a] = i;
            }
        }
        let stride = w.grid_points() * 2;
        let at = index_of(&lens, &coords) * stride;
        w.positions[at..at + stride].to_vec()
    }

    fn x_at(form: &[f32], col: usize, row: usize) -> f32 {
        form[(row * (COLS as usize + 1) + col) * 2]
    }

    fn y_at(form: &[f32], col: usize, row: usize) -> f32 {
        form[(row * (COLS as usize + 1) + col) * 2 + 1]
    }

    #[test]
    fn the_default_keyform_is_the_rest_grid() {
        let mut m = model();
        assert!(sphere_warp(&mut m, 0, params(90.0, 1.0)));
        let d = &m.deformers[0];
        assert_eq!(d.binding.axes.len(), 2);
        assert_eq!(d.binding.axes[0].keys, vec![-30.0, 0.0, 30.0]);
        assert_eq!(d.binding.axes[1].keys, vec![-30.0, 0.0, 30.0]);
        assert_eq!(form(&m, 1, 1, 0), grid(0.0));
        let DeformerKind::Warp(w) = &d.kind else { panic!("warp") };
        assert_eq!(w.positions.len(), 9 * w.grid_points() * 2);
        assert_eq!(w.opacities.len(), 9);
    }

    #[test]
    fn plus_x_moves_the_centre_toward_plus_x_and_squeezes_the_leaving_side() {
        let mut m = model();
        assert!(sphere_warp(&mut m, 0, params(20.0, 1.0)));
        let f = form(&m, 2, 1, 0);
        let rest = grid(0.0);
        let mid = ROWS as usize / 2;
        // the centre column moves right
        assert!(x_at(&f, 2, mid) > x_at(&rest, 2, mid) + 5.0, "centre x {}", x_at(&f, 2, mid));
        // spacing on the right (leaving) side shrinks, on the left it grows
        let right = x_at(&f, 3, mid) - x_at(&f, 2, mid);
        let rim = x_at(&f, 4, mid) - x_at(&f, 3, mid);
        let left = x_at(&f, 1, mid) - x_at(&f, 0, mid);
        assert!(right < 25.0 && right > 0.0, "right spacing {right}");
        assert!(rim < 25.0 && rim > 0.0, "rim spacing {rim}");
        assert!(left > 25.0, "left spacing {left}");
        // a pure yaw leaves the middle row where it was vertically
        for c in 0..=COLS as usize {
            assert!((y_at(&f, c, mid) - 50.0).abs() < 1e-3);
        }
    }

    #[test]
    fn plus_y_moves_the_centre_up() {
        let mut m = model();
        assert!(sphere_warp(&mut m, 0, params(20.0, 1.0)));
        let f = form(&m, 1, 2, 0);
        let mid = COLS as usize / 2;
        // up is -y on screen
        assert!(y_at(&f, mid, 2) < 45.0, "centre y {}", y_at(&f, mid, 2));
        assert!((x_at(&f, mid, 2) - 50.0).abs() < 1e-3);
    }

    #[test]
    fn minus_x_mirrors_plus_x() {
        let mut m = model();
        assert!(sphere_warp(&mut m, 0, params(30.0, 0.8)));
        let plus = form(&m, 2, 1, 0);
        let minus = form(&m, 0, 1, 0);
        for r in 0..=ROWS as usize {
            for c in 0..=COLS as usize {
                let mc = COLS as usize - c;
                assert!((x_at(&minus, c, r) - (100.0 - x_at(&plus, mc, r))).abs() < 1e-3);
                assert!((y_at(&minus, c, r) - y_at(&plus, mc, r)).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn a_flat_sphere_only_foreshortens() {
        let mut m = model();
        assert!(sphere_warp(&mut m, 0, params(45.0, 0.0)));
        let f = form(&m, 2, 1, 0);
        let mid = ROWS as usize / 2;
        assert!((x_at(&f, 2, mid) - 50.0).abs() < 1e-3);
        assert!(x_at(&f, 4, mid) < 100.0 && x_at(&f, 0, mid) > 0.0);
    }

    #[test]
    fn a_third_axis_keeps_its_other_keyforms() {
        let mut m = model_with_third_axis();
        assert!(sphere_warp(&mut m, 0, params(30.0, 1.0)));
        let d = &m.deformers[0];
        assert_eq!(d.binding.axes.len(), 3);
        assert_eq!(d.binding.axes[0].keys, vec![0.0, 1.0], "the third axis keeps its keys");
        let shifted = grid(10.0);
        for ix in 0..3 {
            for iy in 0..3 {
                assert_eq!(form(&m, ix, iy, 1), shifted, "keyform ({ix}, {iy}) at C=1");
            }
        }
        assert_eq!(form(&m, 1, 1, 0), grid(0.0));
        assert_ne!(form(&m, 2, 1, 0), grid(0.0));
        assert_ne!(form(&m, 1, 0, 0), grid(0.0));
    }

    #[test]
    fn the_default_key_is_always_written() {
        let mut m = model();
        let mut p = params(30.0, 1.0);
        p.keys_x = vec![30.0];
        p.keys_y = vec![-30.0];
        assert!(sphere_warp(&mut m, 0, p));
        let d = &m.deformers[0];
        assert_eq!(d.binding.axes[0].keys, vec![0.0, 30.0]);
        assert_eq!(d.binding.axes[1].keys, vec![-30.0, 0.0]);
        assert_eq!(form(&m, 0, 1, 0), grid(0.0));
    }

    #[test]
    fn refuses_what_it_cannot_do() {
        let mut m = model();
        let before = m.deformers[0].binding.clone();
        let mut same = params(30.0, 1.0);
        same.param_y = 0;
        assert!(!sphere_warp(&mut m, 0, same));
        let mut missing = params(30.0, 1.0);
        missing.param_y = 7;
        assert!(!sphere_warp(&mut m, 0, missing));
        assert!(!sphere_warp(&mut m, 3, params(30.0, 1.0)));
        assert_eq!(m.deformers[0].binding.axes.len(), before.axes.len());
    }

    #[test]
    fn the_far_end_of_a_one_sided_parameter_reaches_the_max_angle() {
        assert!((turn_angle(0.0, 1.0, 0.0, 1.0, 40.0) - 40.0).abs() < 1e-6);
        assert!((turn_angle(0.0, 1.0, 0.0, 0.5, 40.0) - 20.0).abs() < 1e-6);
        assert!((turn_angle(-30.0, 30.0, 0.0, -30.0, 90.0) + 90.0).abs() < 1e-6);
        assert!((turn_angle(-10.0, 30.0, 0.0, -10.0, 90.0) + 30.0).abs() < 1e-6);
        assert_eq!(turn_angle(5.0, 5.0, 5.0, 5.0, 90.0), 0.0);
    }
}
