//! A small secret: on the home screen, type "doro" and Ctrl+click, and the
//! pointer becomes a little animated Doro until the same is done again.
//! Nothing is saved; a restart brings the ordinary pointer back.

use egui::{Color32, CursorIcon, Id, LayerId, Order, Pos2, Rect, pos2, vec2};

/// The sprite sheet: FRAMES squares of SIZE pixels in a row.
const SHEET: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/doro.png"));
const FRAMES: usize = 8;
const SIZE: f32 = 48.0;
const FPS: f64 = 20.0;
const WORD: &str = "doro";

#[derive(Default)]
pub(crate) struct Doro {
    on: bool,
    /// The last few letters typed on the home screen outside any text field.
    typed: String,
    texture: Option<egui::TextureHandle>,
}

impl Doro {
    /// Run every frame the home screen is up: watches the letters typed and
    /// the Ctrl+click that follows the word.
    pub(crate) fn home_input(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (typed, click) = ctx.input(|i| {
            let typed: String = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.to_lowercase()),
                    _ => None,
                })
                .collect();
            (typed, i.modifiers.command && i.pointer.primary_pressed())
        });
        if !typed.is_empty() {
            self.typed.push_str(&typed);
            let keep = self.typed.len().saturating_sub(WORD.len());
            if self.typed.is_char_boundary(keep) {
                self.typed.drain(..keep);
            }
        }
        if click && self.typed == WORD {
            self.on = !self.on;
            self.typed.clear();
            log::info!("doro: {}", if self.on { "hello" } else { "bye" });
        }
    }

    /// Run at the end of every frame: hides the pointer and paints the
    /// sprite where it is.
    pub(crate) fn paint(&mut self, ctx: &egui::Context) {
        if !self.on {
            return;
        }
        let Some(at) = ctx.pointer_hover_pos() else { return };
        let Some(texture) = self.texture(ctx) else { return };
        let frame = (ctx.input(|i| i.time) * FPS) as usize % FRAMES;
        let (u0, u1) = (frame as f32 / FRAMES as f32, (frame + 1) as f32 / FRAMES as f32);
        let uv = Rect::from_min_max(pos2(u0, 0.0), pos2(u1, 1.0));
        let rect = Rect::from_center_size(hotspot(at), vec2(SIZE, SIZE));
        ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("doro-cursor"))).image(
            texture,
            rect,
            uv,
            Color32::WHITE,
        );
        ctx.set_cursor_icon(CursorIcon::None);
        ctx.request_repaint_after(std::time::Duration::from_millis((1000.0 / FPS) as u64));
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        if let Some(t) = &self.texture {
            return Some(t.id());
        }
        let img = image::load_from_memory(SHEET).ok()?.into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let handle = ctx.load_texture("doro", color, egui::TextureOptions::LINEAR);
        let id = handle.id();
        self.texture = Some(handle);
        Some(id)
    }
}

/// Where the sprite sits around the pointer: the pointer lands on the face,
/// a little above centre, so clicks feel like they come from the eyes.
fn hotspot(pointer: Pos2) -> Pos2 {
    pointer + vec2(0.0, SIZE * 0.1)
}
