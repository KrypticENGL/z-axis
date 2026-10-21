cbuffer C : register(b0) {
    float4x4 vp;
    float4 eye;
    float4 color;
    float4 opts; // x: has texture
};
Texture2D tex : register(t0);
SamplerState smp : register(s0);

struct VI { float3 p : POSITION; float3 n : NORMAL; float2 uv : TEXCOORD0; float sel : TEXCOORD1; };
struct VO { float4 pos : SV_Position; float3 w : TEXCOORD0; float3 n : NORMAL; float2 uv : TEXCOORD1; float sel : TEXCOORD2; };

VO vs(VI i) {
    VO o;
    o.pos = mul(vp, float4(i.p, 1));
    o.w = i.p; o.n = i.n; o.uv = i.uv; o.sel = i.sel;
    return o;
}

float4 ps(VO i) : SV_Target {
    float3 n = i.n;
    if (dot(n, n) < 0.01) n = cross(ddy(i.w), ddx(i.w)); // no normals in file: flat shading
    n = normalize(n);
    float3 v = normalize(eye.xyz - i.w);
    if (dot(n, v) < 0) n = -n; // double-sided
    float light = 0.25 + 0.75 * saturate(dot(n, v));
    float4 c = color;
    if (opts.x > 0) c *= tex.Sample(smp, i.uv);
    float3 rgb = lerp(c.rgb * light, float3(1.0, 0.35, 0.05), i.sel * 0.65);
    return float4(rgb, 1);
}

float4 vs_line(float2 p : POSITION) : SV_Position { return float4(p, 0, 1); }
float4 ps_line() : SV_Target { return float4(1.0, 0.75, 0.1, 1); }
