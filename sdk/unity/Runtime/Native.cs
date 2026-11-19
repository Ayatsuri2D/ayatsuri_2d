// Ayatsuri2D for Unity: the C ABI (ayatsuri.h) as P/Invoke declarations.
// One entry per ay2d_ function in the header, same names, same order.
// `cargo xtask unity` checks that none is missing. Apache-2.0.

using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Ayatsuri2D.Native
{
    /// <summary>What a call reports. Zero is success.</summary>
    public enum Status : int
    {
        Ok = 0,
        Null = 1,
        BadModel = 2,
        Range = 3,
        TooSmall = 4,
        Internal = 5,
        Busy = 6,
    }

    /// <summary>AY2D_FLAG_* bits on a drawable.</summary>
    [Flags]
    public enum DrawFlags : uint
    {
        None = 0,
        Additive = 1,
        Multiplicative = 2,
        DoubleSided = 4,
        InvertedMask = 8,
    }

    /// <summary>One mesh to draw, as ay2d_drawable. The pointers belong to
    /// the library and stay valid until the update after next.</summary>
    [StructLayout(LayoutKind.Sequential)]
    public unsafe struct Drawable
    {
        public uint mesh;
        public uint texture;
        public DrawFlags flags;
        public float opacity;
        public uint vertexCount;
        public uint indexCount;
        public uint maskCount;
        public float* positions;
        public float* uvs;
        public ushort* indices;
        public uint* masks;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct CanvasInfo
    {
        public float width;
        public float height;
        public float pixelsPerUnit;
        public float originX;
        public float originY;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MotionFacts
    {
        public float duration;
        public float fps;
        public int looped;
        public int composition;
        public float fadeIn;
        public float fadeOut;
        public uint eventCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct FiredEvent
    {
        public uint motion;
        public float time;
        public IntPtr name;
    }

    /// <summary>The C functions, unchanged. Prefer <see cref="AyatsuriModel"/>;
    /// these are for hosts that want the ABI itself.</summary>
    public static unsafe class Ay2d
    {
#if UNITY_IOS && !UNITY_EDITOR
        public const string Lib = "__Internal";
#else
        public const string Lib = "ayatsuri_capi";
#endif

        /// <summary>The ABI version these declarations describe.</summary>
        public const uint AbiVersion = 1;

        [DllImport(Lib)] public static extern uint ay2d_abi_version();
        [DllImport(Lib)] public static extern IntPtr ay2d_version();
        [DllImport(Lib)] public static extern Status ay2d_add_physics3(IntPtr m, byte* json, UIntPtr len);
        [DllImport(Lib)] public static extern int ay2d_add_motion3(IntPtr m, byte* name, byte* json, UIntPtr len);
        [DllImport(Lib)] public static extern int ay2d_add_expression3(IntPtr m, byte* name, byte* json, UIntPtr len);
        [DllImport(Lib)] public static extern uint ay2d_drawables(IntPtr m, Drawable* outp, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_mesh_geometry(IntPtr m, uint mesh, Drawable* outp);
        [DllImport(Lib)] public static extern IntPtr ay2d_last_error();
        [DllImport(Lib)] public static extern uint ay2d_expression_count(IntPtr m);
        [DllImport(Lib)] public static extern IntPtr ay2d_expression_id(IntPtr m, uint i);
        [DllImport(Lib)] public static extern int ay2d_expression_index(IntPtr m, byte* name);
        [DllImport(Lib)] public static extern Status ay2d_expression_set(IntPtr m, uint i, float fadeIn);
        [DllImport(Lib)] public static extern Status ay2d_expression_clear(IntPtr m, float fadeOut);
        [DllImport(Lib)] public static extern int ay2d_expression_shown(IntPtr m);
        [DllImport(Lib)] public static extern Status ay2d_mesh_changes(IntPtr m, uint* outp, UIntPtr n);
        [DllImport(Lib)] public static extern IntPtr ay2d_load(byte* bytes, UIntPtr len);
        [DllImport(Lib)] public static extern void ay2d_free(IntPtr m);
        [DllImport(Lib)] public static extern uint ay2d_parameter_count(IntPtr m);
        [DllImport(Lib)] public static extern uint ay2d_part_count(IntPtr m);
        [DllImport(Lib)] public static extern uint ay2d_mesh_count(IntPtr m);
        [DllImport(Lib)] public static extern uint ay2d_texture_count(IntPtr m);
        [DllImport(Lib)] public static extern Status ay2d_canvas(IntPtr m, CanvasInfo* outp);
        [DllImport(Lib)] public static extern IntPtr ay2d_parameter_id(IntPtr m, uint i);
        [DllImport(Lib)] public static extern IntPtr ay2d_part_id(IntPtr m, uint i);
        [DllImport(Lib)] public static extern IntPtr ay2d_mesh_id(IntPtr m, uint i);
        [DllImport(Lib)] public static extern int ay2d_parameter_index(IntPtr m, byte* id);
        [DllImport(Lib)] public static extern IntPtr ay2d_parameter_name(IntPtr m, uint i);
        [DllImport(Lib)] public static extern IntPtr ay2d_part_name(IntPtr m, uint i);
        [DllImport(Lib)] public static extern IntPtr ay2d_mesh_name(IntPtr m, uint i);
        [DllImport(Lib)] public static extern int ay2d_parameter_repeat(IntPtr m, uint i);
        [DllImport(Lib)] public static extern uint ay2d_parameter_keys(IntPtr m, uint i, float* outp, UIntPtr n);
        [DllImport(Lib)] public static extern int ay2d_part_parent(IntPtr m, uint i);
        [DllImport(Lib)] public static extern int ay2d_mesh_part(IntPtr m, uint i);
        [DllImport(Lib)] public static extern uint ay2d_mesh_blend(IntPtr m, uint i);
        [DllImport(Lib)] public static extern uint ay2d_parameter_group_count(IntPtr m);
        [DllImport(Lib)] public static extern IntPtr ay2d_parameter_group_name(IntPtr m, uint g);
        [DllImport(Lib)] public static extern uint ay2d_parameter_group(IntPtr m, uint g, uint* outp, UIntPtr n);
        [DllImport(Lib)] public static extern uint ay2d_model_group_count(IntPtr m);
        [DllImport(Lib)] public static extern IntPtr ay2d_model_group_name(IntPtr m, uint g);
        [DllImport(Lib)] public static extern uint ay2d_model_group(IntPtr m, uint g, uint* outp, UIntPtr n);
        [DllImport(Lib)] public static extern uint ay2d_motion_count(IntPtr m);
        [DllImport(Lib)] public static extern IntPtr ay2d_motion_id(IntPtr m, uint i);
        [DllImport(Lib)] public static extern int ay2d_motion_index(IntPtr m, byte* name);
        [DllImport(Lib)] public static extern Status ay2d_motion_info(IntPtr m, uint i, MotionFacts* outp);
        [DllImport(Lib)] public static extern Status ay2d_motion_play(IntPtr m, uint i, int looping, float fadeIn);
        [DllImport(Lib)] public static extern Status ay2d_motion_stop(IntPtr m, float fadeOut);
        [DllImport(Lib)] public static extern int ay2d_motion_playing(IntPtr m);
        [DllImport(Lib)] public static extern int ay2d_motion_finished(IntPtr m);
        [DllImport(Lib)] public static extern float ay2d_motion_time(IntPtr m);
        [DllImport(Lib)] public static extern Status ay2d_motion_seek(IntPtr m, float t);
        [DllImport(Lib)] public static extern Status ay2d_motion_set_speed(IntPtr m, float speed);
        [DllImport(Lib)] public static extern uint ay2d_motion_events(IntPtr m, FiredEvent* outp, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_parameter_ranges(IntPtr m, float* min, float* max, float* def, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_get_parameters(IntPtr m, float* values, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_set_parameters(IntPtr m, uint* indices, float* values, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_set_all_parameters(IntPtr m, float* values, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_get_part_opacities(IntPtr m, float* values, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_set_part_opacities(IntPtr m, uint* indices, float* values, UIntPtr n);
        [DllImport(Lib)] public static extern Status ay2d_update(IntPtr m, float dt);
        [DllImport(Lib)] public static extern int ay2d_has_physics(IntPtr m);
        [DllImport(Lib)] public static extern Status ay2d_set_physics(IntPtr m, int on);
        [DllImport(Lib)] public static extern Status ay2d_set_wind(IntPtr m, float x, float y);
        [DllImport(Lib)] public static extern Status ay2d_reset_physics(IntPtr m);
        [DllImport(Lib)] public static extern Status ay2d_texture(IntPtr m, uint i, byte** bytes, UIntPtr* len);

        /// <summary>A NUL-terminated UTF-8 string from the library, or null.</summary>
        public static string Str(IntPtr p)
        {
            if (p == IntPtr.Zero) return null;
            byte* b = (byte*)p;
            int n = 0;
            while (b[n] != 0) n++;
            return Encoding.UTF8.GetString(b, n);
        }

        /// <summary>A string as NUL-terminated UTF-8 for the library.</summary>
        public static byte[] Utf8(string s)
        {
            int n = Encoding.UTF8.GetByteCount(s);
            byte[] b = new byte[n + 1];
            Encoding.UTF8.GetBytes(s, 0, s.Length, b, 0);
            return b;
        }

        /// <summary>Why the last call on this thread failed.</summary>
        public static string LastError() => Str(ay2d_last_error()) ?? "";
    }
}
