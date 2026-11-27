// Modeling > Parameter > Random Pose: parameters wander on their own so a
// rig can be checked without dragging every slider. A pattern picks which
// parameters move and how (Cubism's Pattern A, Pattern B and Posing), the
// list ticks the parameters that take part, and Randomize throws one pose
// at a time. Only parameter values change: keyforms stay as they are, so
// nothing here goes through undo, and Reset puts every value back on its
// default like Ctrl+1.

use crate::panels::EditorUi;
use egui::Ui;

/// Which parameters move and how.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Pattern {
    /// A fixed set of the standard parameters (head angles, eyes, body,
    /// mouth) swing back and forth, each on its own period.
    A,
    /// Every ticked parameter jumps between random values; the eyes swing.
    #[default]
    B,
    /// Every ticked parameter jumps between random values, while the eyes,
    /// mouth and breath swing so the model reads as a natural pose.
    Posing,
}

impl Pattern {
    pub(crate) fn from_index(i: u8) -> Self {
        match i {
            0 => Self::A,
            2 => Self::Posing,
            _ => Self::B,
        }
    }

    pub(crate) fn index(self) -> u8 {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::Posing => 2,
        }
    }
}

/// One parameter on its way from one random value to the next.
struct Wander {
    param: usize,
    from: f32,
    to: f32,
    /// Seconds spent on this leg and how long it lasts.
    at: f32,
    len: f32,
}

#[derive(Default)]
pub(crate) struct RandomPose {
    pub(crate) show: bool,
    pub(crate) playing: bool,
    /// xorshift64* state, seeded from the clock when playback starts.
    rng: u64,
    /// Seconds since playback started, the clock the swings run on.
    time: f32,
    wanders: Vec<Wander>,
    /// Parameter folders folded shut in the list.
    folded: Vec<String>,
}

