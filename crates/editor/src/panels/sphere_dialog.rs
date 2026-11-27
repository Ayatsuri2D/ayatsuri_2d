// Modeling > Deformer > Sphere Warp: write the selected warp deformer's
// keyforms on two parameters as the visible face of a sphere turning with
// them (the core's `sphere::sphere_warp`). The dialog picks the two
// parameters, the keys to write on each, the depth of the lift and the
// turn at the parameters' ends, then Apply does it in one undo step.

use ayatsuri_core::edit::Target;
use ayatsuri_core::sphere::{SphereParams, sphere_warp};
use ayatsuri_format::model::{DeformerKind, Model};
use egui::{Ui, vec2};

use super::{EditorUi, Sel};
use crate::i18n::{tr, trf};

pub(super) struct SphereDialog {
    deformer: usize,
    param_x: usize,
    param_y: usize,
    /// The keys to write, as typed: "-30, 0, 30".
    keys_x: String,
    keys_y: String,
    depth: f32,
    max_angle: f32,
}

/// The keys typed in a field: numbers split by commas, semicolons or
/// spaces, a typographic minus allowed. None when any piece is not a number.
pub(super) fn parse_keys(text: &str) -> Option<Vec<f32>> {
    let mut keys = Vec::new();
    for piece in text.split([',', ';', ' ', '\t']) {
        let piece = piece.trim().replace('\u{2212}', "-");
        if piece.is_empty() {
            continue;
        }
        let v: f32 = piece.parse().ok().filter(|v: &f32| v.is_finite())?;
        keys.push(v);
    }
    Some(keys)
}

/// A parameter's minimum, default and maximum as key text, with its own
/// decimals.
pub(super) fn default_keys(model: &Model, param: usize) -> String {
    let Some(p) = model.parameters.get(param) else { return String::new() };
    let d = p.decimals.clamp(0, 6) as usize;
    let mut keys = vec![p.min, p.default, p.max];
    keys.sort_by(f32::total_cmp);
    keys.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    keys.iter().map(|k| format!("{k:.d$}")).collect::<Vec<_>>().join(", ")
}

/// The parameter with this standard ID, when the model has it.
fn param_by_id(model: &Model, id: &str) -> Option<usize> {
    model.parameters.iter().position(|p| p.id == id)
}

impl EditorUi {
    /// The selected warp deformer, when the selection is one.
    pub(super) fn selected_warp(&self) -> Option<usize> {
        let s = self.session.as_ref()?;
        let Some(Sel::Deformer(d)) = self.selected else { return None };
        let kind = &s.puppet.model().deformers.get(d)?.kind;
        matches!(kind, DeformerKind::Warp(_)).then_some(d)
    }

    /// Open the Sphere Warp dialog for the selected warp deformer, on Angle
    /// X and Angle Y when the model has them.
    pub(super) fn open_sphere_dialog(&mut self) {
        let Some(deformer) = self.selected_warp() else {
            log::warn!("sphere warp: select a warp deformer first");
            return;
        };
        let Some(s) = &self.session else { return };
        let model = s.puppet.model();
        if model.parameters.len() < 2 {
            log::warn!(
                "sphere warp needs two parameters; the model has {}",
                model.parameters.len()
            );
            return;
        }
        let param_x = param_by_id(model, "ParamAngleX").unwrap_or(0);
        let param_y = param_by_id(model, "ParamAngleY").unwrap_or(if param_x == 0 { 1 } else { 0 });
        self.sphere_dlg = Some(SphereDialog {
            deformer,
            param_x,
            param_y,
            keys_x: default_keys(model, param_x),
            keys_y: default_keys(model, param_y),
            depth: 1.0,
            max_angle: 30.0,
        });
    }

