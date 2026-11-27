//! The drawing: one `VisualServer` canvas item per drawable, children of
//! the node's own canvas item so they move, scale and hide with it. The
//! items are made once and kept; a frame submits triangles again only for
//! the meshes that moved, and hands out draw indices only when the order
//! changed.
//!
//! Godot 3 has no canvas groups, so a masked drawable is drawn with a
//! shader material that reads a mask sheet: an offscreen viewport, one per
//! distinct set of masks, into which the masks' triangles are drawn in
//! white (see `mask.rs`).
//!
//! ```text
//! node canvas item
//! ├── item (mesh 4)                              draw index 0
//! ├── item (mesh 5, shader reads sheet {7, 8})   draw index 1
//! └── item (mesh 9)                              draw index 2
//!
//! sheet {7, 8}: viewport -> canvas -> item with masks 7 and 8
//! ```
//!
//! Which items to touch is decided in `plan`; this file only makes the
//! calls. Every `VisualServer` method is `unsafe` in the gdnative crate,
//! which cannot see that the server outlives the node; the one `SAFETY`
//! note below covers them all.

use std::collections::{HashMap, HashSet};

use gdnative::api::{CanvasItemMaterial, Texture, VisualServer};
use gdnative::core_types::{Color, PoolArray, Rid, Transform2D, Vector2};
use gdnative::log::godot_warn;
use gdnative::object::Ref;
use gdnative::object::ownership::Shared;

use ayatsuri_capi::{AY2D_CHANGED_OPACITY, AY2D_CHANGED_POSITIONS, AY2D_CHANGED_VISIBILITY};

use crate::mask;
use crate::plan::{Blend, Pixels, Plan, blend_of, inverted_mask};
use crate::runtime::{Drawable, Runtime};

/// A texture page: the resource kept alive, and its RID for the server.
pub struct Page {
    pub _keep: Ref<Texture, Shared>,
    pub rid: Rid,
}

/// A mesh's canvas item and what hangs off it.
struct Item {
    item: Rid,
    /// Its own shader material when it is masked (freed with it).
    material: Option<Rid>,
}

/// One offscreen viewport holding a set of masks drawn in white.
struct Sheet {
    viewport: Rid,
    canvas: Rid,
    item: Rid,
    texture: Rid,
    drawn_once: bool,
}

pub struct Painter {
    /// The node's canvas item, parent of everything here.
    parent: Rid,
    pixels: Pixels,
    items: HashMap<u32, Item>,
    /// The texture pages, by the runtime's texture index; `None` for a
    /// page that did not decode, drawn untextured.
    textures: Vec<Option<Page>>,
    /// Kept alive as long as the items that use their RIDs.
    _add: Ref<CanvasItemMaterial, Shared>,
    _mul: Ref<CanvasItemMaterial, Shared>,
    add_rid: Rid,
    mul_rid: Rid,
    /// The mask shaders by blend (`mask::shader_slot`), made when first
    /// needed.
    shaders: [Option<Rid>; 3],
    sheets: HashMap<Vec<u32>, Sheet>,
    said_blend: bool,
    // scratch, so a frame allocates nothing once these have grown
    points: Vec<Vector2>,
    uvs: Vec<Vector2>,
    colors: Vec<Color>,
    indices: Vec<i32>,
}

/// The visual server. Its singleton is `unsafe` to fetch in the gdnative
/// crate because the crate cannot prove the engine is running; the painter
/// is only ever called from a node's methods, so it is.
fn server() -> &'static VisualServer {
    // SAFETY: called from the node's `_ready`, `_process`, its setters and
    // its destruction, all of which the engine runs on the main thread
    // while the servers are alive.
    unsafe { VisualServer::godot_singleton() }
}

