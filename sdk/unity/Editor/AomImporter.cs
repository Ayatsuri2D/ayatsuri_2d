// Ayatsuri2D for Unity. Apache-2.0.

using System;
using System.IO;
using Ayatsuri2D.Native;
using UnityEditor.AssetImporters;
using UnityEngine;

namespace Ayatsuri2D.Editor
{
    /// <summary>
    /// An .aom file, as Ayatsuri2D saves it, imports as a ready model: drag
    /// it into a scene. Its texture pages come out as sub-assets; motions,
    /// expressions and physics stay inside the file, where the runtime
    /// reads them.
    /// </summary>
    [ScriptedImporter(1, "aom")]
    public sealed unsafe class AomImporter : ScriptedImporter
    {
        [Tooltip("Build mipmaps for the texture pages. Off keeps the art sharp at its own size.")]
        public bool mipmaps;

        [Tooltip("How the texture pages are filtered.")]
        public FilterMode filterMode = FilterMode.Bilinear;

        public override void OnImportAsset(AssetImportContext ctx)
        {
            byte[] bytes = File.ReadAllBytes(ctx.assetPath);
            string name = Path.GetFileNameWithoutExtension(ctx.assetPath);

            IntPtr model;
            fixed (byte* b = bytes) model = Ay2d.ay2d_load(b, (UIntPtr)bytes.Length);
            if (model == IntPtr.Zero)
            {
                ctx.LogImportError($"{ctx.assetPath} is not a model Ayatsuri2D reads: {Ay2d.LastError()}");
                return;
            }

            Texture2D[] textures;
            try
            {
                uint count = Ay2d.ay2d_texture_count(model);
                textures = new Texture2D[count];
                for (uint i = 0; i < count; i++)
                {
                    byte* png;
                    UIntPtr len;
                    if (Ay2d.ay2d_texture(model, i, &png, &len) != Status.Ok) continue;
                    var data = new byte[(int)len];
                    for (int k = 0; k < data.Length; k++) data[k] = png[k];
                    var tex = new Texture2D(2, 2, TextureFormat.RGBA32, mipmaps)
                    {
                        name = $"{name} texture {i:00}",
                        wrapMode = TextureWrapMode.Clamp,
                        filterMode = filterMode,
                    };
                    if (!tex.LoadImage(data, true))
                    {
                        ctx.LogImportWarning($"texture page {i} of {ctx.assetPath} did not decode");
                        continue;
                    }
                    ctx.AddObjectToAsset($"texture{i}", tex);
                    textures[i] = tex;
                }
            }
            finally
            {
                Ay2d.ay2d_free(model);
            }

            var asset = ScriptableObject.CreateInstance<AyatsuriModelAsset>();
            asset.name = name;
            asset.model = bytes;
            asset.textures = textures;
            ctx.AddObjectToAsset("model", asset);

            var go = new GameObject(name);
            go.AddComponent<MeshFilter>();
            go.AddComponent<MeshRenderer>();
            go.AddComponent<AyatsuriModel>().Assign(asset);
            ctx.AddObjectToAsset("prefab", go);
            ctx.SetMainObject(go);
        }
    }
}
