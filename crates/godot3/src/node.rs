//! `AyatsuriModel`: the NativeScript a Godot 3 scene puts on a `Node2D`.
//! It loads a model, takes parameter and part values from scripts during
//! the frame, and in `_process` advances motions, expressions and physics
//! and draws the result. The API is the Godot 4 addon's, method for method,
//! so a script moves between the two with its calls unchanged.

use gdnative::api::{Engine, File, Image, ImageTexture, Node2D, Object, ResourceLoader, Texture};
use gdnative::core_types::{PoolArray, Rect2, Variant, VariantType, Vector2, Vector3};
use gdnative::export::hint::{EnumHint, StringHint};
use gdnative::export::{ClassBuilder, Export};
use gdnative::log::{godot_error, godot_warn};
use gdnative::object::ownership::Shared;
use gdnative::object::{Ref, TRef};
use gdnative::prelude::{NativeClass, methods};

use crate::painter::{Page, Painter};
use crate::plan::{self, Pixels, Plan};
use crate::runtime::{Runtime, Status};

/// The texture pages by the runtime's texture index; `None` where one did
/// not load.
type Pages = Vec<Option<Page>>;

/// A loaded model and everything the node keeps beside it.
struct Live {
    rt: Runtime,
    painter: Painter,
    /// Every parameter's value after the last update.
    values: Vec<f32>,
    /// What scripts set since the last update, flushed before it.
    set_idx: Vec<u32>,
    set_val: Vec<f32>,
    part_idx: Vec<u32>,
    part_val: Vec<f32>,
    part_count: u32,
    /// What the runtime was last told, so a setting is sent once.
    applied_physics: Option<bool>,
    applied_wind: Option<Vector2>,
    // scratch for the frame
    draw_buf: Vec<ayatsuri_capi::Drawable>,
    changes: Vec<u32>,
    last_order: Vec<u32>,
}

/// A rigged 2D model (Ayatsuri2D `.aom`, or a Cubism `.moc3` with its
/// `.model3.json`) drawn on the canvas at its canvas size in pixels, the
/// model's origin on the node. Set parameters from scripts, play motions
/// and expressions, run physics.
#[derive(NativeClass)]
#[inherit(Node2D)]
#[register_with(Self::register)]
pub struct AyatsuriModel {
    /// The model to load when the node is ready: `res://` or an absolute
    /// path to an `.aom`, a `.model3.json` or a bare `.moc3`.
    #[property(set = "Self::set_model_path", hint = "model_path_hint")]
    model_path: String,
    /// Run the pendulum rig when the model has one.
    #[property(default = true)]
    physics: bool,
    /// Wind on the rig, in the units gravity has (down is 0, -1).
    #[property]
    wind: Vector2,
    /// How fast motions and physics run; 1 is real time.
    #[property(default = 1.0)]
    time_scale: f32,
    live: Option<Live>,
}

/// The file picker filter of the `model_path` property.
fn model_path_hint() -> <String as Export>::Hint {
    StringHint::File(EnumHint::new(vec!["*.aom".into(), "*.model3.json".into(), "*.moc3".into()]))
}

#[methods]
impl AyatsuriModel {
    fn new(_owner: &Node2D) -> Self {
        Self {
            model_path: String::new(),
            physics: true,
            wind: Vector2::ZERO,
            time_scale: 1.0,
            live: None,
        }
    }

    /// The signal: a motion fired an event, with the motion's name, its
    /// time on the motion's clock in seconds, and the event's text.
    fn register(builder: &ClassBuilder<Self>) {
        builder
            .signal("motion_event")
            .with_param("motion", VariantType::GodotString)
            .with_param("time", VariantType::F64)
            .with_param("name", VariantType::GodotString)
            .done();
    }

    #[method]
    fn _ready(&mut self, #[base] owner: TRef<Node2D>) {
        if self.live.is_none() && !self.model_path.is_empty() {
            let path = self.model_path.clone();
            self.load_file(owner, path);
        }
    }

    #[method]
    fn _process(&mut self, #[base] owner: TRef<Node2D>, delta: f64) {
        // in the editor the model shows at its pose but does not run
        let dt = if Engine::godot_singleton().is_editor_hint() {
            0.0
        } else {
            (delta as f32 * self.time_scale).max(0.0)
        };
        self.step(owner, dt);
    }

    #[method]
    fn _notification(&mut self, what: i64) {
        if what == Object::NOTIFICATION_PREDELETE {
            self.unload();
        }
    }

    // ---- loading ----