    pub(super) fn sphere_dialog(&mut self, ui: &mut Ui) {
        let Some(dlg) = &mut self.sphere_dlg else { return };
        let Some(s) = &self.session else {
            self.sphere_dlg = None;
            return;
        };
        let model = s.puppet.model();
        let is_warp = model
            .deformers
            .get(dlg.deformer)
            .is_some_and(|d| matches!(d.kind, DeformerKind::Warp(_)));
        if !is_warp {
            self.sphere_dlg = None;
            return;
        }
        let name = s.deformer_name(dlg.deformer);
        let param_name = |i: usize| -> String {
            match s.param_names.get(i).filter(|n| !n.trim().is_empty()) {
                Some(n) => n.clone(),
                None => model.parameters.get(i).map(|p| p.id.clone()).unwrap_or_default(),
            }
        };
        let keyed = |param: usize| -> bool {
            ayatsuri_core::keyform::keys_on(model, Target::Deformer(dlg.deformer), param)
                .is_some_and(|k| !k.is_empty())
        };
        let mut open = true;
        let mut go = false;
        crate::windows::Dialog::new(tr("Sphere Warp")).open(&mut open).width(400.0).show(
            ui.ctx(),
            |ui| {
                ui.label(trf(
                    "Write the keyforms of {name} as a sphere turning with two parameters.",
                    &[("name", &name)],
                ));
                ui.add_space(4.0);
                let n = model.parameters.len();
                crate::windows::form(ui, "sphere-warp", |ui| {
                    for (label, param, keys, salt) in [
                        (tr("Parameter X"), &mut dlg.param_x, &mut dlg.keys_x, "sphere-px"),
                        (tr("Parameter Y"), &mut dlg.param_y, &mut dlg.keys_y, "sphere-py"),
                    ] {
                        ui.label(label);
                        let before = *param;
                        egui::ComboBox::from_id_salt(salt)
                            .selected_text(param_name(*param))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for i in 0..n {
                                    ui.selectable_value(param, i, param_name(i));
                                }
                            });
                        if *param != before {
                            *keys = default_keys(model, *param);
                        }
                        ui.end_row();
                        ui.label(tr("Keys"));
                        ui.add(egui::TextEdit::singleline(keys).desired_width(220.0));
                        ui.end_row();
                    }
                    ui.label(tr("Depth"));
                    ui.add(
                        egui::DragValue::new(&mut dlg.depth).range(0.0..=1.0).speed(0.01),
                    )
                    .on_hover_text(tr("1 lifts the grid into a full hemisphere, 0 keeps it flat"));
                    ui.end_row();
                    ui.label(tr("Max angle"));
                    ui.add(egui::DragValue::new(&mut dlg.max_angle).range(10.0..=180.0).suffix("°"))
                        .on_hover_text(tr(
                            "The turn at the end of each parameter's range farther from its default",
                        ));
                    ui.end_row();
                });
                ui.add_space(4.0);
                let same = dlg.param_x == dlg.param_y;
                let keys_x = parse_keys(&dlg.keys_x);
                let keys_y = parse_keys(&dlg.keys_y);
                let blend = [dlg.param_x, dlg.param_y]
                    .iter()
                    .any(|&p| model.parameters.get(p).is_some_and(|q| q.blend_shape));
                let mut can = !same && !blend;
                if same {
                    ui.colored_label(ui.visuals().warn_fg_color, tr("Pick two different parameters."));
                }
                if blend {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        tr("A blend shape parameter cannot turn a sphere."),
                    );
                }
                for (keys, which) in [(&keys_x, dlg.param_x), (&keys_y, dlg.param_y)] {
                    if keys.is_none() {
                        can = false;
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            trf(
                                "The keys for {name} must be numbers, like -30, 0, 30.",
                                &[("name", &param_name(which))],
                            ),
                        );
                    }
                }
                let overwritten: Vec<String> = [dlg.param_x, dlg.param_y]
                    .iter()
                    .filter(|&&p| keyed(p))
                    .map(|&p| param_name(p))
                    .collect();
                if !overwritten.is_empty() && !same {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        trf(
                            "{name} already has keys on {params}; their keyforms will be overwritten.",
                            &[("name", &name), ("params", &overwritten.join(", "))],
                        ),
                    );
                }
                ui.weak(tr(
                    "The default key is the front. The result is plain warp keyforms, so every key can be edited afterwards.",
                ));
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(can, egui::Button::new(tr("Apply")).min_size(vec2(84.0, 0.0)))
                        .clicked()
                    {
                        go = true;
                    }
                });
            },
        );
        if go {
            self.run_sphere_dialog();
        } else if !open {
            self.sphere_dlg = None;
        }
    }

    fn run_sphere_dialog(&mut self) {
        let Some(dlg) = self.sphere_dlg.take() else { return };
        let Some(s) = &mut self.session else { return };
        let (Some(keys_x), Some(keys_y)) = (parse_keys(&dlg.keys_x), parse_keys(&dlg.keys_y))
        else {
            return;
        };
        let name = s.deformer_name(dlg.deformer);
        let model = s.puppet.model();
        let id = |p: usize| model.parameters.get(p).map(|q| q.id.clone()).unwrap_or_default();
        let (id_x, id_y) = (id(dlg.param_x), id(dlg.param_y));
        let params = SphereParams {
            param_x: dlg.param_x,
            param_y: dlg.param_y,
            keys_x,
            keys_y,
            radius_x: 0.0,
            radius_y: 0.0,
            depth: dlg.depth,
            max_angle_deg: dlg.max_angle,
        };
        let done = s.edit(|m| sphere_warp(m, dlg.deformer, params));
        if done {
            let keys = |p: usize| {
                ayatsuri_core::keyform::keys_on(s.puppet.model(), Target::Deformer(dlg.deformer), p)
                    .map_or(0, |k| k.len())
            };
            log::info!(
                "sphere warp on {name}: {id_x} ({} keys) by {id_y} ({} keys), depth {:.2}, {}° at the ends",
                keys(dlg.param_x),
                keys(dlg.param_y),
                dlg.depth,
                dlg.max_angle
            );
            self.selected = Some(Sel::Deformer(dlg.deformer));
        } else {
            log::warn!(
                "sphere warp on {name}: nothing written; it needs a warp deformer, two different parameters and a grid with some size"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_with_commas_spaces_and_a_typographic_minus() {
        assert_eq!(parse_keys("-30, 0, 30"), Some(vec![-30.0, 0.0, 30.0]));
        assert_eq!(parse_keys("\u{2212}1;0.5  2"), Some(vec![-1.0, 0.5, 2.0]));
        assert_eq!(parse_keys(""), Some(Vec::new()));
        assert_eq!(parse_keys("1, two"), None);
    }

    #[test]
    fn default_keys_follow_the_parameter_range_and_decimals() {
        let model = Model {
            parameters: vec![
                ayatsuri_format::model::Parameter {
                    id: "A".into(),
                    min: -30.0,
                    max: 30.0,
                    default: 0.0,
                    repeat: false,
                    decimals: 1,
                    blend_shape: false,
                },
                ayatsuri_format::model::Parameter {
                    id: "B".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    repeat: false,
                    decimals: 2,
                    blend_shape: false,
                },
            ],
            ..Default::default()
        };
        assert_eq!(default_keys(&model, 0), "-30.0, 0.0, 30.0");
        assert_eq!(default_keys(&model, 1), "0.00, 1.00");
        assert_eq!(default_keys(&model, 5), "");
    }
}
