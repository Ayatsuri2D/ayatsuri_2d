# Ayatsuri2D for Unity

Load and play rigged 2D models in Unity 2022.3 or newer: Ayatsuri2D's own
`.aom` files, and the `.moc3` bundles VTube Studio loads. Set parameters
from your scripts, play motions and expressions, run physics, and the model
draws itself with its masks and blend modes, in the Built-in Render
Pipeline or URP.

Runs on Windows, macOS, Linux, Android (ARM64) and iOS.

## Install

Download `ayatsuri2d-unity-<version>.tgz` from the
[releases](https://github.com/Ayatsuri2D/ayatsuri_2d/releases), then in
Unity open **Window > Package Manager**, click **+**, choose
**Add package from tarball...** and pick the file. The tarball carries the
native library for every platform; a checkout of this folder does not.

## Bring in a model

- **An `.aom` file**: drop it into your Assets. It imports as a model you
  can drag straight into a scene, with its texture pages inside.
- **A `.moc3` bundle**: drop the whole folder (the `.model3.json`, the
  `.moc3`, the textures, and the motions, expressions and physics it
  names). Two assets appear beside the `.model3.json`: `name.asset` holds
  the model, `name.prefab` is ready for a scene. Changing a file of the
  bundle rebuilds the asset; the prefab is made once, so your changes to
  it are kept.

Select the model in a scene to try it: the Inspector has a slider for every
parameter, and in Play mode a button for every motion and expression.

## Drive it from a script

```csharp
using Ayatsuri2D;
using UnityEngine;

public class Wave : MonoBehaviour
{
    AyatsuriModel model;
    int angleX;

    void Start()
    {
        model = GetComponent<AyatsuriModel>();
        angleX = model.ParameterIndex("ParamAngleX"); // look ids up once
        model.PlayMotion("idle", loop: true);
        model.MotionEvent += (motion, time, name) => Debug.Log($"{motion} fired {name}");
    }

    void Update()
    {
        // values set in Update apply in this frame's LateUpdate
        model.SetParameter(angleX, Mathf.Sin(Time.time) * 30f);
    }
}
```

What `AyatsuriModel` offers:

- **Parameters**: `ParameterCount`, `GetParameterInfo`, `ParameterIndex`,
  `GetParameter`, `SetParameter`, `ParameterKeys`, and
  `GroupParameters("EyeBlink")` or `("LipSync")` for the parameters a blink
  or a lip sync should drive.
- **Parts**: `PartIndex`, `SetPartOpacity`.
- **Motions**: `PlayMotion`, `StopMotion`, `PlayingMotion`,
  `MotionFinished`, `MotionTime`, `SetMotionSpeed`, `MotionInfo`,
  `MotionsInGroup("Idle")`, and the `MotionEvent` event.
- **Expressions**: `SetExpression`, `ClearExpression`, `ShownExpression`.
- **Physics**: `Physics` on or off, `Wind`, `ResetPhysics`, `TimeScale`.

A motion writes over the parameters it animates. To hold a parameter
against one, set it every frame.

Everything else in the C library is in `Ayatsuri2D.Native.Ay2d`, with the
same names as `ayatsuri.h`; `AyatsuriModel.Handle` is the model to call it
on.

## Units and drawing

One Unity unit is one model unit, the canvas's `PixelsPerUnit` pixels; the
model's origin sits on the GameObject. Scale the GameObject to size it.

Each model is one mesh and one renderer, drawn in the transparent queue.
Masks use the stencil buffer, so the camera's target needs one (Unity's
default targets have it; a Render Texture needs a depth format with
stencil, such as 24 bits).

Not drawn yet:

- Masks are hard-edged at half the mask's alpha.
- Blend modes beyond Normal, Additive and Multiply (from moc3 5.3) draw
  as Normal.
- Parts drawn offscreen are drawn straight into the scene.

## Sample

**Package Manager > Ayatsuri2D > Samples > Basic** adds `LookAtMouse`: put
it on a model and the head and eyes follow the mouse, and a click plays a
motion. It reads the mouse through the old Input Manager; with only the
new Input System on, switch **Active Input Handling** to **Both**.

## Licence

This package and the native library's C interface are Apache-2.0
(`LICENSE`). The runtime inside the library is MIT (`LICENSE-MIT`), and its
dependencies are listed in `THIRD-PARTY.md`. You can ship them in a closed
game; keep the notices.

Ayatsuri2D is made separately from Live2D Inc. Live2D and Cubism are Live2D
Inc. trademarks; they are named only to say which files this package reads.
