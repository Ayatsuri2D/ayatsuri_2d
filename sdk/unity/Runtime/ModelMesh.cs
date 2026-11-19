// Ayatsuri2D for Unity. Apache-2.0.

using System;
using System.Collections.Generic;
using Ayatsuri2D.Native;
using UnityEngine;
using UnityEngine.Rendering;

namespace Ayatsuri2D
{
    /// <summary>
    /// Turns the drawables of one update into a Unity mesh: one submesh
    /// per pass, in draw order, so one renderer draws the whole model and
    /// the order holds in every pipeline. A masked mesh becomes three
    /// passes: its masks into the stencil, the mesh tested against it, and
    /// its masks again to clear the stencil for whatever comes next.
    /// </summary>
    internal sealed unsafe class ModelMesh : IDisposable
    {
        // material kinds, the low bits of a material key
        const int MaskWrite = 9;
        const int MaskClear = 10;
        const int StencilBit = 1;

        // Cubism's masks are hard-edged at half alpha
        const float MaskCutoff = 0.5f;

        static Shader shader;

        public readonly Mesh Mesh;

        readonly Dictionary<int, Material> materials = new Dictionary<int, Material>();
        readonly List<Material> passMaterials = new List<Material>();
        Material[] assigned = Array.Empty<Material>();

        readonly List<Vector3> positions = new List<Vector3>();
        readonly List<Vector2> uvs = new List<Vector2>();
        readonly List<Color> colors = new List<Color>();
        readonly List<int> indices = new List<int>();
        readonly List<(int start, int count)> passes = new List<(int, int)>();
        Drawable[] drawables = new Drawable[64];

        public ModelMesh(string name)
        {
            Mesh = new Mesh { name = name, hideFlags = HideFlags.DontSave, indexFormat = IndexFormat.UInt32 };
            Mesh.MarkDynamic();
        }

        /// <summary>
        /// Rebuild the mesh from the model's last update and hand the
        /// renderer one material per pass. Model units become Unity units
        /// with y turned up; texture rows are read top-down, so v flips too.
        /// </summary>
        public void Build(IntPtr model, Texture2D[] textures, Renderer renderer, Bounds bounds)
        {
            uint n = Ay2d.ay2d_drawables(model, null, UIntPtr.Zero);
            if (drawables.Length < n) drawables = new Drawable[Math.Max(n, (uint)drawables.Length * 2)];
            fixed (Drawable* d = drawables) n = Ay2d.ay2d_drawables(model, d, (UIntPtr)drawables.Length);

            positions.Clear();
            uvs.Clear();
            colors.Clear();
            indices.Clear();
            passes.Clear();
            passMaterials.Clear();

            for (int i = 0; i < n; i++)
            {
                Drawable d = drawables[i];
                Texture2D tex = Page(textures, d.texture);
                bool masked = d.maskCount > 0;
                if (masked) Masks(model, d, textures, MaskWrite);

                int blend = (d.flags & DrawFlags.Additive) != 0 ? 1
                    : (d.flags & DrawFlags.Multiplicative) != 0 ? 2 : 0;
                int mask = !masked ? 0 : (d.flags & DrawFlags.InvertedMask) != 0 ? 2 : 1;
                Add(d, d.opacity, MaterialFor(tex, d.texture, blend * 3 + mask));

                if (masked) Masks(model, d, textures, MaskClear);
            }

            Mesh.Clear(true);
            Mesh.SetVertices(positions);
            Mesh.SetUVs(0, uvs);
            Mesh.SetColors(colors);
            Mesh.subMeshCount = passes.Count;
            for (int p = 0; p < passes.Count; p++)
                Mesh.SetIndices(indices, passes[p].start, passes[p].count, MeshTopology.Triangles, p, false);
            Mesh.bounds = bounds;

            if (!Same(passMaterials, assigned))
            {
                assigned = passMaterials.ToArray();
                renderer.sharedMaterials = assigned;
            }
        }

        void Masks(IntPtr model, Drawable d, Texture2D[] textures, int kind)
        {
            for (int k = 0; k < d.maskCount; k++)
            {
                Drawable g;
                if (Ay2d.ay2d_mesh_geometry(model, d.masks[k], &g) != Status.Ok) continue;
                Add(g, 1f, MaterialFor(Page(textures, g.texture), g.texture, kind));
            }
        }

        void Add(Drawable d, float opacity, Material material)
        {
            int baseVertex = positions.Count;
            var c = new Color(1f, 1f, 1f, opacity);
            for (int v = 0; v < d.vertexCount; v++)
            {
                positions.Add(new Vector3(d.positions[2 * v], -d.positions[2 * v + 1], 0f));
                uvs.Add(new Vector2(d.uvs[2 * v], 1f - d.uvs[2 * v + 1]));
                colors.Add(c);
            }
            int start = indices.Count;
            for (int k = 0; k < d.indexCount; k++) indices.Add(baseVertex + d.indices[k]);
            passes.Add((start, (int)d.indexCount));
            passMaterials.Add(material);
        }

        static Texture2D Page(Texture2D[] textures, uint i) =>
            textures != null && i < textures.Length ? textures[i] : null;

        Material MaterialFor(Texture2D tex, uint page, int kind)
        {
            int key = (int)page << 4 | kind;
            if (materials.TryGetValue(key, out Material m) && m != null) return m;

            if (shader == null) shader = Resources.Load<Shader>("Ayatsuri2DUnlit");
            m = new Material(shader) { hideFlags = HideFlags.DontSave, name = $"Ayatsuri2D page {page} kind {kind}" };
            m.mainTexture = tex;
            if (kind == MaskWrite || kind == MaskClear)
            {
                m.SetFloat("_ColorMask", 0);
                m.SetFloat("_Cutoff", MaskCutoff);
                m.SetFloat("_StencilRef", kind == MaskWrite ? StencilBit : 0);
                m.SetFloat("_StencilComp", (float)CompareFunction.Always);
                m.SetFloat("_StencilPass", (float)StencilOp.Replace);
            }
            else
            {
                int blend = kind / 3, mask = kind % 3;
                (BlendMode src, BlendMode dst) = blend switch
                {
                    1 => (BlendMode.One, BlendMode.One),
                    2 => (BlendMode.DstColor, BlendMode.OneMinusSrcAlpha),
                    _ => (BlendMode.One, BlendMode.OneMinusSrcAlpha),
                };
                m.SetFloat("_SrcBlend", (float)src);
                m.SetFloat("_DstBlend", (float)dst);
                m.SetFloat("_StencilRef", StencilBit);
                m.SetFloat("_StencilComp", (float)(mask switch
                {
                    1 => CompareFunction.Equal,
                    2 => CompareFunction.NotEqual,
                    _ => CompareFunction.Always,
                }));
                m.SetFloat("_StencilPass", (float)StencilOp.Keep);
            }
            materials[key] = m;
            return m;
        }

        static bool Same(List<Material> a, Material[] b)
        {
            if (a.Count != b.Length) return false;
            for (int i = 0; i < b.Length; i++)
                if (a[i] != b[i]) return false;
            return true;
        }

        public void Dispose()
        {
            foreach (Material m in materials.Values) Destroy(m);
            materials.Clear();
            Destroy(Mesh);
        }

        static void Destroy(UnityEngine.Object o)
        {
            if (o == null) return;
            if (Application.isPlaying) UnityEngine.Object.Destroy(o);
            else UnityEngine.Object.DestroyImmediate(o);
        }
    }
}
