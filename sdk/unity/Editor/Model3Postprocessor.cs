// Ayatsuri2D for Unity. Apache-2.0.

using System;
using System.Collections.Generic;
using System.IO;
using UnityEditor;
using UnityEngine;

namespace Ayatsuri2D.Editor
{
    /// <summary>
    /// A model3.json bundle (the folder VTube Studio loads) imports as two
    /// assets beside it: <c>name.asset</c> with the rig, physics, motions and
    /// expressions, and <c>name.prefab</c> ready to drag into a scene. The
    /// textures stay the PNGs Unity imported. Changing any file of the
    /// bundle rebuilds the asset; the prefab is made once and kept, so your
    /// changes to it survive.
    /// </summary>
    internal sealed class Model3Postprocessor : AssetPostprocessor
    {
        const string Model3 = ".model3.json";

        static void OnPostprocessAllAssets(string[] imported, string[] deleted, string[] moved, string[] movedFrom)
        {
            var bundles = new HashSet<string>();
            foreach (string raw in imported)
            {
                string path = raw.Replace('\\', '/');
                if (path.EndsWith(Model3, StringComparison.OrdinalIgnoreCase))
                    bundles.Add(path);
                else if (PartOfBundle(path))
                    foreach (string b in BundlesNear(path)) bundles.Add(b);
            }
            foreach (string b in bundles)
            {
                try
                {
                    Build(b);
                }
                catch (Exception e)
                {
                    Debug.LogError($"Ayatsuri2D: {b} did not import: {e.Message}");
                }
            }
        }

        void OnPreprocessTexture()
        {
            // the art pages of a bundle: no mipmaps, clamped, as the runtime
            // expects; only on first import, so your own settings stay
            if (!assetImporter.importSettingsMissing || BundlesNear(assetPath).Count == 0) return;
            var t = (TextureImporter)assetImporter;
            t.textureType = TextureImporterType.Default;
            t.mipmapEnabled = false;
            t.wrapMode = TextureWrapMode.Clamp;
            t.alphaIsTransparency = false;
            t.npotScale = TextureImporterNPOTScale.None;
        }

        static bool PartOfBundle(string path)
        {
            string p = path.ToLowerInvariant();
            return p.EndsWith(".moc3") || p.EndsWith(".motion3.json") || p.EndsWith(".exp3.json") || p.EndsWith(".physics3.json");
        }

        // model3.json files in the folder or the one above it (motions and
        // texture pages often sit in a subfolder)
        static List<string> BundlesNear(string path)
        {
            var found = new List<string>();
            string dir = Path.GetDirectoryName(path);
            for (int up = 0; up < 2 && !string.IsNullOrEmpty(dir); up++)
            {
                if (Directory.Exists(dir))
                    foreach (string f in Directory.GetFiles(dir, "*" + Model3))
                        found.Add(f.Replace('\\', '/'));
                dir = Path.GetDirectoryName(dir);
            }
            return found;
        }

        static string Join(string dir, string rel)
        {
            string p = (dir + "/" + rel).Replace('\\', '/');
            var parts = new List<string>();
            foreach (string s in p.Split('/'))
            {
                if (s == "." || s.Length == 0) continue;
                if (s == ".." && parts.Count > 0) parts.RemoveAt(parts.Count - 1);
                else parts.Add(s);
            }
            return string.Join("/", parts);
        }

