// Ayatsuri2D for Unity. Apache-2.0.

using System;
using System.Collections.Generic;
using Ayatsuri2D.Native;
using UnityEngine;

namespace Ayatsuri2D
{
    /// <summary>
    /// A live model in the scene. It loads an <see cref="AyatsuriModelAsset"/>,
    /// takes parameter values from scripts during Update, and in LateUpdate
    /// advances motions, expressions and physics and draws the result.
    /// </summary>
    /// <example>
    /// <code>
    /// var model = GetComponent&lt;AyatsuriModel&gt;();
    /// int angleX = model.ParameterIndex("ParamAngleX");
    /// model.SetParameter(angleX, 15f);
    /// model.PlayMotion("idle", loop: true);
    /// </code>
    /// </example>
    [ExecuteAlways]
    [DisallowMultipleComponent]
    [RequireComponent(typeof(MeshFilter), typeof(MeshRenderer))]
    [AddComponentMenu("Ayatsuri2D/Ayatsuri Model")]
    public sealed unsafe class AyatsuriModel : MonoBehaviour
    {
        /// <summary>One parameter's facts, from the model.</summary>
        public readonly struct ParameterInfo
        {
            public readonly string Id;
            public readonly string Name;
            public readonly float Min;
            public readonly float Max;
            public readonly float Default;
            /// <summary>Wraps around its range instead of clamping.</summary>
            public readonly bool Repeat;

            internal ParameterInfo(string id, string name, float min, float max, float def, bool repeat)
            {
                Id = id;
                Name = name;
                Min = min;
                Max = max;
                Default = def;
                Repeat = repeat;
            }
        }

        [SerializeField, Tooltip("The imported model to load.")]
        AyatsuriModelAsset asset;

        [SerializeField, Tooltip("Run the pendulum rig when the model has one.")]
        bool physics = true;

        [SerializeField, Tooltip("Wind on the rig, in the units gravity has (down is 0, -1).")]
        Vector2 wind;

        [SerializeField, Tooltip("A motion to loop from the start, by name. Empty plays none.")]
        string idleMotion = "";

        [SerializeField, Min(0f), Tooltip("How fast motions and physics run; 1 is real time.")]
        float timeScale = 1f;

        /// <summary>A motion fired an event: the motion's name, its time on
        /// the motion's clock, and the event's text.</summary>
        public event Action<string, float, string> MotionEvent;

        IntPtr handle;
        ModelMesh mesh;
        Bounds bounds;
        ParameterInfo[] parameters = Array.Empty<ParameterInfo>();
        float[] values = Array.Empty<float>();
        readonly Dictionary<string, int> parameterIds = new Dictionary<string, int>();
        readonly Dictionary<string, int> partIds = new Dictionary<string, int>();
        readonly List<uint> setIndices = new List<uint>();
        readonly List<float> setValues = new List<float>();
        readonly List<uint> partIndices = new List<uint>();
        readonly List<float> partValues = new List<float>();
        FiredEvent[] events = new FiredEvent[8];
        int partCount;
        bool appliedPhysics;
        Vector2 appliedWind;

        /// <summary>The model this component plays. Setting it reloads.</summary>
        public AyatsuriModelAsset Asset
        {
            get => asset;
            set
            {
                asset = value;
                Reload();
            }
        }

        /// <summary>For importers: set the asset without loading it, so no
        /// runtime mesh ends up inside the imported prefab.</summary>
        internal void Assign(AyatsuriModelAsset a) => asset = a;

        /// <summary>True once the asset loaded.</summary>
        public bool IsLoaded => handle != IntPtr.Zero;

        /// <summary>The native handle, for calls on <see cref="Ay2d"/> this
        /// component does not wrap. It dies when the component is disabled.</summary>
        public IntPtr Handle => handle;

        /// <summary>Run the pendulum rig (when the model has one).</summary>
        public bool Physics
        {
            get => physics;
            set => physics = value;
        }