    /// Set the path and, when the node is already in the tree, load it.
    fn set_model_path(&mut self, owner: TRef<Node2D>, path: String) {
        self.model_path = path.clone();
        if owner.is_inside_tree() {
            self.unload();
            if !path.is_empty() {
                self.load_file(owner, path);
            }
        }
    }

    /// Load a model from `res://` or an absolute path. An `.aom` carries
    /// everything; a `.model3.json` loads the `.moc3` it names with its
    /// textures, physics, motions and expressions; a bare `.moc3` loads
    /// without textures or clips. Returns false, with the reason in the
    /// log, when the file is not a model.
    #[method]
    pub fn load_file(&mut self, #[base] owner: TRef<Node2D>, path: String) -> bool {
        self.unload();
        if !Runtime::abi_matches() {
            godot_error!(
                "Ayatsuri2D: the runtime inside this addon speaks another ABI; rebuild it"
            );
            return false;
        }
        match self.load_from(owner, &path) {
            Ok(live) => {
                self.live = Some(live);
                self.step(owner, 0.0);
                true
            }
            Err(e) => {
                godot_error!("Ayatsuri2D: {path} did not load: {e}");
                false
            }
        }
    }

    fn load_from(&mut self, owner: TRef<Node2D>, path: &str) -> Result<Live, String> {
        let lower = path.to_lowercase();
        let (rt, textures) = if lower.ends_with(".model3.json") {
            load_bundle(path)?
        } else {
            let bytes = read_bytes(path)?;
            let rt = Runtime::load(&bytes)?;
            let textures = (0..rt.texture_count())
                .map(|i| rt.texture(i).and_then(|png| decode_png(png, i)))
                .collect();
            (rt, textures)
        };
        let n = rt.parameter_count() as usize;
        let (_, _, defaults) = rt.parameter_ranges();
        let pixels = Pixels::of(&rt.canvas());
        let painter = Painter::new(owner.get_canvas_item(), pixels, textures);
        Ok(Live {
            part_count: rt.part_count(),
            rt,
            painter,
            values: if defaults.len() == n { defaults } else { vec![0.0; n] },
            set_idx: Vec::new(),
            set_val: Vec::new(),
            part_idx: Vec::new(),
            part_val: Vec::new(),
            applied_physics: None,
            applied_wind: None,
            draw_buf: Vec::new(),
            changes: Vec::new(),
            last_order: Vec::new(),
        })
    }

    /// Free the model and its canvas items. The node stays, empty.
    #[method]
    pub fn unload(&mut self) {
        if let Some(mut live) = self.live.take() {
            live.painter.free();
        }
    }

    /// True once a model loaded.
    #[method]
    pub fn is_loaded(&self) -> bool {
        self.live.is_some()
    }

    /// The model's canvas as drawn on the node, in the node's pixels: its
    /// top left corner and its size. The model's origin is on the node, so
    /// the corner is at minus the origin.
    #[method]
    pub fn canvas_rect(&self) -> Rect2 {
        match &self.live {
            Some(l) => {
                let (x, y, w, h) = Pixels::of(&l.rt.canvas()).canvas_rect();
                Rect2 { position: Vector2::new(x, y), size: Vector2::new(w, h) }
            }
            None => Rect2 { position: Vector2::ZERO, size: Vector2::ZERO },
        }
    }

    /// The runtime's version string.
    #[method]
    pub fn runtime_version(&self) -> String {
        Runtime::version()
    }

    // ---- the frame ----

