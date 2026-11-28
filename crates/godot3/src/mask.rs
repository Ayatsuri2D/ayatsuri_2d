//! Masks in Godot 3, decided without a Godot type in sight. Godot 3 has no
//! canvas groups, so a mesh's masks are drawn in white into an offscreen
//! viewport (the mask sheet, one per distinct set of masks) sized to the
//! model's canvas, and the mesh is drawn with a small shader that reads
//! the sheet where the fragment falls on the canvas and keeps only what
//! the masks cover (or, for an inverted mask, what they leave).
//!
//! The shader cuts a mesh by scaling its alpha. Godot 3's multiply blend
//! (`GL_DST_COLOR, GL_ZERO`) ignores alpha, so for a Multiply mesh the
//! shader also fades the colour towards white by the alpha: multiplying
//! by white leaves the canvas as it was, which is what being cut out means
//! for that blend. Without it a multiplied mesh (a face shadow, a blush)
//! darkens everything under its triangles, mask or no mask.
//!
//! ```text
//! model canvas (pixels, origin on the node)      mask sheet (viewport)
//! +-------------------+                           +----------+
//! |      o            |   p * scale + origin      |  o       |
//! |   mesh 5          |  ------------------->     |          |
//! |   masks 7, 8      |                           |          |
//! +-------------------+                           +----------+
//! ```

use crate::plan::{Blend, Pixels};

/// The sheet's longest side, in pixels. A mask is a soft shape; a canvas of
/// 4096 pixels gains nothing from a sheet that large, and a 32-bit sheet
/// of that size is 64 MB.
pub const SHEET_CAP: f32 = 1024.0;

/// Sheet pixels per canvas pixel: 1 for a canvas within the cap, smaller
/// for a bigger one so its longest side lands on the cap.
pub fn sheet_scale(width: f32, height: f32) -> f32 {
    let long = width.max(height);
    // a NaN side falls to the plain scale, as the comparison below is false for it
    if long > SHEET_CAP { SHEET_CAP / long } else { 1.0 }
}

/// The sheet's size in pixels for a canvas, never below one pixel.
pub fn sheet_size(width: f32, height: f32) -> (i64, i64) {
    let s = sheet_scale(width, height);
    let side = |v: f32| ((v * s).ceil() as i64).max(1);
    (side(width), side(height))
}

/// Where the canvas's origin lands on the sheet, in sheet pixels: the
/// sheet's canvas transform is a uniform scale by `sheet_scale` with this
/// offset, so a point `p` of the model (pixels, origin on the node) draws
/// at `p * scale + offset`.
pub fn sheet_offset(px: &Pixels) -> (f32, f32) {
    let s = sheet_scale(px.width, px.height);
    (px.origin_x * s, px.origin_y * s)
}

/// The uniforms every mask shader declares, in the order the source
/// does; `missing_uniforms` checks a compiled shader lists them.
pub const UNIFORMS: [&str; 4] = ["mask_tex", "canvas_origin", "canvas_size", "inverted"];

/// Of `UNIFORMS`, the ones not in `listed`, the parameter names the server
/// reports for a shader. A shader that did not compile reports none, so
/// every uniform comes back; a shader that compiled reports all four, so
/// nothing does.
pub fn missing_uniforms<'a>(listed: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let listed: Vec<&str> = listed.into_iter().collect();
    UNIFORMS.into_iter().filter(|u| !listed.contains(u)).collect()
}

/// The shader a masked mesh is drawn with, for its blend. The vertex
/// function keeps the local position; the fragment turns it into a sheet
/// coordinate (`canvas_origin` and `canvas_size` are uniforms) and scales
/// the alpha by the sheet's, or by one minus it when `inverted`. For the
/// multiply blend, which ignores alpha, the colour fades to white by the
/// alpha as well.
pub fn shader_source(blend: Blend) -> String {
    let mode = match blend {
        Blend::Mix => "",
        Blend::Add => "render_mode blend_add;\n",
        Blend::Mul => "render_mode blend_mul;\n",
    };
    let fade = match blend {
        Blend::Mul => "\tc.rgb = mix(vec3(1.0), c.rgb, c.a);\n",
        Blend::Mix | Blend::Add => "",
    };
    format!(
        "shader_type canvas_item;\n\
         {mode}\
         uniform sampler2D mask_tex;\n\
         uniform vec2 canvas_origin;\n\
         uniform vec2 canvas_size;\n\
         uniform bool inverted = false;\n\
         varying vec2 model_px;\n\
         void vertex() {{\n\
         \tmodel_px = VERTEX;\n\
         }}\n\
         void fragment() {{\n\
         \tvec4 c = texture(TEXTURE, UV) * COLOR;\n\
         \tvec2 muv = (model_px + canvas_origin) / canvas_size;\n\
         \tfloat m = texture(mask_tex, muv).a;\n\
         \tif (inverted) {{\n\
         \t\tm = 1.0 - m;\n\
         \t}}\n\
         \tc.a *= m;\n\
         {fade}\
         \tCOLOR = c;\n\
         }}\n"
    )
}

