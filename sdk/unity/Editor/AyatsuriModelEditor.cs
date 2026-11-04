// Ayatsuri2D for Unity. Apache-2.0.

using UnityEditor;
using UnityEngine;

namespace Ayatsuri2D.Editor
{
    /// <summary>
    /// The model's settings, then a live view to try it with: parameter
    /// sliders, and in Play mode its motions and expressions. Slider values
    /// are a preview; scripts set the pose a game keeps.
    /// </summary>
    [CustomEditor(typeof(AyatsuriModel))]
    internal sealed class AyatsuriModelEditor : UnityEditor.Editor
    {
        bool showParameters = true;
        bool showMotions = true;
        bool showExpressions = true;
        string filter = "";

        public override bool RequiresConstantRepaint() => Application.isPlaying;

        public override void OnInspectorGUI()
        {
            DrawDefaultInspector();
            var model = (AyatsuriModel)target;
            if (!model.IsLoaded) return;

            EditorGUILayout.Space();
            showParameters = EditorGUILayout.Foldout(showParameters, $"Parameters ({model.ParameterCount})", true);
            if (showParameters)
            {
                filter = EditorGUILayout.TextField("Search", filter);
                bool changed = false;
                for (int i = 0; i < model.ParameterCount; i++)
                {
                    AyatsuriModel.ParameterInfo p = model.GetParameterInfo(i);
                    if (filter.Length > 0
                        && p.Id.IndexOf(filter, System.StringComparison.OrdinalIgnoreCase) < 0
                        && p.Name.IndexOf(filter, System.StringComparison.OrdinalIgnoreCase) < 0) continue;
                    float v = model.GetParameter(i);
                    float nv = EditorGUILayout.Slider(new GUIContent(p.Name, p.Id), v, p.Min, p.Max);
                    if (!Mathf.Approximately(nv, v))
                    {
                        model.SetParameter(i, nv);
                        changed = true;
                    }
                }
                // in Edit mode nothing updates on its own: ask for a frame
                if (changed && !Application.isPlaying) EditorApplication.QueuePlayerLoopUpdate();
            }

            if (!Application.isPlaying)
            {
                EditorGUILayout.HelpBox("Enter Play mode to try motions, expressions and physics.", MessageType.None);
                return;
            }

            showMotions = EditorGUILayout.Foldout(showMotions, $"Motions ({model.MotionCount})", true);
            if (showMotions)
            {
                for (int i = 0; i < model.MotionCount; i++)
                {
                    string name = model.MotionName(i);
                    using (new EditorGUILayout.HorizontalScope())
                    {
                        EditorGUILayout.LabelField(name);
                        if (GUILayout.Button("Play", GUILayout.Width(50))) model.PlayMotion(name);
                        if (GUILayout.Button("Loop", GUILayout.Width(50))) model.PlayMotion(name, true);
                    }
                }
                if (model.MotionCount > 0 && GUILayout.Button("Stop")) model.StopMotion();
            }

            showExpressions = EditorGUILayout.Foldout(showExpressions, $"Expressions ({model.ExpressionCount})", true);
            if (showExpressions)
            {
                for (int i = 0; i < model.ExpressionCount; i++)
                {
                    string name = model.ExpressionName(i);
                    if (GUILayout.Button(name)) model.SetExpression(name);
                }
                if (model.ExpressionCount > 0 && GUILayout.Button("Clear")) model.ClearExpression();
            }

            if (model.HasPhysics && GUILayout.Button("Reset physics")) model.ResetPhysics();
        }
    }
}
