//! Foreign formats in, [`ayatsuri_format::Model`] out.
//!
//! This crate is the only bridge between the hostile-input parsers and the
//! runtime: `ayatsuri-core` never sees a `.moc3` (docs/architecture.md). Every
//! cross-pool index is validated here, so the model handed out cannot point
//! out of range.

#![forbid(unsafe_code)]

#[cfg(feature = "documents")]
mod clip;
#[cfg(feature = "documents")]
mod cmo3;
mod exp3;
#[cfg(feature = "documents")]
mod kra;
#[cfg(feature = "documents")]
mod layers;
mod moc3;
mod moc3_export;
mod motion3;
mod physics3;
#[cfg(feature = "documents")]
mod psd_doc;
#[cfg(feature = "documents")]
mod sqlite;

#[cfg(feature = "documents")]
pub use layers::{AlphaBlend, Blend, ColorBlend, Document, Folder, Imported, Layer};

/// A paint document's layers by file extension (`psd`, `kra` or `clip`),
/// before they are turned into a model, with how many layers were skipped
/// while reading. Anything else is read as a PSD.
#[cfg(feature = "documents")]
pub fn read_document(bytes: &[u8], ext: &str) -> Result<(Document, usize), ImportError> {
    let read = match ext {
        "kra" => kra::read_kra(bytes),
        "clip" => clip::read_clip(bytes),
        _ => psd_doc::read_psd(bytes).map(|d| (d, 0)),
    };
    match &read {
        Ok((doc, skipped)) => log::debug!(
            "{ext}: {} x {} document, {} layer(s), {skipped} skipped",
            doc.width,
            doc.height,
            doc.layers.len()
        ),
        Err(e) => log::debug!("{ext}: could not read the document: {e}"),
    }
    read
}

/// Whether a file extension names a paint document this crate opens.
#[cfg(feature = "documents")]
pub fn is_paint_ext(ext: &str) -> bool {
    matches!(ext, "psd" | "kra" | "clip")
}

/// Whether a file extension names a plain image (`png`, `jpg`, `jpeg`)
/// this crate turns into a single layer.
#[cfg(feature = "documents")]
pub fn is_image_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg")
}

/// A PNG or JPEG file as a model with one mesh, named `name`. With a
/// `canvas` (width, height in pixels) the picture is centred on it and
/// clipped to it, the way a paint document's layers are; without one the
/// picture is the canvas.
#[cfg(feature = "documents")]
pub fn from_image(
    bytes: &[u8],
    name: &str,
    canvas: Option<(u32, u32)>,
) -> Result<Imported, ImportError> {
    let (doc, _) = image_document(bytes, name, canvas)?;
    layers::build(&doc)
}

/// The picture's pixel size, without decoding more than its header.
#[cfg(feature = "documents")]
pub fn image_size(bytes: &[u8]) -> Result<(u32, u32), ImportError> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImportError::Export(format!("image: {e}")))?;
    reader.into_dimensions().map_err(|e| ImportError::Export(format!("image: {e}")))
}

/// A PNG or JPEG as a one-layer document; the second value is the
/// picture's own size.
#[cfg(feature = "documents")]
fn image_document(
    bytes: &[u8],
    name: &str,
    canvas: Option<(u32, u32)>,
) -> Result<(Document, (u32, u32)), ImportError> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| ImportError::Export(format!("image: {e}")))?
        .into_rgba8();
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err(ImportError::Export("image: the picture has no size".into()));
    }
    let (cw, ch) = canvas.unwrap_or((w, h));
    let (cw, ch) = (cw.max(1), ch.max(1));
    // centred: an even margin on each side, or an even overhang
    let x = (i64::from(cw) - i64::from(w)) / 2;
    let y = (i64::from(ch) - i64::from(h)) / 2;
    let layer = Layer {
        name: name.to_string(),
        groups: Vec::new(),
        x: x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y: y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        width: w,
        height: h,
        rgba: img.into_raw(),
        visible: true,
        opacity: 1.0,
        clip_to_below: false,
        blend: Blend::NORMAL,
    };
    Ok((Document { width: cw, height: ch, layers: vec![layer], folders: Vec::new() }, (w, h)))
}

/// A paint document, by extension, as a fresh rig-ready model.
#[cfg(feature = "documents")]
pub fn from_paint(bytes: &[u8], ext: &str) -> Result<Imported, ImportError> {
    let (doc, skipped) = read_document(bytes, ext)?;
    let mut out = layers::build(&doc)?;
    out.skipped += skipped;
    Ok(out)
}

/// Any layered document (already read from its file) as a fresh model.
#[cfg(feature = "documents")]
pub fn from_layers(doc: &Document) -> Result<Imported, ImportError> {
    layers::build(doc)
}

/// A Photoshop document as a fresh, rig-ready model.
#[cfg(feature = "documents")]
pub fn from_psd(bytes: &[u8]) -> Result<Imported, ImportError> {
    let doc = psd_doc::read_psd(bytes)?;
    layers::build(&doc)
}