impl RandomPose {
    fn seed(&mut self) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64);
        self.rng = nanos | 1;
        // a few throws so a seed with few bits set spreads out
        for _ in 0..4 {
            self.next();
        }
    }

    /// The next number in 0..1.
    fn next(&mut self) -> f32 {
        if self.rng == 0 {
            self.seed();
        }
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let bits = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        bits as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// Swings of the standard parameters: the span each pattern moves them
/// over, cut down to the parameter's own range, and the period in seconds.
fn swing(pattern: Pattern, id: &str) -> Option<(f32, f32, f32)> {
    let eyes_mouth_breath = match id {
        "ParamEyeLOpen" | "ParamEyeROpen" => Some((0.0, 2.0, 2.6)),
        "ParamMouthOpenY" if pattern != Pattern::B => Some((-1.0, 1.0, 1.3)),
        "ParamBreath" if pattern == Pattern::Posing => Some((0.0, 1.0, 3.2)),
        _ => None,
    };
    if pattern != Pattern::A {
        return eyes_mouth_breath;
    }
    match id {
        "ParamAngleX" => Some((-30.0, 30.0, 3.1)),
        "ParamAngleZ" => Some((-30.0, 30.0, 4.3)),
        "ParamBodyAngleX" => Some((-10.0, 10.0, 5.2)),
        _ => eyes_mouth_breath,
    }
}

impl EditorUi {
    /// Whether a parameter is ticked in the Random Pose list: the saved
    /// choice when there is one, else on, unless physics writes it.
    pub(crate) fn random_pose_takes_part(&self, param: usize) -> bool {
        let Some(s) = &self.session else { return false };
        let model = s.puppet.model();
        let Some(p) = model.parameters.get(param) else { return false };
        if self.prefs.random_pose_on.contains(&p.id) {
            return true;
        }
        if self.prefs.random_pose_off.contains(&p.id) {
            return false;
        }
        let physics_output = model.physics.as_ref().is_some_and(|rig| {
            rig.groups.iter().any(|g| g.outputs.iter().any(|o| o.param == p.id))
        });
        !physics_output
    }

    /// Tick or untick a parameter, remembered by id across models.
    fn random_pose_set(&mut self, param: usize, on: bool) {
        let Some(id) = self
            .session
            .as_ref()
            .and_then(|s| s.puppet.model().parameters.get(param))
            .map(|p| p.id.clone())
        else {
            return;
        };
        self.prefs.random_pose_on.retain(|i| *i != id);
        self.prefs.random_pose_off.retain(|i| *i != id);
        let default_on = self.random_pose_takes_part(param);
        if on != default_on {
            if on {
                self.prefs.random_pose_on.push(id);
            } else {
                self.prefs.random_pose_off.push(id);
            }
        }
    }

    /// The parameters the current pattern moves.
    fn random_pose_members(&self) -> Vec<usize> {
        let Some(s) = &self.session else { return Vec::new() };
        let pattern = Pattern::from_index(self.prefs.random_pose_pattern);
        let params = &s.puppet.model().parameters;
        (0..params.len())
            .filter(|&i| {
                if pattern == Pattern::A {
                    params.get(i).is_some_and(|p| swing(pattern, &p.id).is_some())
                } else {
                    self.random_pose_takes_part(i)
                }
            })
            .collect()
    }

    /// The Random Pose entry of the Parameter menu: open the dialog.
    pub(crate) fn open_random_pose(&mut self) {
        if self.session.is_some() {
            self.random_pose.show = true;
        }
    }

    /// Start or stop the motion; stopping leaves the pose where it is.
    pub(crate) fn random_pose_toggle(&mut self) {
        if self.session.is_none() {
            return;
        }
        self.random_pose.playing = !self.random_pose.playing;
        if self.random_pose.playing {
            self.random_pose.seed();
            self.random_pose.time = 0.0;
            self.random_pose.wanders.clear();
            log::info!("random pose: started");
        } else {
            log::info!("random pose: stopped");
        }
    }

    /// Randomize: one random value for every moving parameter, at once.
    pub(crate) fn random_pose_once(&mut self) {
        let members = self.random_pose_members();
        let Some(s) = &mut self.session else { return };
        for i in members {
            let Some((min, max)) = s.puppet.model().parameters.get(i).map(|p| (p.min, p.max))
            else {
                continue;
            };
            let v = self.random_pose.range(min, max);
            s.puppet.set_value(i, v);
        }
        s.dirty = true;
        self.random_pose.wanders.clear();
        log::info!("random pose: one pose thrown");
    }

    /// Called once a frame: moves the parameters while playback is on.
    pub(crate) fn random_pose_step(&mut self, ctx: &egui::Context) {
        if !self.random_pose.playing {
            return;
        }
        if self.session.is_none() {
            self.random_pose.playing = false;
            return;
        }
        let dt = self.frame_dt.clamp(0.0, 0.1);
        let pattern = Pattern::from_index(self.prefs.random_pose_pattern);
        let members = self.random_pose_members();
        self.random_pose.time += dt;
        let time = self.random_pose.time;
        // the swinging ones follow the clock
        let mut swung = Vec::new();
        if let Some(s) = &mut self.session {
            for &i in &members {
                let Some(p) = s.puppet.model().parameters.get(i) else { continue };
                let Some((lo, hi, period)) = swing(pattern, &p.id) else { continue };
                let lo = lo.max(p.min);
                let hi = hi.min(p.max);
                if hi <= lo {
                    continue;
                }
                let phase = (time / period.max(0.1)) * std::f32::consts::TAU;
                let v = lo + (hi - lo) * (0.5 - 0.5 * phase.cos());
                s.puppet.set_value(i, v);
                swung.push(i);
            }
        }
        // the rest wander: each leg goes to a new random value and eases in
        self.random_pose
            .wanders
            .retain(|w| members.contains(&w.param) && !swung.contains(&w.param));
        let have: Vec<usize> = self.random_pose.wanders.iter().map(|w| w.param).collect();
        let mut fresh = Vec::new();
        for &i in &members {
            if swung.contains(&i) || have.contains(&i) {
                continue;
            }
            let Some((min, max, now)) = self.session.as_ref().and_then(|s| {
                s.puppet.model().parameters.get(i).map(|p| (p.min, p.max, s.puppet.value(i)))
            }) else {
                continue;
            };
            let to = self.random_pose.range(min, max);
            let len = self.random_pose.range(0.5, 1.6);
            fresh.push(Wander { param: i, from: now, to, at: 0.0, len });
        }
        self.random_pose.wanders.extend(fresh);
        let mut done = Vec::new();
        if let Some(s) = &mut self.session {
            for w in &mut self.random_pose.wanders {
                w.at += dt;
                let f = (w.at / w.len.max(0.01)).clamp(0.0, 1.0);
                let eased = f * f * (3.0 - 2.0 * f);
                s.puppet.set_value(w.param, w.from + (w.to - w.from) * eased);
                if f >= 1.0 {
                    done.push(w.param);
                }
            }
            s.dirty = true;
        }
        // finished legs start over from where they ended, next frame
        self.random_pose.wanders.retain(|w| !done.contains(&w.param));
        ctx.request_repaint();
    }

    pub(crate) fn random_pose_window(&mut self, ui: &mut Ui) {
        use crate::i18n::tr;
        if !self.random_pose.show {
            return;
        }
        let mut open = self.random_pose.show;
        let mut save_prefs = false;
        crate::windows::Dialog::new(tr("Random Pose")).open(&mut open).show(ui.ctx(), |ui| {
            if self.session.is_none() {
                ui.weak(tr("No model loaded."));
                return;
            }
            ui.set_width(300.0);
            ui.weak(tr("Parameters move on their own so the rig can be checked. Values only: no keyform changes, nothing to undo."));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = if self.random_pose.playing { tr("Stop") } else { tr("Start") };
                if ui.button(egui::RichText::new(label).strong()).clicked() {
                    self.random_pose_toggle();
                }
                if ui.button(tr("Randomize")).on_hover_text(tr("One random pose, now")).clicked() {
                    self.random_pose_once();
                }
                if ui
                    .button(tr("Reset"))
                    .on_hover_text(tr("Every parameter back on its default (Ctrl+1)"))
                    .clicked()
                {
                    self.random_pose.playing = false;
                    self.reset_parameters();
                }
            });
            ui.add_space(6.0);
            let mut pattern = Pattern::from_index(self.prefs.random_pose_pattern);
            ui.label(tr("Pattern"));
            for (p, label, hint) in [
                (Pattern::A, "Pattern A", "The standard head, eye, body and mouth parameters swing back and forth"),
                (Pattern::B, "Pattern B", "Every ticked parameter moves at random; the eyes swing"),
                (Pattern::Posing, "Posing", "Every ticked parameter moves at random; eyes, mouth and breath swing"),
            ] {
                if ui.radio_value(&mut pattern, p, tr(label)).on_hover_text(tr(hint)).changed() {
                    self.prefs.random_pose_pattern = p.index();
                    self.random_pose.wanders.clear();
                    save_prefs = true;
                }
            }
            ui.add_space(6.0);
            let fixed = pattern == Pattern::A;
            ui.horizontal(|ui| {
                ui.label(tr("Parameters that take part"));
                if fixed {
                    ui.weak(tr("(Pattern A uses its own set)"));
                }
            });
            save_prefs |= self.random_pose_list(ui, fixed);
        });
        if save_prefs {
            self.prefs.save();
        }
        if !open && self.random_pose.playing {
            self.random_pose_toggle();
        }
        self.random_pose.show = open;
    }

    /// The tick list, by folder like the palette. Returns whether the
    /// prefs changed.
    fn random_pose_list(&mut self, ui: &mut Ui, disabled: bool) -> bool {
        use crate::i18n::tr;
        let Some(s) = &self.session else { return false };
        let n = s.puppet.model().parameters.len();
        let ids: Vec<String> = s.puppet.model().parameters.iter().map(|p| p.id.clone()).collect();
        let mut folders: Vec<(String, Vec<usize>)> =
            s.groups.iter().map(|g| (g.name.clone(), g.params.clone())).collect();
        let grouped: Vec<usize> = folders.iter().flat_map(|(_, p)| p.iter().copied()).collect();
        let loose: Vec<usize> = (0..n).filter(|i| !grouped.contains(i)).collect();
        if !loose.is_empty() {
            folders.push((String::new(), loose));
        }
        let mut changed = false;
        let mut set_all: Option<bool> = None;
        ui.horizontal(|ui| {
            if ui.add_enabled(!disabled, egui::Button::new(tr("Select All"))).clicked() {
                set_all = Some(true);
            }
            if ui.add_enabled(!disabled, egui::Button::new(tr("Deselect All"))).clicked() {
                set_all = Some(false);
            }
            if ui.button(tr("Expand All")).clicked() {
                self.random_pose.folded.clear();
            }
            if ui.button(tr("Collapse All")).clicked() {
                self.random_pose.folded = folders.iter().map(|(name, _)| name.clone()).collect();
            }
        });
        if let Some(on) = set_all {
            for i in 0..n {
                self.random_pose_set(i, on);
            }
            changed = true;
        }
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for (fi, (name, params)) in folders.iter().enumerate() {
                let on: Vec<bool> =
                    params.iter().map(|&i| self.random_pose_takes_part(i)).collect();
                let all = on.iter().all(|&b| b);
                let any = on.iter().any(|&b| b);
                let folded = self.random_pose.folded.contains(name);
                ui.horizontal(|ui| {
                    let arrow = if folded { "▸" } else { "▾" };
                    if ui.small_button(arrow).clicked() {
                        if folded {
                            self.random_pose.folded.retain(|f| f != name);
                        } else {
                            self.random_pose.folded.push(name.clone());
                        }
                    }
                    // "-" on the folder when its parameters disagree
                    let mut tick = all;
                    let folder =
                        if name.is_empty() { tr("Ungrouped").to_string() } else { name.clone() };
                    let label = if any && !all { format!("{folder}  -") } else { folder };
                    let box_ = ui.add_enabled(!disabled, egui::Checkbox::new(&mut tick, label));
                    if box_.changed() {
                        for &i in params {
                            self.random_pose_set(i, tick);
                        }
                        changed = true;
                    }
                });
                if folded {
                    continue;
                }
                ui.indent(("random-pose-folder", fi), |ui| {
                    for &i in params {
                        let Some(id) = ids.get(i) else { continue };
                        let mut tick = self.random_pose_takes_part(i);
                        if ui.add_enabled(!disabled, egui::Checkbox::new(&mut tick, id)).changed() {
                            self.random_pose_set(i, tick);
                            changed = true;
                        }
                    }
                });
            }
        });
        changed
    }
}
