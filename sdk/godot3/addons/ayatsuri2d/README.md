# Ayatsuri2D for Godot 3

Load and play rigged 2D models in Godot 3.5 or 3.6: Ayatsuri2D's own
`.aom` files, and the `.moc3` bundles VTube Studio loads. One NativeScript
class, `AyatsuriModel`, draws the model on the 2D canvas; your scripts set
its parameters, play motions and expressions, and run physics.

The addon is a GDNative library written in Rust on the same runtime
library the Unity package and the Godot 4 addon use, so the three behave
the same. Its methods, properties and signal are the Godot 4 addon's, name
for name.

Runs on Windows (x64), Linux (x64 and ARM64) and macOS (universal). The
mobile targets the C library ships for (Android, iOS) are not in this
addon. Godot 4 has its own addon, `ayatsuri2d-godot-<version>.zip`.

## Install

Download `ayatsuri2d-godot3-<version>.zip` from the
[releases](https://github.com/Ayatsuri2D/ayatsuri_2d/releases) and unzip it
into your project, so that `addons/ayatsuri2d/ayatsuri2d.gdnlib` sits under
`res://`. The zip carries the library for every platform; a checkout of
this folder does not. There is no editor plugin to enable: a GDNative
library is loaded when a scene uses it.

## Bring in a model

- **An `.aom` file**: drop it anywhere in your project. It carries the rig,
  the texture pages, the motions, the expressions and the physics.
- **A `.moc3` bundle**: drop the whole folder (the `.model3.json`, the
  `.moc3`, the texture PNGs, and the motions, expressions and physics it
  names). Godot imports the PNGs as textures; the addon reads the rest as
  files.

Add a `Node2D` to a scene and, in the Inspector, set its **Script** to
`res://addons/ayatsuri2d/ayatsuri_model.gdns` (drag the file onto the
Script slot, or pick **Load** in the slot's menu). The node gains the
`AyatsuriModel` properties: set **Model Path** to the `.aom` or the
`.model3.json`. The model appears in the editor at its canvas size in
pixels, with the model's origin on the node; move, scale and rotate the
node to place it.

When you export the game, Godot copies only the files it knows. Add
`*.aom, *.moc3, *.model3.json, *.motion3.json, *.exp3.json, *.physics3.json`
to **Export > Resources > Filters to export non-resource files** so the
models travel with it. The PNGs of a bundle are imported textures and need
no filter.

## Drive it from a script

A GDScript cannot extend a NativeScript class, so put the script on a
parent node (or a sibling) and call the model through a reference:

```gdscript
extends Node2D

onready var model = $AyatsuriModel # a Node2D with ayatsuri_model.gdns on it
var angle_x: int

func _ready() -> void:
    angle_x = model.parameter_index("ParamAngleX") # look ids up once
    model.play_motion("idle", true, -1.0)
    model.connect("motion_event", self, "_on_motion_event")

func _process(_delta: float) -> void:
    # the mouse across the viewport turns the head from -30 to 30
    var r: Vector3 = model.parameter_range(angle_x) # x is the minimum, y the maximum
    var t := get_viewport().get_mouse_position().x / get_viewport_rect().size.x
    model.set_parameter(angle_x, lerp(r.x, r.y, clamp(t, 0.0, 1.0)))

func _input(event: InputEvent) -> void:
    if event is InputEventMouseButton and event.pressed:
        model.play_motion("tap", false, -1.0)

func _on_motion_event(motion: String, time: float, name: String) -> void:
    print(motion, " fired ", name, " at ", time)
```

To make a model from code, load the script and set it on a fresh node:

```gdscript
var model := Node2D.new()
model.set_script(load("res://addons/ayatsuri2d/ayatsuri_model.gdns"))
add_child(model)
model.load_file("res://models/hiyori/hiyori.model3.json")
```

A value set during the frame applies in this frame's `_process` of the
node, which runs the model and draws it. A motion writes over the
parameters it animates; to hold a parameter against one, set it every
frame.

What `AyatsuriModel` offers:

- **Loading**: `model_path` (loads when the node is ready, or at once when
  set on a node in the tree), `load_file(path)`, `unload()`, `is_loaded()`,
  `canvas_rect()`, `runtime_version()`.
- **Parameters**: `parameter_count()`, `parameter_index(id)`,
  `parameter_id(i)`, `parameter_name(i)`, `parameter_range(i)` (a
  `Vector3`: minimum, maximum, default), `get_parameter(i)`,
  `set_parameter(i, value)`, and `get_parameter_by_id(id)`,
  `set_parameter_by_id(id, value)`.
- **Parts**: `part_count()`, `part_index(id)`, `part_id(i)`,
  `set_part_opacity(i, value)`, `set_part_opacity_by_id(id, value)`.
- **Motions**: `motion_count()`, `motion_name(i)`, `motion_index(name)`,
  `play_motion(name, looping, fade_in)`, `stop_motion(fade_out)`,
  `playing_motion()`, `motion_finished()`, `get_motion_time()`,
  `set_motion_time(t)`, `set_motion_speed(speed)`, and the `motion_event`
  signal with the motion's name, its time in seconds and the event's text.
- **Expressions**: `expression_count()`, `expression_name(i)`,
  `set_expression(name, fade_in)`, `clear_expression(fade_out)`,
  `shown_expression()`.
- **Physics**: the `physics` property, `has_physics()`, the `wind`
  property (a `Vector2` in the units gravity has; down is `(0, -1)`),
  `reset_physics()`, and `time_scale` for how fast motions and physics
  run.

A fade of `-1.0` takes the fade the clip was authored with; `0.0` switches
at once.

## Units and drawing

The model's canvas is drawn at its size in pixels: a 2048 by 2048 canvas
covers 2048 by 2048 pixels of the node's local space, its origin (where
the rig's `0, 0` sits, often the middle of the canvas) on the node's own
position. Scale the node to size it. This is the Godot 4 addon's
convention, so a scene moves between the two unchanged.

Each drawable is one `VisualServer` canvas item under the node's own, so
the model moves, scales, hides and sorts with the node like any `Node2D`.
Normal, Additive and Multiply blending use `CanvasItemMaterial` blend
modes.

Godot 3 has no canvas groups, so masks work differently from the Godot 4
addon: the masks of a mesh are drawn in white into an offscreen viewport
(one per distinct set of masks, at most 1024 pixels on its longest side),
and the mesh is drawn with a small shader that keeps only what the masks
cover. Inverted masks (a mesh cut out where its masks are drawn) use the
same shader the other way round, so they are drawn here, unlike in the
first Godot 4 version.

Limits of this version:

- **Blend modes beyond Normal, Additive and Multiply** (the advanced
  blends moc3 5.3 adds) draw with the plain blend the mesh's flags name.
  The log says so once per model.
- **Masks are soft at the edge** when the canvas is larger than 1024
  pixels, since the mask sheet is scaled down to that; and a multiplied
  mesh's opacity cannot fade it (the multiply blend ignores alpha, as the
  Godot 4 addon's does).
- Parts drawn offscreen are drawn straight into the scene.
- GDNative loads the library once per editor session: after replacing the
  library file, restart the editor.

## Licence

This addon and the native library's C interface are Apache-2.0
(`LICENSE`, with the full text in `LICENSE-APACHE`). The runtime inside the
library is MIT (`LICENSE-MIT`), and its dependencies, among them the MIT
`gdnative` binding and the Godot headers it carries, are listed in
`THIRD-PARTY.md`. You can ship them in a closed game; keep the notices.

Ayatsuri2D is made separately from Live2D Inc. Live2D and Cubism are Live2D
Inc. trademarks; they are named only to say which files this addon reads.