        /// <summary>True when the model carries a pendulum rig.</summary>
        public bool HasPhysics => IsLoaded && Ay2d.ay2d_has_physics(handle) != 0;

        public Vector2 Wind
        {
            get => wind;
            set => wind = value;
        }

        public float TimeScale
        {
            get => timeScale;
            set => timeScale = Mathf.Max(0f, value);
        }

        // ---- lifetime ----

        void OnEnable() => Load();

        void OnDisable() => Unload();

        void OnDestroy() => Unload();

#if UNITY_EDITOR
        void OnValidate()
        {
            if (isActiveAndEnabled && !Application.isPlaying) UnityEditor.EditorApplication.delayCall += () =>
            {
                if (this != null && isActiveAndEnabled) Reload();
            };
        }
#endif

        void Reload()
        {
            Unload();
            if (isActiveAndEnabled) Load();
        }

        void Load()
        {
            if (handle != IntPtr.Zero || asset == null || asset.model == null || asset.model.Length == 0) return;
            if (Ay2d.ay2d_abi_version() != Ay2d.AbiVersion)
            {
                Debug.LogError($"Ayatsuri2D: the native library speaks ABI {Ay2d.ay2d_abi_version()}, this package {Ay2d.AbiVersion}. Update the package.", this);
                return;
            }

            fixed (byte* b = asset.model) handle = Ay2d.ay2d_load(b, (UIntPtr)asset.model.Length);
            if (handle == IntPtr.Zero)
            {
                Debug.LogError($"Ayatsuri2D: {asset.name} did not load: {Ay2d.LastError()}", this);
                return;
            }

            if (asset.physics != null && asset.physics.Length > 0)
                fixed (byte* j = asset.physics)
                    if (Ay2d.ay2d_add_physics3(handle, j, (UIntPtr)asset.physics.Length) != Status.Ok)
                        Debug.LogWarning($"Ayatsuri2D: {asset.name}'s physics did not load: {Ay2d.LastError()}", this);
            foreach (AyatsuriModelAsset.Clip c in asset.motions) AddClip(c, true);
            foreach (AyatsuriModelAsset.Clip c in asset.expressions) AddClip(c, false);

            ReadParameters();
            ReadParts();

            CanvasInfo canvas;
            Ay2d.ay2d_canvas(handle, &canvas);
            float ppu = Mathf.Max(canvas.pixelsPerUnit, 1e-6f);
            // px = unit * ppu + origin, with y turned up for Unity
            var min = new Vector2(-canvas.originX / ppu, -(canvas.height - canvas.originY) / ppu);
            var max = new Vector2((canvas.width - canvas.originX) / ppu, canvas.originY / ppu);
            bounds = new Bounds((min + max) / 2f, new Vector3(max.x - min.x, max.y - min.y, 0.01f));
            // meshes may swing past the canvas; leave them room before culling
            bounds.Expand(new Vector3(bounds.size.x, bounds.size.y, 0f));

            mesh = new ModelMesh(asset.name);
            GetComponent<MeshFilter>().sharedMesh = mesh.Mesh;
            appliedPhysics = !physics;
            appliedWind = new Vector2(float.NaN, float.NaN);

            if (Application.isPlaying && !string.IsNullOrEmpty(idleMotion)) PlayMotion(idleMotion, true, 0f);
            Step(0f);
        }

        void AddClip(AyatsuriModelAsset.Clip c, bool motion)
        {
            if (c.json == null || c.json.Length == 0) return;
            byte[] name = Ay2d.Utf8(c.name ?? "");
            int i;
            fixed (byte* n = name)
            fixed (byte* j = c.json)
                i = motion
                    ? Ay2d.ay2d_add_motion3(handle, n, j, (UIntPtr)c.json.Length)
                    : Ay2d.ay2d_add_expression3(handle, n, j, (UIntPtr)c.json.Length);
            if (i < 0) Debug.LogWarning($"Ayatsuri2D: {(motion ? "motion" : "expression")} {c.name} did not load: {Ay2d.LastError()}", this);
        }