    fn step(&mut self, owner: TRef<Node2D>, dt: f32) {
        let physics = self.physics;
        let wind = self.wind;
        let Some(live) = self.live.as_mut() else { return };
        if live.applied_physics != Some(physics) {
            live.rt.set_physics(physics);
            live.applied_physics = Some(physics);
        }
        if live.applied_wind != Some(wind) {
            live.rt.set_wind(wind.x, wind.y);
            live.applied_wind = Some(wind);
        }
        if !live.set_idx.is_empty() {
            live.rt.set_parameters(&live.set_idx, &live.set_val);
            live.set_idx.clear();
            live.set_val.clear();
        }
        if !live.part_idx.is_empty() {
            live.rt.set_part_opacities(&live.part_idx, &live.part_val);
            live.part_idx.clear();
            live.part_val.clear();
        }
        let status = live.rt.update(dt);
        if status != Status::Ok {
            godot_warn!("Ayatsuri2D: update failed ({status:?}): {}", crate::runtime::last_error());
            return;
        }
        live.rt.get_parameters(&mut live.values);
        let events: Vec<(String, f32, String)> = live
            .rt
            .motion_events()
            .into_iter()
            .map(|e| (live.rt.motion_id(e.motion).unwrap_or_default(), e.time, e.name))
            .collect();

        // the drawing
        live.rt.mesh_changes(&mut live.changes);
        let drawn = live.rt.drawables(&mut live.draw_buf);
        let order: Vec<u32> = drawn.iter().map(|d| d.mesh).collect();
        let masks_of = |m: u32| -> Vec<u32> {
            drawn.iter().find(|d| d.mesh == m).map(|d| d.masks.to_vec()).unwrap_or_default()
        };
        let p: Plan =
            plan::plan(&order, masks_of, &live.changes, &live.painter.known(), &live.last_order);
        live.painter.frame(&live.rt, &drawn, &p, &live.changes);
        live.last_order = order;

        for (motion, time, name) in events {
            owner.emit_signal(
                "motion_event",
                &[Variant::new(motion), Variant::new(time), Variant::new(name)],
            );
        }
    }

    // ---- parameters ----

    /// How many parameters the model has.
    #[method]
    pub fn parameter_count(&self) -> i32 {
        self.live.as_ref().map_or(0, |l| l.rt.parameter_count() as i32)
    }

    /// The index of the parameter with this id (`ParamAngleX`), or -1.
    /// Look ids up once; drive by index.
    #[method]
    pub fn parameter_index(&self, id: String) -> i32 {
        self.live.as_ref().map_or(-1, |l| l.rt.parameter_index(&id))
    }

    /// Parameter `index`'s id, or an empty string past the count.
    #[method]
    pub fn parameter_id(&self, index: i32) -> String {
        let id = self.live.as_ref().and_then(|l| l.rt.parameter_id(u32::try_from(index).ok()?));
        id.unwrap_or_default()
    }

    /// Parameter `index`'s display name ("Angle X"), the id when the file
    /// names none.
    #[method]
    pub fn parameter_name(&self, index: i32) -> String {
        let name = self.live.as_ref().and_then(|l| l.rt.parameter_name(u32::try_from(index).ok()?));
        name.unwrap_or_default()
    }

    /// Parameter `index`'s minimum, maximum and default, as x, y and z.
    #[method]
    pub fn parameter_range(&self, index: i32) -> Vector3 {
        let Some(l) = &self.live else { return Vector3::ZERO };
        let Ok(i) = usize::try_from(index) else { return Vector3::ZERO };
        let (min, max, def) = l.rt.parameter_ranges();
        match (min.get(i), max.get(i), def.get(i)) {
            (Some(&a), Some(&b), Some(&c)) => Vector3::new(a, b, c),
            _ => Vector3::ZERO,
        }
    }

    /// A parameter's value after the last update: what motions,
    /// expressions and physics made of the pose.
    #[method]
    pub fn get_parameter(&self, index: i32) -> f32 {
        let i = usize::try_from(index).unwrap_or(usize::MAX);
        self.live.as_ref().and_then(|l| l.values.get(i).copied()).unwrap_or(0.0)
    }

    /// Set a parameter for the next update, clamped to its range. Set it
    /// every frame to hold it against a motion.
    #[method]
    pub fn set_parameter(&mut self, index: i32, value: f32) {
        let Some(l) = self.live.as_mut() else { return };
        if let Ok(i) = u32::try_from(index)
            && i < l.rt.parameter_count()
        {
            l.set_idx.push(i);
            l.set_val.push(value);
        }
    }

    /// `get_parameter` by id.
    #[method]
    pub fn get_parameter_by_id(&self, id: String) -> f32 {
        self.get_parameter(self.parameter_index(id))
    }

    /// `set_parameter` by id.
    #[method]
    pub fn set_parameter_by_id(&mut self, id: String, value: f32) {
        let i = self.parameter_index(id);
        self.set_parameter(i, value);
    }

    // ---- parts ----

    /// How many parts the model has.
    #[method]
    pub fn part_count(&self) -> i32 {
        self.live.as_ref().map_or(0, |l| l.part_count as i32)
    }

    /// The index of the part with this id, or -1.
    #[method]
    pub fn part_index(&self, id: String) -> i32 {
        let Some(l) = &self.live else { return -1 };
        (0..l.part_count)
            .find(|&i| l.rt.part_id(i).as_deref() == Some(id.as_str()))
            .map_or(-1, |i| i as i32)
    }