        static void Build(string path)
        {
            string dir = Path.GetDirectoryName(path).Replace('\\', '/');
            string file = Path.GetFileName(path);
            string stem = file.Substring(0, file.Length - Model3.Length);

            object root = Json.Parse(File.ReadAllText(path));
            var refs = Json.Obj(root, "FileReferences") ?? throw new FormatException("no FileReferences");
            string moc = Json.Str(refs, "Moc") ?? throw new FormatException("no Moc");

            var asset = AssetDatabase.LoadAssetAtPath<AyatsuriModelAsset>($"{dir}/{stem}.asset");
            bool fresh = asset == null;
            if (fresh) asset = ScriptableObject.CreateInstance<AyatsuriModelAsset>();

            asset.model = File.ReadAllBytes(Join(dir, moc));

            var textures = new List<Texture2D>();
            foreach (object t in Json.Arr(refs, "Textures") ?? new List<object>())
            {
                string tp = Join(dir, t as string ?? "");
                var tex = AssetDatabase.LoadAssetAtPath<Texture2D>(tp);
                if (tex == null) Debug.LogWarning($"Ayatsuri2D: {path} names a texture Unity has not imported: {tp}");
                textures.Add(tex);
            }
            asset.textures = textures.ToArray();

            string physics = Json.Str(refs, "Physics");
            asset.physics = physics != null && File.Exists(Join(dir, physics))
                ? File.ReadAllBytes(Join(dir, physics))
                : Array.Empty<byte>();

            var names = new HashSet<string>();
            var motions = new List<AyatsuriModelAsset.Clip>();
            var groups = Json.Obj(refs, "Motions");
            if (groups != null)
                foreach (KeyValuePair<string, object> g in groups)
                    foreach (object m in g.Value as List<object> ?? new List<object>())
                    {
                        string mf = Json.Str(m, "File");
                        if (mf == null || !File.Exists(Join(dir, mf))) continue;
                        motions.Add(new AyatsuriModelAsset.Clip
                        {
                            name = Unique(names, ClipName(mf, ".motion3.json")),
                            group = g.Key,
                            json = File.ReadAllBytes(Join(dir, mf)),
                        });
                    }
            asset.motions = motions.ToArray();

            names.Clear();
            var expressions = new List<AyatsuriModelAsset.Clip>();
            foreach (object e in Json.Arr(refs, "Expressions") ?? new List<object>())
            {
                string ef = Json.Str(e, "File");
                if (ef == null || !File.Exists(Join(dir, ef))) continue;
                expressions.Add(new AyatsuriModelAsset.Clip
                {
                    name = Unique(names, Json.Str(e, "Name") ?? ClipName(ef, ".exp3.json")),
                    group = "",
                    json = File.ReadAllBytes(Join(dir, ef)),
                });
            }
            asset.expressions = expressions.ToArray();

            var parameterGroups = new List<AyatsuriModelAsset.ParameterGroup>();
            foreach (object g in Json.Arr(root, "Groups") ?? new List<object>())
            {
                if (Json.Str(g, "Target") != "Parameter") continue;
                var ids = new List<string>();
                foreach (object id in Json.Arr(g, "Ids") ?? new List<object>())
                    if (id is string s) ids.Add(s);
                parameterGroups.Add(new AyatsuriModelAsset.ParameterGroup { name = Json.Str(g, "Name") ?? "", ids = ids.ToArray() });
            }
            asset.groups = parameterGroups.ToArray();

            if (fresh)
            {
                asset.name = stem;
                AssetDatabase.CreateAsset(asset, $"{dir}/{stem}.asset");
            }
            else
            {
                EditorUtility.SetDirty(asset);
                AssetDatabase.SaveAssetIfDirty(asset);
            }

            string prefab = $"{dir}/{stem}.prefab";
            if (AssetDatabase.LoadAssetAtPath<GameObject>(prefab) == null)
            {
                var go = new GameObject(stem);
                try
                {
                    go.AddComponent<MeshFilter>();
                    go.AddComponent<MeshRenderer>();
                    go.AddComponent<AyatsuriModel>().Assign(asset);
                    PrefabUtility.SaveAsPrefabAsset(go, prefab);
                }
                finally
                {
                    UnityEngine.Object.DestroyImmediate(go);
                }
            }
        }

        static string ClipName(string file, string ext)
        {
            string n = Path.GetFileName(file);
            return n.EndsWith(ext, StringComparison.OrdinalIgnoreCase) ? n.Substring(0, n.Length - ext.Length) : Path.GetFileNameWithoutExtension(n);
        }

        static string Unique(HashSet<string> taken, string name)
        {
            string n = name;
            for (int i = 2; !taken.Add(n); i++) n = $"{name}_{i}";
            return n;
        }
    }
}