/// The index of a blend's shader in a table of three.
pub fn shader_slot(blend: Blend) -> usize {
    match blend {
        Blend::Mix => 0,
        Blend::Add => 1,
        Blend::Mul => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: f32, height: f32) -> Pixels {
        Pixels { per_unit: 1.0, origin_x: width / 2.0, origin_y: height / 2.0, width, height }
    }

    #[test]
    fn a_small_canvas_keeps_its_size() {
        assert_eq!(sheet_scale(800.0, 600.0), 1.0);
        assert_eq!(sheet_size(800.0, 600.0), (800, 600));
        assert_eq!(sheet_size(1024.0, 1024.0), (1024, 1024));
        assert_eq!(sheet_offset(&pixels(800.0, 600.0)), (400.0, 300.0));
    }

    #[test]
    fn a_large_canvas_lands_on_the_cap() {
        assert_eq!(sheet_scale(2048.0, 4096.0), 0.25);
        assert_eq!(sheet_size(2048.0, 4096.0), (512, 1024));
        assert_eq!(sheet_size(4096.0, 1000.0), (1024, 250));
        // the origin scales with the sheet
        assert_eq!(sheet_offset(&pixels(2048.0, 4096.0)), (256.0, 512.0));
    }

    #[test]
    fn a_degenerate_canvas_still_has_a_sheet() {
        assert_eq!(sheet_size(0.0, 0.0), (1, 1));
        assert_eq!(sheet_scale(0.0, 0.0), 1.0);
        assert_eq!(sheet_scale(f32::NAN, 10.0), 1.0);
    }

    #[test]
    fn each_blend_has_its_render_mode() {
        let mix = shader_source(Blend::Mix);
        assert!(mix.starts_with("shader_type canvas_item;\n"));
        assert!(!mix.contains("render_mode"));
        assert!(shader_source(Blend::Add).contains("render_mode blend_add;"));
        assert!(shader_source(Blend::Mul).contains("render_mode blend_mul;"));
        for b in [Blend::Mix, Blend::Add, Blend::Mul] {
            let s = shader_source(b);
            for name in ["mask_tex", "canvas_origin", "canvas_size", "inverted"] {
                assert!(
                    s.contains(&format!("uniform sampler2D {name}"))
                        || s.contains(&format!("uniform vec2 {name}"))
                        || s.contains(&format!("uniform bool {name}")),
                    "{name}"
                );
            }
        }
        assert_eq!(
            [shader_slot(Blend::Mix), shader_slot(Blend::Add), shader_slot(Blend::Mul)],
            [0, 1, 2]
        );
    }

    #[test]
    fn only_the_multiply_shader_fades_its_colour_to_white() {
        let fade = "c.rgb = mix(vec3(1.0), c.rgb, c.a);";
        assert!(shader_source(Blend::Mul).contains(fade));
        assert!(!shader_source(Blend::Mix).contains(fade));
        assert!(!shader_source(Blend::Add).contains(fade));
        // the fade comes after the mask has scaled the alpha and before the write
        let mul = shader_source(Blend::Mul);
        let (cut, fade_at, write) = (
            mul.find("c.a *= m;").unwrap_or(usize::MAX),
            mul.find(fade).unwrap_or(0),
            mul.find("COLOR = c;").unwrap_or(0),
        );
        assert!(cut < fade_at && fade_at < write, "{mul}");
    }

    #[test]
    fn the_uniform_check_names_what_a_failed_compile_lacks() {
        assert!(missing_uniforms(UNIFORMS).is_empty());
        assert_eq!(missing_uniforms([]), UNIFORMS.to_vec());
        assert_eq!(
            missing_uniforms(["canvas_origin", "canvas_size", "something_else"]),
            ["mask_tex", "inverted"]
        );
    }
}