    /// Part `index`'s id, or an empty string past the count.
    #[method]
    pub fn part_id(&self, index: i32) -> String {
        let id = self.live.as_ref().and_then(|l| l.rt.part_id(u32::try_from(index).ok()?));
        id.unwrap_or_default()
    }

    /// Fade a part (0 to 1) from the next update on.
    #[method]
    pub fn set_part_opacity(&mut self, index: i32, opacity: f32) {
        let Some(l) = self.live.as_mut() else { return };
        if let Ok(i) = u32::try_from(index)
            && i < l.part_count
        {
            l.part_idx.push(i);
            l.part_val.push(opacity);
        }
    }

    /// `set_part_opacity` by id.
    #[method]
    pub fn set_part_opacity_by_id(&mut self, id: String, opacity: f32) {
        let i = self.part_index(id);
        self.set_part_opacity(i, opacity);
    }

    // ---- motions ----

    /// How many motions the model carries.
    #[method]
    pub fn motion_count(&self) -> i32 {
        self.live.as_ref().map_or(0, |l| l.rt.motion_count() as i32)
    }

    /// Motion `index`'s name, or an empty string.
    #[method]
    pub fn motion_name(&self, index: i32) -> String {
        let name = self.live.as_ref().and_then(|l| l.rt.motion_id(u32::try_from(index).ok()?));
        name.unwrap_or_default()
    }

    /// The index of the motion with this name, or -1.
    #[method]
    pub fn motion_index(&self, name: String) -> i32 {
        self.live.as_ref().map_or(-1, |l| l.rt.motion_index(&name))
    }

    /// Start a motion from its beginning. Whatever plays fades out; this
    /// one fades in over `fade_in` seconds (below zero: its authored fade;
    /// zero: at once). `looping` repeats it. Returns false when there is
    /// no such motion.
    #[method]
    pub fn play_motion(&mut self, name: String, looping: bool, fade_in: f32) -> bool {
        let Some(l) = &self.live else { return false };
        match u32::try_from(l.rt.motion_index(&name)) {
            Ok(i) => l.rt.motion_play(i, looping, fade_in) == Status::Ok,
            Err(_) => false,
        }
    }

    /// Fade every playing motion out over `fade_out` seconds (below zero:
    /// its own fade; zero: at once).
    #[method]
    pub fn stop_motion(&mut self, fade_out: f32) {
        if let Some(l) = &self.live {
            l.rt.motion_stop(fade_out);
        }
    }

    /// The motion in front, by name, or an empty string.
    #[method]
    pub fn playing_motion(&self) -> String {
        let name = self
            .live
            .as_ref()
            .and_then(|l| l.rt.motion_id(u32::try_from(l.rt.motion_playing()).ok()?));
        name.unwrap_or_default()
    }

    /// True when the motion in front played once through and holds its
    /// last frame.
    #[method]
    pub fn motion_finished(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.rt.motion_finished())
    }

    /// Where the motion in front is on its clock, in seconds; -1 when
    /// nothing plays.
    #[method]
    pub fn get_motion_time(&self) -> f32 {
        self.live.as_ref().map_or(-1.0, |l| l.rt.motion_time())
    }

    /// Put the motion in front at `time` seconds on its clock.
    #[method]
    pub fn set_motion_time(&mut self, time: f32) {
        if let Some(l) = &self.live {
            l.rt.motion_seek(time);
        }
    }

    /// Play the motion in front at `speed` times its rate (1 is as
    /// authored).
    #[method]
    pub fn set_motion_speed(&mut self, speed: f32) {
        if let Some(l) = &self.live {
            l.rt.motion_set_speed(speed);
        }
    }

    // ---- expressions ----

    /// How many expressions the model carries.
    #[method]
    pub fn expression_count(&self) -> i32 {
        self.live.as_ref().map_or(0, |l| l.rt.expression_count() as i32)
    }

    /// Expression `index`'s name, or an empty string.
    #[method]
    pub fn expression_name(&self, index: i32) -> String {
        let name = self.live.as_ref().and_then(|l| l.rt.expression_id(u32::try_from(index).ok()?));
        name.unwrap_or_default()
    }

    /// Show an expression; the one shown before fades out while this one
    /// fades in over `fade_in` seconds (below zero: its authored fade).
    /// Returns false when there is no such expression.
    #[method]
    pub fn set_expression(&mut self, name: String, fade_in: f32) -> bool {
        let Some(l) = &self.live else { return false };
        match u32::try_from(l.rt.expression_index(&name)) {
            Ok(i) => l.rt.expression_set(i, fade_in) == Status::Ok,
            Err(_) => false,
        }
    }

    /// Fade the shown expression out over `fade_out` seconds.
    #[method]
    pub fn clear_expression(&mut self, fade_out: f32) {
        if let Some(l) = &self.live {
            l.rt.expression_clear(fade_out);
        }
    }

    /// The expression shown, by name, or an empty string.
    #[method]
    pub fn shown_expression(&self) -> String {
        let name = self
            .live
            .as_ref()
            .and_then(|l| l.rt.expression_id(u32::try_from(l.rt.expression_shown()).ok()?));
        name.unwrap_or_default()
    }

    // ---- physics ----

    /// True when the model carries a pendulum rig.
    #[method]
    pub fn has_physics(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.rt.has_physics())
    }

    /// Put every pendulum back at rest, as after a scene cut.
    #[method]
    pub fn reset_physics(&mut self) {
        if let Some(l) = &self.live {
            l.rt.reset_physics();
        }
    }
}

