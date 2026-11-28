//! Follow: the model turns to face a point, as the editor's Follow does
//! with the cursor. A point in the node's pixels becomes a target in -1..1
//! on both axes (measured from the canvas's centre, in half canvas sizes),
//! the target is eased towards frame by frame, and the eased position is
//! written to the standard parameter set every tracker expects: head and
//! body angles and the eyeballs.
//!
//! ```text
//! canvas (pixels)            target            parameters
//! +-------------+        x = -1 .. 1        ParamAngleX     = x * 30
//! |      .      |  --->  y = -1 .. 1  --->  ParamAngleY     = -y * 30
//! |   centre    |        (eased)            ParamBodyAngleX = x * 10
//! +-------------+                           ...
//! ```

/// The `follow` property's values.
pub const FOLLOW_OFF: i32 = 0;
pub const FOLLOW_MOUSE: i32 = 1;
pub const FOLLOW_TARGET: i32 = 2;

/// The editor's easing rate: the eased target closes `1 - exp(-dt * 8)`
/// of the gap each frame.
pub const DEFAULT_EASE: f32 = 8.0;

/// The six parameters follow writes, with how much of the eased x and y
/// each takes (at strength 1). The model's y points down, so an upward
/// look is a negative y.
pub const OUTPUTS: [(&str, f32, f32); 6] = [
    ("ParamAngleX", 30.0, 0.0),
    ("ParamAngleY", 0.0, -30.0),
    ("ParamBodyAngleX", 10.0, 0.0),
    ("ParamBodyAngleY", 0.0, -10.0),
    ("ParamEyeBallX", 1.0, 0.0),
    ("ParamEyeBallY", 0.0, -1.0),
];

/// A point in the node's pixels as a target in -1..1 on both axes, from
/// the canvas's centre, clamped at the canvas's edges. `rect` is the
/// canvas as `canvas_rect` gives it: left, top, width, height. A canvas
/// without size (not loaded, or degenerate) puts every point at the
/// centre.
pub fn normalise(point: (f32, f32), rect: (f32, f32, f32, f32)) -> (f32, f32) {
    let (left, top, width, height) = rect;
    let axis = |p: f32, edge: f32, size: f32| {
        let half = size * 0.5;
        if half > 0.0 { ((p - (edge + half)) / half).clamp(-1.0, 1.0) } else { 0.0 }
    };
    let x = axis(point.0, left, width);
    let y = axis(point.1, top, height);
    (if x.is_nan() { 0.0 } else { x }, if y.is_nan() { 0.0 } else { y })
}

/// The eased position, moved towards `target` as the editor moves its
/// Follow: by `1 - exp(-dt * rate)` of the gap, and the rest of the way
/// once the gap is below a thousandth. A zero or negative `dt` moves
/// nothing; a zero or negative `rate` snaps.
pub fn ease(current: (f32, f32), target: (f32, f32), dt: f32, rate: f32) -> (f32, f32) {
    let (dx, dy) = (target.0 - current.0, target.1 - current.1);
    if (dx * dx + dy * dy).sqrt() <= 1e-3 {
        return target;
    }
    if rate <= 0.0 {
        return target;
    }
    if dt <= 0.0 {
        return current;
    }
    let k = 1.0 - (-dt * rate).exp();
    (current.0 + dx * k, current.1 + dy * k)
}

/// What the eased position puts on each of `OUTPUTS`, scaled by
/// `strength`.
pub fn outputs(eased: (f32, f32), strength: f32) -> [(&'static str, f32); 6] {
    let mut out = [("", 0.0); 6];
    for (slot, (id, fx, fy)) in out.iter_mut().zip(OUTPUTS) {
        *slot = (id, (eased.0 * fx + eased.1 * fy) * strength);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// An 800 by 600 canvas with its origin in the middle, as the node
    /// reports it: the corner at minus the origin.
    const RECT: (f32, f32, f32, f32) = (-400.0, -300.0, 800.0, 600.0);

    #[test]
    fn a_point_is_measured_from_the_canvas_centre_in_half_sizes() {
        assert_eq!(normalise((0.0, 0.0), RECT), (0.0, 0.0));
        assert_eq!(normalise((400.0, -300.0), RECT), (1.0, -1.0));
        assert_eq!(normalise((-200.0, 150.0), RECT), (-0.5, 0.5));
        // past the edge is clamped
        assert_eq!(normalise((4000.0, -9000.0), RECT), (1.0, -1.0));
        // an origin off centre: the centre of the canvas still maps to zero
        let rect = (0.0, 0.0, 800.0, 600.0);
        assert_eq!(normalise((400.0, 300.0), rect), (0.0, 0.0));
        assert_eq!(normalise((800.0, 600.0), rect), (1.0, 1.0));
        // no canvas: everything is the centre
        assert_eq!(normalise((123.0, 45.0), (0.0, 0.0, 0.0, 0.0)), (0.0, 0.0));
        assert_eq!(normalise((f32::NAN, 0.0), RECT), (0.0, 0.0));
    }

    #[test]
    fn easing_closes_the_editors_share_of_the_gap() {
        let (x, y) = ease((0.0, 0.0), (1.0, -1.0), 0.1, DEFAULT_EASE);
        let k = 1.0 - (-0.8f32).exp();
        assert!(close(x, k) && close(y, -k), "{x} {y}");
        // a tiny gap snaps shut
        assert_eq!(ease((0.9995, 0.0), (1.0, 0.0), 0.016, DEFAULT_EASE), (1.0, 0.0));
        // no time, no movement; no rate, a snap
        assert_eq!(ease((0.0, 0.0), (1.0, 1.0), 0.0, DEFAULT_EASE), (0.0, 0.0));
        assert_eq!(ease((0.0, 0.0), (1.0, 1.0), 0.016, 0.0), (1.0, 1.0));
        // many frames later it has arrived
        let mut p = (0.0, 0.0);
        for _ in 0..600 {
            p = ease(p, (-1.0, 0.5), 1.0 / 60.0, DEFAULT_EASE);
        }
        assert_eq!(p, (-1.0, 0.5));
    }

    #[test]
    fn the_six_standard_parameters_take_their_factors() {
        let out = outputs((0.5, -0.25), 1.0);
        let get = |id: &str| out.iter().find(|(i, _)| *i == id).map_or(f32::NAN, |e| e.1);
        assert!(close(get("ParamAngleX"), 15.0));
        assert!(close(get("ParamAngleY"), 7.5));
        assert!(close(get("ParamBodyAngleX"), 5.0));
        assert!(close(get("ParamBodyAngleY"), 2.5));
        assert!(close(get("ParamEyeBallX"), 0.5));
        assert!(close(get("ParamEyeBallY"), 0.25));
        // strength scales every output; the order is the table's
        let half = outputs((0.5, -0.25), 0.5);
        for ((id, v), (id2, v2)) in out.iter().zip(half) {
            assert_eq!(*id, id2);
            assert!(close(*v * 0.5, v2));
        }
        assert_eq!(out.first().map(|e| e.0), Some("ParamAngleX"));
        assert_eq!(out.last().map(|e| e.0), Some("ParamEyeBallY"));
    }
}
