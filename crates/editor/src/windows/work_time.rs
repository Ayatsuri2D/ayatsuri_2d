// Window > Work Time: how long the open project has been worked on. The
// count runs only while the user is doing something (input within the last
// half minute), so a model left open over lunch adds nothing, and it is
// saved in the .aom, so it carries across sessions and machines. Meant for
// pricing commissions by the hours they took.

use crate::panels::EditorUi;
use egui::Ui;

/// How long after the last input the timer keeps counting.
pub(crate) const IDLE_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

impl EditorUi {
    /// Called once a frame: adds the frame's length to the open project's
    /// work time while the user is active and the timer is not paused.
    pub(crate) fn count_work(&mut self, ctx: &egui::Context, now: std::time::Instant) {
        let active =
            ctx.input(|i| i.pointer.is_moving() || i.pointer.any_down() || !i.events.is_empty());
        if active {
            self.last_activity = Some(now);
        }
        let Some(s) = &mut self.session else { return };
        if s.work_paused {
            return;
        }
        let recent = self.last_activity.is_some_and(|t| now.duration_since(t) <= IDLE_AFTER);
        if recent && self.frame_dt > 0.0 && self.frame_dt <= 0.5 {
            s.work_seconds += self.frame_dt as f64;
        }
    }

    pub(crate) fn work_time_window(&mut self, ui: &mut Ui) {
        if !self.show_work_time {
            return;
        }
        let mut open = self.show_work_time;
        let counting = self.last_activity.is_some_and(|t| t.elapsed() <= IDLE_AFTER);
        crate::windows::Dialog::new(crate::i18n::tr("Work Time")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                let Some(s) = &mut self.session else {
                    ui.weak(crate::i18n::tr("No model loaded."));
                    return;
                };
                ui.set_width(260.0);
                ui.label(egui::RichText::new(clock(s.work_seconds)).size(28.0).strong());
                ui.weak(crate::i18n::trf(
                    "on {name}, saved with the project",
                    &[("name", &s.name)],
                ));
                ui.add_space(6.0);
                let state = if s.work_paused {
                    crate::i18n::tr("Paused")
                } else if counting {
                    crate::i18n::tr("Counting")
                } else {
                    crate::i18n::tr("Idle: counting resumes on your next input")
                };
                ui.label(state);
                ui.horizontal(|ui| {
                    let label = if s.work_paused {
                        crate::i18n::tr("Resume")
                    } else {
                        crate::i18n::tr("Pause")
                    };
                    if ui.button(label).clicked() {
                        s.work_paused = !s.work_paused;
                    }
                    if ui.button(crate::i18n::tr("Reset to zero")).clicked() {
                        s.work_seconds = 0.0;
                        s.modified = true;
                    }
                });
                ui.add_space(4.0);
                ui.weak(crate::i18n::tr(
                    "Counts only while you work: it stops half a minute after your last input and starts again on the next.",
                ));
            },
        );
        self.show_work_time = open;
    }
}

/// Seconds as h:mm:ss.
pub(crate) fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!("{}:{:02}:{:02}", total / 3600, (total / 60) % 60, total % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_clock_reads_hours_minutes_seconds() {
        assert_eq!(super::clock(0.0), "0:00:00");
        assert_eq!(super::clock(59.4), "0:00:59");
        assert_eq!(super::clock(3600.0 * 27.0 + 61.0), "27:01:01");
    }
}
