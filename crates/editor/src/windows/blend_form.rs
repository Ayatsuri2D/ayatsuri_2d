// Modeling > Edit Form > Blend Form: the copied form mixed into the
// keyform the parameters sit on, by an amount from 0 (the keyform as it
// is) to 100 percent (a plain paste). The view follows the slider through
// an amend and Apply writes the blend as one undo step; Cancel puts the
// keyform back.

use crate::panels::EditorUi;
use ayatsuri_core::edit::Target;
use ayatsuri_core::keyform::{Form, blend_form, paste_form};
use egui::Ui;

pub(crate) struct BlendDialog {
    target: Target,
    name: String,
    keyform: usize,
    /// The keyform before the dialog touched it.
    base: Form,
    /// What Copy Form took.
    from: Form,
    /// 0 to 1.
    amount: f32,
    /// The amount the view shows right now.
    shown: f32,
    was_modified: bool,
}

impl EditorUi {
    /// Open the dialog on the selected object, when a full form was copied
    /// and the parameters sit on one of its keys.
    pub(crate) fn open_blend_form(&mut self) {
        use crate::i18n::tr;
        let Some(from) = self.form_clip_full.clone() else {
            log::warn!("{}", tr("Copy a form first (Ctrl+Shift+C with the parameters on a key)"));
            return;
        };
        let Some((target, name)) = self.edit_target() else {
            log::warn!(
                "{}",
                tr("Select an object of the kind the form came from, with the parameters on a key")
            );
            return;
        };
        let Some(s) = &mut self.session else { return };
        let values: Vec<f32> =
            (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
        let Some(keyform) = ayatsuri_core::keyform::keyform_at(s.puppet.model(), target, &values)
        else {
            log::warn!(
                "{}",
                tr("Select an object of the kind the form came from, with the parameters on a key")
            );
            return;
        };
        let was_modified = s.modified;
        // a dry run at 0 tells whether the shapes match before anything shows
        let mut probe = s.puppet.model().clone();
        if !blend_form(&mut probe, target, keyform, &from, 0.0) {
            log::warn!(
                "{}",
                tr(
                    "The shapes do not match: the object needs the same number of points as the copied one"
                )
            );
            return;
        }
        let Some(base) = ayatsuri_core::keyform::copy_form(&mut probe, target, keyform) else {
            return;
        };
        self.blend_dlg = Some(BlendDialog {
            target,
            name,
            keyform,
            base,
            from,
            amount: 0.5,
            shown: 0.0,
            was_modified,
        });
    }

    pub(crate) fn blend_form_window(&mut self, ui: &mut Ui) {
        use crate::i18n::{tr, trf};
        let Some(d) = &mut self.blend_dlg else { return };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        crate::windows::Dialog::new(tr("Blend Form")).open(&mut open).show(ui.ctx(), |ui| {
            ui.set_width(300.0);
            ui.weak(trf("Into {name}, keyform {k}", &[("name", &d.name), ("k", &d.keyform)]));
            ui.add_space(4.0);
            let mut percent = (d.amount * 100.0).round();
            ui.horizontal(|ui| {
                ui.label(tr("Amount"));
                ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%").step_by(1.0));
            });
            d.amount = (percent / 100.0).clamp(0.0, 1.0);
            ui.weak(tr(
                "0% keeps the shape as it is, 100% is a plain paste. The view follows the slider; Apply writes it as one undo step.",
            ));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new(tr("Apply")).strong()).clicked() {
                    apply = true;
                }
                if ui.button(tr("Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if !open {
            cancel = true;
        }
        let Some(s) = &mut self.session else {
            self.blend_dlg = None;
            return;
        };
        if apply || cancel {
            let Some(d) = self.blend_dlg.take() else { return };
            // back to the shape before the preview, then one real edit
            if d.shown != 0.0 {
                s.amend(|m| paste_form(m, d.target, d.keyform, &d.base));
            }
            s.modified = d.was_modified;
            if apply
                && d.amount > 0.0
                && s.edit(|m| blend_form(m, d.target, d.keyform, &d.from, d.amount))
            {
                log::info!(
                    "form blended into keyform {} of {} at {:.0}%",
                    d.keyform,
                    d.name,
                    d.amount * 100.0
                );
            }
            return;
        }
        if (d.amount - d.shown).abs() > 1e-4 {
            let (target, keyform, base, from, amount) =
                (d.target, d.keyform, d.base.clone(), d.from.clone(), d.amount);
            s.amend(|m| {
                paste_form(m, target, keyform, &base)
                    && blend_form(m, target, keyform, &from, amount)
            });
            d.shown = amount;
        }
    }
}
