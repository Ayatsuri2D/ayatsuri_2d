//! What a model looks like to the editor right after it opens, and what the
//! first frame hands the GPU, written to the log at debug level. For the
//! reports where a model draws wrong on a machine we cannot sit at: the
//! log alone should say which stage went astray, without the file.

use std::sync::atomic::{AtomicBool, Ordering};

use ayatsuri_render::{Cmd, Packet};

use crate::session::Session;

static GPU_REPORT: AtomicBool = AtomicBool::new(false);

/// Log the opened model: canvas, parts, pages, and one line per mesh as
/// the renderer will see it at the default pose. Debug level only.
pub(crate) fn model(s: &Session) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let m = s.puppet.model();
    let c = m.canvas;
    log::debug!(
        "report {}: canvas {}x{} px, origin ({}, {}), {} px per unit; {} meshes, {} parts, {} deformers, {} pages; hidden by the eye: {} meshes, {} parts, {} deformers",
        s.name,
        c.width,
        c.height,
        c.origin_x,
        c.origin_y,
        c.pixels_per_unit,
        m.art_meshes.len(),
        m.parts.len(),
        m.deformers.len(),
        s.texture_pngs.len(),
        s.mesh_shown.iter().filter(|v| !**v).count(),
        s.part_shown.iter().filter(|v| !**v).count(),
        s.deformer_shown.iter().filter(|v| !**v).count()
    );
    for (i, p) in m.parts.iter().enumerate() {
        log::debug!(
            "report part {i} {:?} ({}) parent {:?} visible {} enabled {} opacity {:.2} offscreen {}",
            s.part_names.get(i).map(String::as_str).unwrap_or(""),
            p.id,
            p.parent_part,
            p.visible,
            p.enabled,
            s.puppet.part_user_opacity(i),
            p.offscreen.as_ref().map_or("no".to_string(), |o| format!(
                "yes blend {:?} opacities {:?}",
                o.blend, o.opacities
            ))
        );
    }
    for (i, t) in s.preview_tex.iter().enumerate() {
        let full = s.texture_pngs.get(i).map_or(0, Vec::len);
        log::debug!("report page {i}: proxy {}x{}, png {} bytes", t.width, t.height, full);
    }
    let mut drawn = 0usize;
    for d in s.puppet.drawables() {
        drawn += 1;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut nan = 0usize;
        for &[x, y] in d.positions.as_chunks::<2>().0 {
            if !x.is_finite() || !y.is_finite() {
                nan += 1;
                continue;
            }
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &[u, v] in d.uvs.as_chunks::<2>().0 {
            u0 = u0.min(u);
            v0 = v0.min(v);
            u1 = u1.max(u);
            v1 = v1.max(v);
        }
        let am = m.art_meshes.get(d.mesh);
        log::debug!(
            "report mesh {} {:?} page {} verts {} tris {} pos units x {:.3}..{:.3} y {:.3}..{:.3} nan {} uv {:.3},{:.3}..{:.3},{:.3} opacity {:.2} blend {:?} flags {:#04x} masks {:?} and {} part {:?} rig visible {} enabled {} shown {}",
            d.mesh,
            s.mesh_name(d.mesh),
            d.texture,
            d.positions.len() / 2,
            d.indices.len() / 3,
            x0,
            x1,
            y0,
            y1,
            nan,
            u0,
            v0,
            u1,
            v1,
            d.opacity,
            d.blend,
            d.flags,
            d.masks,
            d.clip_and.len(),
            am.and_then(|a| a.parent_part),
            am.is_some_and(|a| a.visible),
            am.is_some_and(|a| a.enabled),
            s.mesh_shown.get(d.mesh).copied().unwrap_or(true)
        );
    }
    log::debug!(
        "report: {drawn} of {} meshes reach the renderer at the default pose",
        m.art_meshes.len()
    );
    GPU_REPORT.store(true, Ordering::Release);
}

/// Log the next packet handed to the GPU, once per `model` report.
pub(crate) fn packet(p: &Packet) {
    if !GPU_REPORT.swap(false, Ordering::AcqRel) {
        return;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut nan = 0usize;
    let mut clear = 0usize;
    for v in &p.vertices {
        if !v.pos[0].is_finite() || !v.pos[1].is_finite() {
            nan += 1;
            continue;
        }
        x0 = x0.min(v.pos[0]);
        y0 = y0.min(v.pos[1]);
        x1 = x1.max(v.pos[0]);
        y1 = y1.max(v.pos[1]);
        if v.color[3] <= 0.0 {
            clear += 1;
        }
    }
    let (mut draws, mut masks, mut begins, mut ends, mut deepest, mut depth) =
        (0, 0, 0, 0, 0usize, 0usize);
    let mut pages = std::collections::BTreeSet::new();
    let mut blends = std::collections::BTreeMap::new();
    for c in &p.cmds {
        match c {
            Cmd::Draw { texture, blend, .. } => {
                draws += 1;
                pages.insert(*texture);
                *blends.entry(format!("{blend:?}")).or_insert(0usize) += 1;
            }
            Cmd::MaskSet { .. } => masks += 1,
            Cmd::BeginLayer => {
                begins += 1;
                depth += 1;
                deepest = deepest.max(depth);
            }
            Cmd::EndLayer { .. } => {
                ends += 1;
                depth = depth.saturating_sub(1);
            }
        }
    }
    log::debug!(
        "report gpu: target {}x{} px, to_ndc {:?}; {} vertices ({} not finite, {} fully transparent), {} indices; ndc x {:.3}..{:.3} y {:.3}..{:.3}; {} draws over pages {:?}, {} mask sets, {} layers opened {} closed (deepest {}); blends {:?}; textures sent now: {}, full pages sent now: {}",
        p.width,
        p.height,
        p.to_ndc,
        p.vertices.len(),
        nan,
        clear,
        p.indices.len(),
        x0,
        x1,
        y0,
        y1,
        draws,
        pages.iter().fold((u32::MAX, 0u32), |(lo, hi), &t| (lo.min(t), hi.max(t))),
        masks,
        begins,
        ends,
        deepest,
        blends,
        p.textures.as_ref().map_or(0, |t| t.len()),
        p.pages.as_ref().map_or(0, |t| t.len())
    );
}