        void Unload()
        {
            if (mesh != null)
            {
                var filter = GetComponent<MeshFilter>();
                if (filter != null && filter.sharedMesh == mesh.Mesh) filter.sharedMesh = null;
                mesh.Dispose();
                mesh = null;
            }
            if (handle != IntPtr.Zero)
            {
                Ay2d.ay2d_free(handle);
                handle = IntPtr.Zero;
            }
            parameters = Array.Empty<ParameterInfo>();
            values = Array.Empty<float>();
            parameterIds.Clear();
            partIds.Clear();
            partCount = 0;
            setIndices.Clear();
            setValues.Clear();
            partIndices.Clear();
            partValues.Clear();
        }

        // ---- the frame ----

        void LateUpdate()
        {
            if (!IsLoaded) return;
            Step(Application.isPlaying ? Time.deltaTime * timeScale : 0f);
        }

        void Step(float dt)
        {
            if (physics != appliedPhysics)
            {
                Ay2d.ay2d_set_physics(handle, physics ? 1 : 0);
                appliedPhysics = physics;
            }
            if (wind != appliedWind)
            {
                Ay2d.ay2d_set_wind(handle, wind.x, wind.y);
                appliedWind = wind;
            }
            Flush(setIndices, setValues, true);
            Flush(partIndices, partValues, false);

            Status s = Ay2d.ay2d_update(handle, dt);
            if (s != Status.Ok)
            {
                Debug.LogWarning($"Ayatsuri2D: update failed ({s}): {Ay2d.LastError()}", this);
                return;
            }
            fixed (float* v = values) Ay2d.ay2d_get_parameters(handle, v, (UIntPtr)values.Length);
            FireEvents();
            mesh.Build(handle, asset.textures, GetComponent<MeshRenderer>(), bounds);
        }

        void Flush(List<uint> idx, List<float> val, bool parameter)
        {
            if (idx.Count == 0) return;
            uint[] i = idx.ToArray();
            float[] v = val.ToArray();
            fixed (uint* ip = i)
            fixed (float* vp = v)
            {
                if (parameter) Ay2d.ay2d_set_parameters(handle, ip, vp, (UIntPtr)i.Length);
                else Ay2d.ay2d_set_part_opacities(handle, ip, vp, (UIntPtr)i.Length);
            }
            idx.Clear();
            val.Clear();
        }

        void FireEvents()
        {
            uint n = Ay2d.ay2d_motion_events(handle, null, UIntPtr.Zero);
            if (n == 0 || MotionEvent == null) return;
            if (events.Length < n) events = new FiredEvent[n];
            fixed (FiredEvent* e = events) n = Ay2d.ay2d_motion_events(handle, e, (UIntPtr)events.Length);
            for (int k = 0; k < n; k++)
                MotionEvent(MotionName((int)events[k].motion), events[k].time, Ay2d.Str(events[k].name));
        }

        // ---- parameters ----

        void ReadParameters()
        {
            int n = (int)Ay2d.ay2d_parameter_count(handle);
            var min = new float[n];
            var max = new float[n];
            var def = new float[n];
            fixed (float* a = min)
            fixed (float* b = max)
            fixed (float* c = def)
                Ay2d.ay2d_parameter_ranges(handle, a, b, c, (UIntPtr)n);
            parameters = new ParameterInfo[n];
            values = new float[n];
            for (uint i = 0; i < n; i++)
            {
                string id = Ay2d.Str(Ay2d.ay2d_parameter_id(handle, i)) ?? "";
                parameters[i] = new ParameterInfo(id, Ay2d.Str(Ay2d.ay2d_parameter_name(handle, i)) ?? id,
                    min[i], max[i], def[i], Ay2d.ay2d_parameter_repeat(handle, i) != 0);
                values[i] = def[i];
                parameterIds[id] = (int)i;
            }
        }

        /// <summary>How many parameters the model has.</summary>
        public int ParameterCount => parameters.Length;

        /// <summary>Parameter <paramref name="index"/>'s id, name, range and default.</summary>
        public ParameterInfo GetParameterInfo(int index) => parameters[index];