impl Painter {
    pub fn new(parent: Rid, pixels: Pixels, textures: Vec<Option<Page>>) -> Self {
        let add = CanvasItemMaterial::new();
        add.set_blend_mode(CanvasItemMaterial::BLEND_MODE_ADD);
        let mul = CanvasItemMaterial::new();
        mul.set_blend_mode(CanvasItemMaterial::BLEND_MODE_MUL);
        let (add_rid, mul_rid) = (add.get_rid(), mul.get_rid());
        Self {
            parent,
            pixels,
            items: HashMap::new(),
            textures,
            _add: add.into_shared(),
            _mul: mul.into_shared(),
            add_rid,
            mul_rid,
            shaders: [None, None, None],
            sheets: HashMap::new(),
            said_blend: false,
            points: Vec::new(),
            uvs: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// The meshes that have items, for the plan.
    pub fn known(&self) -> Vec<u32> {
        self.items.keys().copied().collect()
    }

    /// Carry out one frame's plan over this frame's drawables (back to
    /// front). `changes` holds the `AY2D_CHANGED_*` bits by mesh, for the
    /// mask sheets.
    pub fn frame(&mut self, rt: &Runtime, drawn: &[Drawable], plan: &Plan, changes: &[u32]) {
        let vs = server();
        let redraw: HashSet<u32> = plan.redraw.iter().copied().collect();
        // SAFETY: see `server`; every RID passed was made by this painter
        // and not freed yet (freeing clears the maps).
        unsafe {
            for &m in &plan.hide {
                if let Some(it) = self.items.get(&m) {
                    vs.canvas_item_set_visible(it.item, false);
                }
            }
            for (at, d) in drawn.iter().enumerate() {
                let fresh = !self.items.contains_key(&d.mesh);
                if fresh {
                    self.make(vs, rt, d);
                }
                let Some(it) = self.items.get(&d.mesh) else { continue };
                let item = it.item;
                if plan.reorder || fresh {
                    vs.canvas_item_set_draw_index(item, at as i64);
                    vs.canvas_item_set_visible(item, true);
                }
                if redraw.contains(&d.mesh) {
                    vs.canvas_item_set_visible(item, true);
                    vs.canvas_item_clear(item);
                    self.append(vs, item, d, d.opacity);
                }
            }
            self.sheets_frame(vs, rt, changes);
        }
    }

    /// Redraw every mask sheet whose masks moved, and the ones never drawn.
    unsafe fn sheets_frame(&mut self, vs: &VisualServer, rt: &Runtime, changes: &[u32]) {
        let bits = |m: u32| changes.get(m as usize).copied().unwrap_or(u32::MAX);
        let moved = AY2D_CHANGED_POSITIONS | AY2D_CHANGED_VISIBILITY | AY2D_CHANGED_OPACITY;
        let keys: Vec<Vec<u32>> = self
            .sheets
            .iter()
            .filter(|(masks, sheet)| {
                !sheet.drawn_once || masks.iter().any(|&m| bits(m) & moved != 0)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            let Some(sheet) = self.sheets.get_mut(&key) else { continue };
            let (item, viewport) = (sheet.item, sheet.viewport);
            sheet.drawn_once = true;
            // SAFETY: see `frame`.
            unsafe {
                vs.canvas_item_clear(item);
                for &mask in &key {
                    if let Some(geometry) = rt.mesh_geometry(mask) {
                        self.append(vs, item, &geometry, 1.0);
                    }
                }
                // the viewport renders once more, then rests until the
                // masks move again
                vs.viewport_set_update_mode(viewport, VisualServer::VIEWPORT_UPDATE_ONCE);
            }
        }
    }

    /// The item for a mesh seen for the first time, with its blend
    /// material, or its mask shader material and sheet when it has masks.
    unsafe fn make(&mut self, vs: &VisualServer, rt: &Runtime, d: &Drawable) {
        let item = vs.canvas_item_create();
        // SAFETY: see `frame`.
        unsafe {
            vs.canvas_item_set_parent(item, self.parent);
        }
        let (blend, advanced) = blend_of(d.flags, rt.mesh_blend(d.mesh));
        if advanced && !self.said_blend {
            self.said_blend = true;
            godot_warn!(
                "Ayatsuri2D: mesh {} uses a Cubism 5.3 blend mode the canvas cannot do; it is drawn with the plain blend instead",
                d.mesh
            );
        }
        let material = if d.masks.is_empty() {
            // SAFETY: see `frame`.
            unsafe {
                match blend {
                    Blend::Mix => {}
                    Blend::Add => vs.canvas_item_set_material(item, self.add_rid),
                    Blend::Mul => vs.canvas_item_set_material(item, self.mul_rid),
                }
            }
            None
        } else {
            let mut key = d.masks.to_vec();
            key.sort_unstable();
            key.dedup();
            // SAFETY: see `frame`.
            let texture = unsafe { self.sheet(vs, &key) };
            let shader = unsafe { self.shader(vs, blend) };
            let material = vs.material_create();
            // SAFETY: see `frame`.
            unsafe {
                vs.material_set_shader(material, shader);
                vs.material_set_param(material, "mask_tex", texture);
                vs.material_set_param(
                    material,
                    "canvas_origin",
                    Vector2::new(self.pixels.origin_x, self.pixels.origin_y),
                );
                vs.material_set_param(
                    material,
                    "canvas_size",
                    Vector2::new(self.pixels.width, self.pixels.height),
                );
                vs.material_set_param(material, "inverted", inverted_mask(d.flags));
                vs.canvas_item_set_material(item, material);
            }
            Some(material)
        };
        self.items.insert(d.mesh, Item { item, material });
    }

    /// The sheet for a set of masks: made on first use. Returns its
    /// texture RID.
    unsafe fn sheet(&mut self, vs: &VisualServer, key: &[u32]) -> Rid {
        if let Some(s) = self.sheets.get(key) {
            return s.texture;
        }
        let (w, h) = mask::sheet_size(self.pixels.width, self.pixels.height);
        let scale = mask::sheet_scale(self.pixels.width, self.pixels.height);
        let (ox, oy) = mask::sheet_offset(&self.pixels);
        let viewport = vs.viewport_create();
        let canvas = vs.canvas_create();
        let item = vs.canvas_item_create();
        // SAFETY: see `frame`.
        unsafe {
            vs.viewport_set_size(viewport, w, h);
            vs.viewport_set_usage(viewport, VisualServer::VIEWPORT_USAGE_2D);
            vs.viewport_set_disable_3d(viewport, true);
            vs.viewport_set_hdr(viewport, false);
            vs.viewport_set_transparent_background(viewport, true);
            // Godot 3 renders a viewport upside down unless told to flip
            vs.viewport_set_vflip(viewport, true);
            vs.viewport_set_update_mode(viewport, VisualServer::VIEWPORT_UPDATE_ONCE);
            vs.viewport_set_active(viewport, true);
            vs.viewport_attach_canvas(viewport, canvas);
            vs.viewport_set_canvas_transform(
                viewport,
                canvas,
                Transform2D::from_scale_rotation_origin(
                    Vector2::new(scale, scale),
                    0.0,
                    Vector2::new(ox, oy),
                ),
            );
            vs.canvas_item_set_parent(item, canvas);
        }
        // SAFETY: see `frame`.
        let texture = unsafe { vs.viewport_get_texture(viewport) };
        self.sheets
            .insert(key.to_vec(), Sheet { viewport, canvas, item, texture, drawn_once: false });
        texture
    }

    /// The mask shader for a blend: compiled on first use.
    unsafe fn shader(&mut self, vs: &VisualServer, blend: Blend) -> Rid {
        let slot = mask::shader_slot(blend);
        if let Some(Some(s)) = self.shaders.get(slot) {
            return *s;
        }
        let shader = vs.shader_create();
        // SAFETY: see `frame`.
        unsafe {
            vs.shader_set_code(shader, mask::shader_source(blend));
        }
        if let Some(s) = self.shaders.get_mut(slot) {
            *s = Some(shader);
        }
        shader
    }

    /// Add a drawable's triangles to an item, in pixels, with its page.
    unsafe fn append(&mut self, vs: &VisualServer, item: Rid, d: &Drawable, opacity: f32) {
        if d.indices.len() < 3 || d.positions.len() < 2 {
            return;
        }
        self.points.clear();
        self.uvs.clear();
        self.colors.clear();
        self.indices.clear();
        let tint = Color::from_rgba(1.0, 1.0, 1.0, opacity);
        let (xys, _) = d.positions.as_chunks::<2>();
        let (uvs, _) = d.uvs.as_chunks::<2>();
        for (&[x, y], &[u, v]) in xys.iter().zip(uvs) {
            let (px, py) = self.pixels.point(x, y);
            self.points.push(Vector2::new(px, py));
            self.uvs.push(Vector2::new(u, v));
            self.colors.push(tint);
        }
        let n = self.points.len();
        // a triangle naming a vertex past the mesh would be refused whole
        // by the server; keep the ones that are in range
        let (tris, _) = d.indices.as_chunks::<3>();
        for tri in tris {
            if tri.iter().all(|&i| (i as usize) < n) {
                self.indices.extend(tri.iter().map(|&i| i32::from(i)));
            }
        }
        if self.indices.is_empty() {
            return;
        }
        let page = self
            .textures
            .get(d.texture as usize)
            .and_then(|t| t.as_ref())
            .map_or_else(Rid::new, |t| t.rid);
        // SAFETY: see `frame`.
        unsafe {
            vs.canvas_item_add_triangle_array(
                item,
                PoolArray::from_slice(&self.indices),
                PoolArray::from_slice(&self.points),
                PoolArray::from_slice(&self.colors),
                PoolArray::from_slice(&self.uvs),
                PoolArray::new(),
                PoolArray::new(),
                page,
                -1,
                Rid::new(),
                false,
                false,
            );
        }
    }

    /// Free every canvas item, material, shader and sheet. The
    /// `CanvasItemMaterial`s and the textures go with the value.
    pub fn free(&mut self) {
        if self.items.is_empty() && self.sheets.is_empty() {
            return;
        }
        let vs = server();
        // SAFETY: see `frame`; each RID is freed once, as its map entry
        // leaves with it.
        unsafe {
            for (_, it) in self.items.drain() {
                vs.free_rid(it.item);
                if let Some(m) = it.material {
                    vs.free_rid(m);
                }
            }
            for (_, s) in self.sheets.drain() {
                vs.free_rid(s.item);
                vs.free_rid(s.canvas);
                vs.free_rid(s.viewport);
            }
            for s in self.shaders.iter_mut() {
                if let Some(rid) = s.take() {
                    vs.free_rid(rid);
                }
            }
        }
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        self.free();
    }
}
