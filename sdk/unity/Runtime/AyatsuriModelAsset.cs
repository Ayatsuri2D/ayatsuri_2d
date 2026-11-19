// Ayatsuri2D for Unity. Apache-2.0.

using System;
using UnityEngine;

namespace Ayatsuri2D
{
    /// <summary>
    /// A model as imported: the rig's bytes (.aom or .moc3), its texture
    /// pages, and for a .moc3 the bundle's physics, motions and
    /// expressions. Made by the importers; an <see cref="AyatsuriModel"/>
    /// loads one.
    /// </summary>
    public sealed class AyatsuriModelAsset : ScriptableObject
    {
        /// <summary>A JSON file from a model3.json bundle.</summary>
        [Serializable]
        public struct Clip
        {
            /// <summary>The name to play or show it by.</summary>
            public string name;
            /// <summary>The model3.json motion group ("Idle", "TapBody"),
            /// empty for expressions.</summary>
            public string group;
            public byte[] json;
        }

        /// <summary>A model3.json group: the parameters a blink or a lip
        /// sync drives, by id.</summary>
        [Serializable]
        public struct ParameterGroup
        {
            public string name;
            public string[] ids;
        }

        [Tooltip("The bytes of the .aom or .moc3 file.")]
        public byte[] model = Array.Empty<byte>();

        [Tooltip("Texture pages, by the index the model draws them with.")]
        public Texture2D[] textures = Array.Empty<Texture2D>();

        [Tooltip("physics3.json, for a .moc3 bundle; empty when there is none or the .aom carries it.")]
        public byte[] physics = Array.Empty<byte>();

        public Clip[] motions = Array.Empty<Clip>();
        public Clip[] expressions = Array.Empty<Clip>();

        [Tooltip("model3.json Groups (EyeBlink, LipSync); an .aom names its own.")]
        public ParameterGroup[] groups = Array.Empty<ParameterGroup>();
    }
}
