// Ayatsuri2D for Unity: how every mesh is drawn. Apache-2.0.
//
// Textures are straight alpha; the colour is premultiplied here so the
// three blend modes match the Cubism runtimes:
//   Normal          One       OneMinusSrcAlpha
//   Additive        One       One
//   Multiplicative  DstColor  OneMinusSrcAlpha
// Masks go through the stencil buffer: a mask pass writes where its
// texture is solid (alpha at or above _Cutoff) with no colour, the masked
// mesh draws where the stencil is (or is not) set, and a clear pass puts
// the stencil back. The pass has no LightMode tag, so the Built-in
// pipeline and URP both draw it.
Shader "Ayatsuri2D/Unlit"
{
    Properties
    {
        _MainTex ("Texture", 2D) = "white" {}
        [Enum(UnityEngine.Rendering.BlendMode)] _SrcBlend ("Source blend", Float) = 1
        [Enum(UnityEngine.Rendering.BlendMode)] _DstBlend ("Destination blend", Float) = 10
        _StencilRef ("Stencil reference", Float) = 0
        [Enum(UnityEngine.Rendering.CompareFunction)] _StencilComp ("Stencil compare", Float) = 8
        [Enum(UnityEngine.Rendering.StencilOp)] _StencilPass ("Stencil pass", Float) = 0
        _ColorMask ("Colour mask", Float) = 15
        _Cutoff ("Alpha cutoff", Float) = -1
    }
    SubShader
    {
        Tags { "Queue" = "Transparent" "RenderType" = "Transparent" "IgnoreProjector" = "True" "PreviewType" = "Plane" }
        Cull Off
        ZWrite Off
        ZTest LEqual
        Blend [_SrcBlend] [_DstBlend]
        ColorMask [_ColorMask]
        Stencil
        {
            Ref [_StencilRef]
            Comp [_StencilComp]
            Pass [_StencilPass]
        }
        Pass
        {
            CGPROGRAM
            #pragma vertex vert
            #pragma fragment frag
            #include "UnityCG.cginc"

            sampler2D _MainTex;
            float _Cutoff;

            struct appdata
            {
                float4 vertex : POSITION;
                float2 uv : TEXCOORD0;
                float4 color : COLOR;
            };

            struct v2f
            {
                float4 pos : SV_POSITION;
                float2 uv : TEXCOORD0;
                float4 color : COLOR;
            };

            v2f vert(appdata v)
            {
                v2f o;
                o.pos = UnityObjectToClipPos(v.vertex);
                o.uv = v.uv;
                o.color = v.color;
                return o;
            }

            fixed4 frag(v2f i) : SV_Target
            {
                fixed4 c = tex2D(_MainTex, i.uv);
                clip(c.a - _Cutoff);
                c.rgb *= c.a;
                return c * i.color.a;
            }
            ENDCG
        }
    }
}
