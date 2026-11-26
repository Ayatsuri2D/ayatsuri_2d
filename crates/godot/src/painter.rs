//! The drawing: one `RenderingServer` canvas item per drawable, children
//! of the node's own canvas item so they move, scale and hide with it. The
//! items are made once and kept; a frame submits triangles again only for
//! the meshes that moved, and hands out draw indices only when the order
//! changed. A masked drawable sits inside a clip-only canvas group that
//! holds its masks' triangles, so the masks' alpha clips it.
//!
//! ```text
//! node canvas item
//! ├── item (mesh 4)                       draw index 0
//! ├── group (clip only: masks 7, 8)       draw index 1
//! │   └── item (mesh 5)
//! └── item (mesh 9)                       draw index 2
//! ```
//!
//! Which items to touch is decided in `plan`; this file only makes the
//! calls.

use std::collections::{HashMap, HashSet};

use godot::builtin::{Color, PackedColorArray, PackedInt32Array, PackedVector2Array, Rid, Vector2};
use godot::classes::canvas_item_material::BlendMode;
use godot::classes::rendering_server::CanvasGroupMode;
use godot::classes::{CanvasItemMaterial, RenderingServer, Texture2D};
use godot::global::godot_warn;
use godot::obj::{Gd, NewGd, Singleton};

use crate::plan::{Blend, Pixels, Plan, blend_of, inverted_mask};
use crate::runtime::{Drawable, Runtime};

/// A mesh's canvas items.
struct Item {
    /// The drawable's own triangles.
    item: Rid,
    /// The clip group its masks are drawn into, when it has any; the
    /// parent of `item`, and what takes the draw index.
    group: Option<Rid>,
}

impl Item {
    fn top(&self) -> Rid {
        self.group.unwrap_or(self.item)
    }
}

pub struct Painter {
    /// The node's canvas item, parent of everything here.
    parent: Rid,
    pixels: Pixels,
    items: HashMap<u32, Item>,
    /// The texture pages, by the runtime's texture index; `None` for a
    /// page that did not decode, drawn untextured.
    textures: Vec<Option<Gd<Texture2D>>>,
    /// Kept alive as long as the items that use their RIDs.
    add: Gd<CanvasItemMaterial>,
    mul: Gd<CanvasItemMaterial>,
    said_blend: bool,
    said_inverted: bool,
    // scratch, so a frame allocates nothing once these have grown
    points: Vec<Vector2>,
    uvs: Vec<Vector2>,
    colors: Vec<Color>,
    indices: Vec<i32>,
}

impl Painter {
    pub fn new(parent: Rid, pixels: Pixels, textures: Vec<Option<Gd<Texture2D>>>) -> Self {
        let mut add = CanvasItemMaterial::new_gd();
        add.set_blend_mode(BlendMode::ADD);
        let mut mul = CanvasItemMaterial::new_gd();
        mul.set_blend_mode(BlendMode::MUL);
        Self {
            parent,
            pixels,
            items: HashMap::new(),
            textures,
            add,
            mul,
            said_blend: false,
            said_inverted: false,
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
    /// front).
    pub fn frame(&mut self, rt: &Runtime, drawn: &[Drawable], plan: &Plan) {
        let mut rs = RenderingServer::singleton();
        let redraw: HashSet<u32> = plan.redraw.iter().copied().collect();
        for &m in &plan.hide {
            if let Some(it) = self.items.get(&m) {
                rs.canvas_item_set_visible(it.top(), false);
            }
        }
        for (at, d) in drawn.iter().enumerate() {
            let fresh = !self.items.contains_key(&d.mesh);
            let masked = !d.masks.is_empty() && !inverted_mask(d.flags);
            if fresh {
                self.make(&mut rs, rt, d, masked);
            }
            let Some(it) = self.items.get(&d.mesh) else { continue };
            let (item, group, top) = (it.item, it.group, it.top());
            if plan.reorder || fresh {
                rs.canvas_item_set_draw_index(top, at as i32);
                rs.canvas_item_set_visible(top, true);
            }
            if redraw.contains(&d.mesh) {
                rs.canvas_item_set_visible(top, true);
                self.triangles(&mut rs, item, d, d.opacity);
                if let Some(g) = group {
                    rs.canvas_item_clear(g);
                    for &mask in d.masks {
                        if let Some(geometry) = rt.mesh_geometry(mask) {
                            self.append(&mut rs, g, &geometry, 1.0);
                        }
                    }
                }
            }
        }
    }

    /// The items for a mesh seen for the first time: its own item, its
    /// blend material, and its clip group when it has masks.
    fn make(&mut self, rs: &mut Gd<RenderingServer>, rt: &Runtime, d: &Drawable, masked: bool) {
        let item = rs.canvas_item_create();
        let group = if masked {
            let g = rs.canvas_item_create();
            rs.canvas_item_set_parent(g, self.parent);
            rs.canvas_item_set_canvas_group_mode(g, CanvasGroupMode::CLIP_ONLY);
            rs.canvas_item_set_parent(item, g);
            Some(g)
        } else {
            rs.canvas_item_set_parent(item, self.parent);
            None
        };
        if !d.masks.is_empty() && inverted_mask(d.flags) && !self.said_inverted {
            self.said_inverted = true;
            godot_warn!(
                "Ayatsuri2D: mesh {} has an inverted mask, which this addon does not draw yet; it is drawn unmasked",
                d.mesh
            );
        }
        let (blend, advanced) = blend_of(d.flags, rt.mesh_blend(d.mesh));
        if advanced && !self.said_blend {
            self.said_blend = true;
            godot_warn!(
                "Ayatsuri2D: mesh {} uses a Cubism 5.3 blend mode the canvas cannot do; it is drawn with the plain blend instead",
                d.mesh
            );
        }
        match blend {
            Blend::Mix => {}
            Blend::Add => rs.canvas_item_set_material(item, self.add.get_rid()),
            Blend::Mul => rs.canvas_item_set_material(item, self.mul.get_rid()),
        }
        self.items.insert(d.mesh, Item { item, group });
    }

    /// Replace an item's triangles with a drawable's.
    fn triangles(&mut self, rs: &mut Gd<RenderingServer>, item: Rid, d: &Drawable, opacity: f32) {
        rs.canvas_item_clear(item);
        self.append(rs, item, d, opacity);
    }

    /// Add a drawable's triangles to an item, in pixels, with its page.
    fn append(&mut self, rs: &mut Gd<RenderingServer>, item: Rid, d: &Drawable, opacity: f32) {
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
        let indices = PackedInt32Array::from(self.indices.as_slice());
        let points = PackedVector2Array::from(self.points.as_slice());
        let uvs = PackedVector2Array::from(self.uvs.as_slice());
        let colors = PackedColorArray::from(self.colors.as_slice());
        let page =
            self.textures.get(d.texture as usize).and_then(|t| t.as_ref()).map(|t| t.get_rid());
        let call = rs.canvas_item_add_triangle_array_ex(item, &indices, &points, &colors).uvs(&uvs);
        match page {
            Some(rid) => call.texture(rid).done(),
            None => call.done(),
        }
    }

    /// Free every canvas item. The materials and textures go with the
    /// value.
    pub fn free(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let mut rs = RenderingServer::singleton();
        for (_, it) in self.items.drain() {
            rs.free_rid(it.item);
            if let Some(g) = it.group {
                rs.free_rid(g);
            }
        }
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        self.free();
    }
}
