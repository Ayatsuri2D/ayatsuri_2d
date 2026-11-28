//! Breath for an idle model: a few parameters swing on slow sine waves, as
//! the editor's Breath and the Cubism SDK's CubismBreath do, so a model
//! standing still still moves and physics has something to react to. Each
//! frame a row adds `(offset + peak * sin(2 * pi * t / cycle)) * weight`
//! to its parameter; the next frame takes that back before adding again,
//! so the value underneath (a script's, a motion's) stays where it was.
//!
//! The clock and the rows live here; what a parameter is called, which
//! index it has and where it is clamped is the node's business, so the
//! table is plain data and `step` hands back ids and deltas.

/// One parameter's breathing.
#[derive(Debug, Clone, PartialEq)]
pub struct BreathRow {
    pub param: &'static str,
    pub offset: f32,
    pub peak: f32,
    /// Seconds per swing.
    pub cycle: f32,
    pub weight: f32,
}

/// The rows the editor's Breath uses by default: the standard head, body
/// and breath parameters, with the periods and amplitudes CubismBreath is
/// set up with in the Cubism SDK.
pub fn sdk_rows() -> Vec<BreathRow> {
    [
        ("ParamAngleX", 0.0, 15.0, 6.5345, 0.5),
        ("ParamAngleY", 0.0, 8.0, 3.5345, 0.5),
        ("ParamAngleZ", 0.0, 10.0, 5.5345, 0.5),
        ("ParamBodyAngleX", 0.0, 4.0, 15.5345, 0.5),
        ("ParamBreath", 0.5, 0.5, 3.2345, 1.0),
    ]
    .into_iter()
    .map(|(param, offset, peak, cycle, weight)| BreathRow { param, offset, peak, cycle, weight })
    .collect()
}

/// The running breath: its rows and its clock.
#[derive(Debug, Clone)]
pub struct Breath {
    rows: Vec<BreathRow>,
    time: f32,
}

impl Default for Breath {
    fn default() -> Self {
        Self::with_rows(sdk_rows())
    }
}

impl Breath {
    pub fn with_rows(rows: Vec<BreathRow>) -> Self {
        Self { rows, time: 0.0 }
    }

    /// The rows, in the order `step` reports them.
    pub fn rows(&self) -> &[BreathRow] {
        &self.rows
    }

    /// Advance the clock by `dt` seconds (never backwards) and give what
    /// each row adds to its parameter this frame, one entry per row in row
    /// order.
    pub fn step(&mut self, dt: f32) -> Vec<(&'static str, f32)> {
        self.time += dt.max(0.0);
        let tau = std::f32::consts::TAU;
        self.rows
            .iter()
            .map(|r| {
                let swing = if r.cycle > 0.0 { (tau * self.time / r.cycle).sin() } else { 0.0 };
                (r.param, (r.offset + r.peak * swing) * r.weight)
            })
            .collect()
    }

    /// Clock back to zero, so the next step starts a swing from rest.
    pub fn reset(&mut self) {
        self.time = 0.0;
    }
}

/// The value to set a breathing parameter to this frame, and what breath
/// then holds of it. `current` is the value the runtime last reported (or
/// what a script set this frame), `applied_before` what breath added to it
/// last frame (zero when the script set it fresh), `delta` this frame's
/// swing. The result is clamped to `min..=max`; the second number is the
/// amount that landed after the clamp, so it can be taken back exactly.
pub fn breathe(current: f32, applied_before: f32, delta: f32, min: f32, max: f32) -> (f32, f32) {
    let (lo, hi) = (min.min(max), max.max(min));
    let base = current - applied_before;
    let after = (base + delta).clamp(lo, hi);
    (after, after - base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn at_time_zero_only_the_offset_is_added() {
        let mut b = Breath::default();
        let out = b.step(0.0);
        assert_eq!(out.len(), 5);
        for ((id, delta), row) in out.iter().zip(sdk_rows()) {
            assert_eq!(*id, row.param);
            assert!(close(*delta, row.offset * row.weight), "{id}: {delta}");
        }
        // every head row rests at zero, breath at its offset
        assert!(close(out.iter().find(|(id, _)| *id == "ParamBreath").map_or(-1.0, |e| e.1), 0.5));
        assert!(close(out.iter().find(|(id, _)| *id == "ParamAngleX").map_or(-1.0, |e| e.1), 0.0));
    }

    #[test]
    fn a_quarter_cycle_later_the_peak_is_added() {
        let row =
            BreathRow { param: "ParamBreath", offset: 0.5, peak: 0.5, cycle: 4.0, weight: 1.0 };
        let mut b = Breath::with_rows(vec![row]);
        let out = b.step(1.0);
        assert_eq!(out.len(), 1);
        assert!(close(out.first().map_or(-1.0, |e| e.1), 1.0));
        // three quarters: offset minus peak
        let out = b.step(2.0);
        assert!(close(out.first().map_or(-1.0, |e| e.1), 0.0));
        // the weight scales the whole swing
        let mut b = Breath::with_rows(vec![BreathRow {
            param: "ParamAngleX",
            offset: 0.0,
            peak: 15.0,
            cycle: 4.0,
            weight: 0.5,
        }]);
        assert!(close(b.step(1.0).first().map_or(-1.0, |e| e.1), 7.5));
    }

    #[test]
    fn the_row_order_is_stable() {
        let mut b = Breath::default();
        let first: Vec<&str> = b.step(0.3).into_iter().map(|(id, _)| id).collect();
        let again: Vec<&str> = b.step(0.7).into_iter().map(|(id, _)| id).collect();
        assert_eq!(first, again);
        assert_eq!(
            first,
            ["ParamAngleX", "ParamAngleY", "ParamAngleZ", "ParamBodyAngleX", "ParamBreath"]
        );
    }

    #[test]
    fn the_clock_never_runs_backwards_and_resets() {
        let row = BreathRow { param: "P", offset: 0.0, peak: 1.0, cycle: 4.0, weight: 1.0 };
        let mut b = Breath::with_rows(vec![row.clone()]);
        b.step(1.0);
        let at_one = b.step(-5.0).first().map_or(-1.0, |e| e.1);
        assert!(close(at_one, 1.0), "{at_one}");
        b.reset();
        assert!(close(b.step(0.0).first().map_or(-1.0, |e| e.1), 0.0));
        // a zero cycle swings nothing
        let mut b = Breath::with_rows(vec![BreathRow { cycle: 0.0, offset: 0.25, ..row }]);
        assert!(close(b.step(1.0).first().map_or(-1.0, |e| e.1), 0.25));
    }

    #[test]
    fn breathe_takes_last_frame_back_and_clamps() {
        // a resting value of 3 with nothing applied yet
        let (value, applied) = breathe(3.0, 0.0, 2.0, -30.0, 30.0);
        assert!(close(value, 5.0) && close(applied, 2.0));
        // next frame the runtime reports 5; the 2 comes off before the new swing
        let (value, applied) = breathe(5.0, 2.0, -1.0, -30.0, 30.0);
        assert!(close(value, 2.0) && close(applied, -1.0));
        // clamped at the top: only what landed is remembered
        let (value, applied) = breathe(0.9, 0.0, 0.5, 0.0, 1.0);
        assert!(close(value, 1.0) && close(applied, 0.1));
        // a range given backwards still clamps
        let (value, _) = breathe(0.0, 0.0, 5.0, 1.0, -1.0);
        assert!(close(value, 1.0));
    }
}