/// A Krita document as a fresh, rig-ready model. Layers Krita stores in
/// other colour models are skipped and counted.
#[cfg(feature = "documents")]
pub fn from_kra(bytes: &[u8]) -> Result<Imported, ImportError> {
    let (doc, skipped) = kra::read_kra(bytes)?;
    let mut out = layers::build(&doc)?;
    out.skipped += skipped;
    Ok(out)
}

#[cfg(feature = "documents")]
pub use cmo3::{Cmo3, Cmo3Export, from_cmo3, to_cmo3};
pub use exp3::{from_exp3, to_exp3};
pub use moc3::from_moc3;
pub use moc3_export::{ExportVersion, VTS_NOTE, export_warnings, to_moc3};
pub use motion3::{from_motion3, to_motion3};
pub use physics3::{cubism_json, from_physics3, to_physics3};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("moc3: {0}")]
    Moc3(#[from] openmoc3::Error),
    #[error("{what} {at}: index {value} is outside -1..{limit}")]
    BadIndex { what: &'static str, at: usize, value: i64, limit: usize },
    #[error("{what} {at}: span of {len} at {begin} does not fit a pool of {pool}")]
    BadSpan { what: &'static str, at: usize, begin: i64, len: i64, pool: usize },
    #[error("{what} {at}: {got} keyforms stored but the binding calls for {expected}")]
    KeyformMismatch { what: &'static str, at: usize, expected: usize, got: usize },
    #[error("parameter binding {0} is not owned by any parameter")]
    UnownedBinding(usize),
    #[error(
        "deformer {child} appears before its parent {parent}; the hierarchy must be topological"
    )]
    NotTopological { child: usize, parent: usize },
    #[error("deformer {at}: unknown type {value}")]
    UnknownDeformerType { at: usize, value: i32 },
    #[error("physics3.json: {0}")]
    Physics(String),
    #[error("exp3.json: {0}")]
    Expression(String),
    #[error("motion3.json: {0}")]
    Motion(String),
    #[error("export: {0}")]
    Export(String),
}

#[cfg(all(test, feature = "documents"))]
mod image_tests {
    use super::*;

    fn png(w: u32, h: u32, col: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(col));
        let mut out = std::io::Cursor::new(Vec::new());
        // test data: a failed encode leaves an empty buffer the test refuses
        let _ = img.write_to(&mut out, image::ImageFormat::Png);
        out.into_inner()
    }

    #[test]
    fn a_picture_alone_is_the_canvas() {
        let got = from_image(&png(30, 20, [200, 40, 40, 255]), "ref", None).unwrap();
        let c = got.model.canvas;
        assert_eq!((c.width, c.height), (30.0, 20.0));
        assert_eq!(got.model.art_meshes.len(), 1);
        assert_eq!(got.mesh_names, vec!["ref".to_string()]);
        assert_eq!(image_size(&png(30, 20, [0, 0, 0, 255])).unwrap(), (30, 20));
    }

    #[test]
    fn a_picture_on_a_canvas_sits_in_the_middle_and_is_clipped_to_it() {
        // 40 wide on a 100 wide canvas: 30 px of margin each side
        let got = from_image(&png(40, 20, [40, 200, 40, 255]), "p", Some((100, 60))).unwrap();
        let c = got.model.canvas;
        assert_eq!((c.width, c.height), (100.0, 60.0));
        let m = &got.model.art_meshes[0];
        let px: Vec<f32> = m
            .positions
            .chunks(2)
            .take(m.vertex_count as usize)
            .flat_map(|p| {
                [p[0] * c.pixels_per_unit + c.origin_x, p[1] * c.pixels_per_unit + c.origin_y]
            })
            .collect();
        let lo_x = px.iter().step_by(2).cloned().fold(f32::MAX, f32::min);
        let hi_x = px.iter().step_by(2).cloned().fold(f32::MIN, f32::max);
        let lo_y = px.iter().skip(1).step_by(2).cloned().fold(f32::MAX, f32::min);
        assert!((lo_x - 30.0).abs() <= 1.0 && (hi_x - 70.0).abs() <= 1.0, "{lo_x}..{hi_x}");
        assert!((lo_y - 20.0).abs() <= 1.0, "{lo_y}");
        // bigger than the canvas: the overhang is clipped away, the rest stays
        let got = from_image(&png(200, 20, [40, 40, 200, 255]), "wide", Some((100, 60))).unwrap();
        let m = &got.model.art_meshes[0];
        let c = got.model.canvas;
        let xs: Vec<f32> = m
            .positions
            .chunks(2)
            .take(m.vertex_count as usize)
            .map(|p| p[0] * c.pixels_per_unit + c.origin_x)
            .collect();
        let hi = xs.iter().cloned().fold(f32::MIN, f32::max);
        let lo = xs.iter().cloned().fold(f32::MAX, f32::min);
        assert!(lo >= -1.0 && hi <= 101.0, "{lo}..{hi}");
    }

    #[test]
    fn not_a_picture_is_refused() {
        assert!(from_image(b"nope", "x", None).is_err());
        assert!(image_size(b"nope").is_err());
    }
}