        /// <summary>The index of the parameter with this id
        /// ("ParamAngleX"), or -1. Look ids up once; drive by index.</summary>
        public int ParameterIndex(string id) => id != null && parameterIds.TryGetValue(id, out int i) ? i : -1;

        /// <summary>A parameter's value after the last update: what
        /// motions, expressions and physics made of the pose.</summary>
        public float GetParameter(int index) => index >= 0 && index < values.Length ? values[index] : 0f;

        public float GetParameter(string id) => GetParameter(ParameterIndex(id));

        /// <summary>Set a parameter for the next update, clamped to its
        /// range. Set it every frame to hold it against motions.</summary>
        public void SetParameter(int index, float value)
        {
            if (index < 0 || index >= parameters.Length) return;
            setIndices.Add((uint)index);
            setValues.Add(value);
        }

        public void SetParameter(string id, float value) => SetParameter(ParameterIndex(id), value);

        /// <summary>The values parameter <paramref name="index"/> was
        /// rigged at: the dots on a slider.</summary>
        public float[] ParameterKeys(int index)
        {
            if (!IsLoaded || index < 0 || index >= parameters.Length) return Array.Empty<float>();
            uint n = Ay2d.ay2d_parameter_keys(handle, (uint)index, null, UIntPtr.Zero);
            var keys = new float[n];
            fixed (float* k = keys) Ay2d.ay2d_parameter_keys(handle, (uint)index, k, (UIntPtr)n);
            return keys;
        }

        /// <summary>The parameters a standard group ("EyeBlink",
        /// "LipSync") names, by index; empty when the model has no such group.</summary>
        public int[] GroupParameters(string group)
        {
            if (!IsLoaded) return Array.Empty<int>();
            uint count = Ay2d.ay2d_model_group_count(handle);
            for (uint g = 0; g < count; g++)
            {
                if (Ay2d.Str(Ay2d.ay2d_model_group_name(handle, g)) != group) continue;
                uint n = Ay2d.ay2d_model_group(handle, g, null, UIntPtr.Zero);
                var raw = new uint[n];
                fixed (uint* r = raw) Ay2d.ay2d_model_group(handle, g, r, (UIntPtr)n);
                return Array.ConvertAll(raw, x => (int)x);
            }
            // a .moc3 names its groups in the bundle's model3.json
            if (asset != null)
                foreach (AyatsuriModelAsset.ParameterGroup pg in asset.groups)
                {
                    if (pg.name != group || pg.ids == null) continue;
                    var found = new List<int>();
                    foreach (string id in pg.ids)
                    {
                        int i = ParameterIndex(id);
                        if (i >= 0) found.Add(i);
                    }
                    return found.ToArray();
                }
            return Array.Empty<int>();
        }

        // ---- parts ----

        void ReadParts()
        {
            uint n = Ay2d.ay2d_part_count(handle);
            partCount = (int)n;
            for (uint i = 0; i < n; i++)
            {
                string id = Ay2d.Str(Ay2d.ay2d_part_id(handle, i));
                if (id != null) partIds[id] = (int)i;
            }
        }

        /// <summary>The index of the part with this id, or -1.</summary>
        public int PartIndex(string id) => id != null && partIds.TryGetValue(id, out int i) ? i : -1;

        /// <summary>Fade a part (0 to 1) from the next update on.</summary>
        public void SetPartOpacity(int index, float opacity)
        {
            if (index < 0 || index >= partCount) return;
            partIndices.Add((uint)index);
            partValues.Add(opacity);
        }

        public void SetPartOpacity(string id, float opacity) => SetPartOpacity(PartIndex(id), opacity);

        // ---- motions ----

        public int MotionCount => IsLoaded ? (int)Ay2d.ay2d_motion_count(handle) : 0;

        public string MotionName(int index) =>
            IsLoaded && index >= 0 ? Ay2d.Str(Ay2d.ay2d_motion_id(handle, (uint)index)) : null;

