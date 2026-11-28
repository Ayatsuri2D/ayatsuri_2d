# Ayatsuri2D for Godot 4

Load and play rigged 2D models in Godot 4.2 or newer: Ayatsuri2D's own
`.aom` files, and the `.moc3` bundles VTube Studio loads. One node,
`AyatsuriModel`, draws the model on the 2D canvas; your scripts set its
parameters, play motions and expressions, and run physics.

The addon is a GDExtension written in Rust on the same runtime library the
Unity package uses, so the two behave the same.

Runs on Windows (x64), Linux (x64 and ARM64) and macOS (universal). The
mobile targets the C library ships for (Android, iOS) are not in this first
version of the addon.

## Install

Download `ayatsuri2d-godot-<version>.zip` from the
[releases](https://github.com/Ayatsuri2D/ayatsuri_2d/releases) and unzip it
into your project, so that `addons/ayatsuri2d/ayatsuri2d.gdextension` sits
under `res://`. Godot picks the extension up on the next scan; restart the
editor if the `AyatsuriModel` node does not appear in **Add Node**. The zip
carries the library for every platform; a checkout of this folder does not.

## Bring in a model

- **An `.aom` file**: drop it anywhere in your project. It carries the rig,
  the texture pages, the motions, the expressions and the physics.
- **A `.moc3` bundle**: drop the whole folder (the `.model3.json`, the
  `.moc3`, the texture PNGs, and the motions, expressions and physics it
  names). Godot imports the PNGs as textures; the addon reads the rest as
  files.

Add an `AyatsuriModel` node to a scene and set its **Model Path** to the
`.aom` or the `.model3.json`. The model appears in the editor at its canvas
size in pixels, with the model's origin on the node; move, scale and
rotate the node to place it.

When you export the game, Godot copies only the files it knows. Add
`*.aom, *.moc3, *.model3.json, *.motion3.json, *.exp3.json, *.physics3.json`
to **Export > Resources > Filters to export non-resource files** so the
models travel with it. The PNGs of a bundle are imported textures and need
no filter.

## Drive it from a script

```gdscript
extends AyatsuriModel

var angle_x: int

func _ready() -> void:
    angle_x = parameter_index("ParamAngleX") # look ids up once
    play_motion("idle", true, -1.0)
    motion_event.connect(func(motion, time, name):
        print(motion, " fired ", name, " at ", time))

func _process(_delta: float) -> void:
    # the mouse across the viewport turns the head from -30 to 30
    var r := parameter_range(angle_x) # x is the minimum, y the maximum
    var t := get_viewport().get_mouse_position().x / get_viewport_rect().size.x
    set_parameter(angle_x, lerp(r.x, r.y, clamp(t, 0.0, 1.0)))

func _input(event: InputEvent) -> void:
    if event is InputEventMouseButton and event.pressed:
        play_motion("tap", false, -1.0)
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
- **Breath**: the `breath` property (off by default). While on, the
  standard head, body and breath parameters (`ParamAngleX`,
  `ParamAngleY`, `ParamAngleZ`, `ParamBodyAngleX` and `ParamBreath`,
  those the model has) swing on slow sine waves on top of whatever your
  script and the playing motion set, so an idle model does not stand
  perfectly still; physics reacts to the swing. Off, nothing is added
  and the parameters rest where they were.
- **Follow**: the `follow` property, Off (the default), Mouse or Target.
  Mouse turns the model to face the pointer over the node; Target turns
  it to `follow_target`, a `Vector2` in the node's local pixels (the
  canvas's centre is straight ahead, its edges the furthest turn). The
  look eases towards the point by `1 - exp(-dt * follow_ease)` of the
  gap each frame (`follow_ease` is 8 by default, 0 snaps) and goes to the
  standard parameters the model has: `ParamAngleX` and `ParamAngleY` (30
  degrees at the edge), `ParamBodyAngleX` and `ParamBodyAngleY` (10),
  `ParamEyeBallX` and `ParamEyeBallY` (1), all scaled by
  `follow_strength` (1 by default). `look_at_point(point)` sets
  `follow_target` and switches `follow` to Target in one call.

A fade of `-1.0` takes the fade the clip was authored with; `0.0` switches
at once.

## Units and drawing

The model's canvas is drawn at its size in pixels: a 2048 by 2048 canvas
covers 2048 by 2048 pixels of the node's local space, its origin (where
the rig's `0, 0` sits, often the middle of the canvas) on the node's own
position. Scale the node to size it.

Each drawable is one `RenderingServer` canvas item under the node's own,
so the model moves, scales, hides and sorts with the node like any
`Node2D`. Normal, Additive and Multiply blending use `CanvasItemMaterial`
blend modes. A mask becomes a clip-only canvas group that holds the
masks' triangles, so their alpha clips the mesh.

Not drawn yet in this first version:

- **Inverted masks** (a mesh cut out where its masks are drawn) are drawn
  without their masks. The log says so once per model.
- **Blend modes beyond Normal, Additive and Multiply** (the advanced
  blends moc3 5.3 adds) draw with the plain blend the mesh's flags name.
  The log says so once per model.
- Parts drawn offscreen are drawn straight into the scene.

## Godot 3

This addon does not run in Godot 3: it is a GDExtension, which Godot 3
does not load. Godot 3.5 and 3.6 have their own addon,
`ayatsuri2d-godot3-<version>.zip` in the same release, a GDNative library
with the same methods, properties and signal (a NativeScript on a `Node2D`
instead of a node class; see its README). The C library and its header,
`ayatsuri.h`, remain in every release for anyone who wants to bind it by
hand anywhere else that calls C.

## Licence

This addon and the native library's C interface are Apache-2.0
(`LICENSE`, with the full text in `LICENSE-APACHE`). The runtime inside the
library is MIT (`LICENSE-MIT`), and its dependencies are listed in
`THIRD-PARTY.md`. You can ship them in a
closed game; keep the notices.

Ayatsuri2D is made separately from Live2D Inc. Live2D and Cubism are Live2D
Inc. trademarks; they are named only to say which files this addon reads.