// ---- files ----

/// The `.moc3` a `model3.json` names, with the files beside it: the
/// texture pages as Godot resources, and the physics, motions and
/// expressions handed to the runtime.
fn load_bundle(path: &str) -> Result<(Runtime, Pages), String> {
    let text = read_text(path)?;
    let bundle = plan::parse_model3(&text)?;
    let dir = plan::dir_of(path);
    let rt = Runtime::load(&read_bytes(&plan::join(dir, &bundle.moc))?)?;
    let textures = bundle
        .textures
        .iter()
        .map(|t| {
            let p = plan::join(dir, t);
            let tex = load_texture(&p);
            if tex.is_none() {
                godot_warn!("Ayatsuri2D: {path} names a texture Godot cannot load: {p}");
            }
            tex
        })
        .collect();
    if let Some(p) = &bundle.physics {
        match read_bytes(&plan::join(dir, p)).and_then(|json| rt.add_physics3(&json)) {
            Ok(()) => {}
            Err(e) => godot_warn!("Ayatsuri2D: the physics of {path} did not load: {e}"),
        }
    }
    for (name, _group, file) in &bundle.motions {
        match read_bytes(&plan::join(dir, file)).and_then(|json| rt.add_motion3(name, &json)) {
            Ok(_) => {}
            Err(e) => godot_warn!("Ayatsuri2D: motion {name} did not load: {e}"),
        }
    }
    for (name, file) in &bundle.expressions {
        match read_bytes(&plan::join(dir, file)).and_then(|json| rt.add_expression3(name, &json)) {
            Ok(_) => {}
            Err(e) => godot_warn!("Ayatsuri2D: expression {name} did not load: {e}"),
        }
    }
    Ok((rt, textures))
}

/// A file's bytes through Godot, so `res://` and `user://` work, in the
/// editor and in an exported game alike.
fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    let file = File::new();
    file.open(path, File::READ).map_err(|e| format!("{path}: {e:?}"))?;
    let bytes = file.get_buffer(file.get_len());
    file.close();
    Ok(bytes.to_vec())
}

fn read_text(path: &str) -> Result<String, String> {
    let bytes = read_bytes(path)?;
    String::from_utf8(bytes).map_err(|e| format!("{path}: {e}"))
}

/// A texture page from PNG bytes, as the `.aom` stores them: filtered,
/// with mipmaps, not repeating.
fn decode_png(png: &[u8], page: u32) -> Option<Page> {
    let image = Image::new();
    if image.load_png_from_buffer(PoolArray::from_slice(png)).is_err() {
        godot_warn!("Ayatsuri2D: texture page {page} did not decode as PNG");
        return None;
    }
    let texture = ImageTexture::new();
    texture.create_from_image(image.into_shared(), Texture::FLAG_FILTER | Texture::FLAG_MIPMAPS);
    let rid = texture.get_rid();
    Some(Page { _keep: texture.into_shared().upcast(), rid })
}

/// A texture page of a moc3 bundle: the PNG beside the model, loaded as
/// the resource Godot imported from it.
fn load_texture(path: &str) -> Option<Page> {
    let res: Ref<Texture, Shared> =
        ResourceLoader::godot_singleton().load(path, "Texture", false)?.cast::<Texture>()?;
    // SAFETY: the resource was handed out by the loader on this thread a
    // moment ago and `res` keeps it alive while its RID is read.
    let rid = unsafe { res.assume_safe() }.get_rid();
    Some(Page { _keep: res, rid })
}