        public int MotionIndex(string name)
        {
            if (!IsLoaded || name == null) return -1;
            fixed (byte* n = Ay2d.Utf8(name)) return Ay2d.ay2d_motion_index(handle, n);
        }

        /// <summary>The motions a model3.json filed under this group
        /// ("Idle", "TapBody"), by name.</summary>
        public string[] MotionsInGroup(string group)
        {
            var names = new List<string>();
            if (asset != null)
                foreach (AyatsuriModelAsset.Clip c in asset.motions)
                    if (c.group == group) names.Add(c.name);
            return names.ToArray();
        }

        /// <summary>Start a motion from its beginning. Whatever plays fades
        /// out; this fades in over <paramref name="fadeIn"/> seconds (below
        /// zero: its authored fade). Returns false when there is no such motion.</summary>
        public bool PlayMotion(string name, bool loop = false, float fadeIn = -1f)
        {
            int i = MotionIndex(name);
            return i >= 0 && Ay2d.ay2d_motion_play(handle, (uint)i, loop ? 1 : 0, fadeIn) == Status.Ok;
        }

        /// <summary>Fade every playing motion out (below zero: its own fade).</summary>
        public void StopMotion(float fadeOut = -1f)
        {
            if (IsLoaded) Ay2d.ay2d_motion_stop(handle, fadeOut);
        }

        /// <summary>The motion in front, by name, or null.</summary>
        public string PlayingMotion => IsLoaded ? MotionName(Ay2d.ay2d_motion_playing(handle)) : null;

        /// <summary>True when the motion in front played once through and holds its last frame.</summary>
        public bool MotionFinished => IsLoaded && Ay2d.ay2d_motion_finished(handle) != 0;

        /// <summary>Where the motion in front is, in seconds; -1 when nothing plays.</summary>
        public float MotionTime
        {
            get => IsLoaded ? Ay2d.ay2d_motion_time(handle) : -1f;
            set
            {
                if (IsLoaded) Ay2d.ay2d_motion_seek(handle, value);
            }
        }

        /// <summary>Play the motion in front at this many times its rate.</summary>
        public void SetMotionSpeed(float speed)
        {
            if (IsLoaded) Ay2d.ay2d_motion_set_speed(handle, speed);
        }

        /// <summary>A motion's length, rate, loop flag and authored fades.</summary>
        public MotionFacts MotionInfo(int index)
        {
            MotionFacts f = default;
            if (IsLoaded && index >= 0) Ay2d.ay2d_motion_info(handle, (uint)index, &f);
            return f;
        }

        // ---- expressions ----

        public int ExpressionCount => IsLoaded ? (int)Ay2d.ay2d_expression_count(handle) : 0;

        public string ExpressionName(int index) =>
            IsLoaded && index >= 0 ? Ay2d.Str(Ay2d.ay2d_expression_id(handle, (uint)index)) : null;

        /// <summary>Show an expression; the one shown before fades out.
        /// Returns false when there is no such expression.</summary>
        public bool SetExpression(string name, float fadeIn = -1f)
        {
            if (!IsLoaded || name == null) return false;
            int i;
            fixed (byte* n = Ay2d.Utf8(name)) i = Ay2d.ay2d_expression_index(handle, n);
            return i >= 0 && Ay2d.ay2d_expression_set(handle, (uint)i, fadeIn) == Status.Ok;
        }

        /// <summary>Fade the shown expression out.</summary>
        public void ClearExpression(float fadeOut = -1f)
        {
            if (IsLoaded) Ay2d.ay2d_expression_clear(handle, fadeOut);
        }

        /// <summary>The expression shown, by name, or null.</summary>
        public string ShownExpression => IsLoaded ? ExpressionName(Ay2d.ay2d_expression_shown(handle)) : null;

        // ---- physics ----

        /// <summary>Put every pendulum back at rest, as after a scene cut.</summary>
        public void ResetPhysics()
        {
            if (IsLoaded) Ay2d.ay2d_reset_physics(handle);
        }
    }
}
