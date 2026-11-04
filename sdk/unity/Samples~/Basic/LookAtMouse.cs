// Ayatsuri2D for Unity, Basic sample. Apache-2.0.
//
// Put this on a GameObject that has an AyatsuriModel. The head and eyes
// follow the mouse, and a click plays a motion from the "TapBody" group
// (or any motion, when the model has no such group).

using UnityEngine;

namespace Ayatsuri2D.Samples
{
    [RequireComponent(typeof(AyatsuriModel))]
    public sealed class LookAtMouse : MonoBehaviour
    {
        [Tooltip("How far the head turns at the screen's edge, in the parameter's units.")]
        public float headRange = 30f;

        [Tooltip("How quickly the gaze catches up with the mouse.")]
        public float smoothing = 8f;

        AyatsuriModel model;
        int angleX, angleY, bodyX, eyeX, eyeY;
        Vector2 look;

        void Start()
        {
            model = GetComponent<AyatsuriModel>();
            // look ids up once, then drive by index
            angleX = model.ParameterIndex("ParamAngleX");
            angleY = model.ParameterIndex("ParamAngleY");
            bodyX = model.ParameterIndex("ParamBodyAngleX");
            eyeX = model.ParameterIndex("ParamEyeBallX");
            eyeY = model.ParameterIndex("ParamEyeBallY");
        }

        void Update()
        {
            Vector2 mouse = Input.mousePosition;
            var target = new Vector2(mouse.x / Screen.width * 2f - 1f, mouse.y / Screen.height * 2f - 1f);
            target = Vector2.ClampMagnitude(target, 1f);
            look = Vector2.Lerp(look, target, 1f - Mathf.Exp(-smoothing * Time.deltaTime));

            model.SetParameter(angleX, look.x * headRange);
            model.SetParameter(angleY, look.y * headRange);
            model.SetParameter(bodyX, look.x * headRange / 3f);
            model.SetParameter(eyeX, look.x);
            model.SetParameter(eyeY, look.y);

            if (Input.GetMouseButtonDown(0))
            {
                string[] tap = model.MotionsInGroup("TapBody");
                if (tap.Length > 0) model.PlayMotion(tap[Random.Range(0, tap.Length)]);
                else if (model.MotionCount > 0) model.PlayMotion(model.MotionName(Random.Range(0, model.MotionCount)));
            }
        }
    }
}
