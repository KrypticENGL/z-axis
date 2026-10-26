// Compiled offline for vs_4_0 / ps_4_0 (feature level 10_0).

cbuffer Frame : register(b0)
{
    float4x4 viewProj;
    float4 toLight;    // xyz: unit vector towards the directional light
    float4 lightColor; // linear RGB
    float4 ambient;    // linear RGB
};

cbuffer Material : register(b1)
{
    float4 baseColor;
};

Texture2D baseTexture : register(t0);
SamplerState linearSampler : register(s0);

struct VSIn
{
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
};

struct VSOut
{
    float4 position : SV_Position;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
};

VSOut VSMain(VSIn i)
{
    VSOut o;
    o.position = mul(float4(i.position, 1), viewProj);
    o.normal = i.normal;
    o.uv = i.uv;
    return o;
}

float4 PSMain(VSOut i) : SV_Target
{
    float4 albedo = baseTexture.Sample(linearSampler, i.uv) * baseColor;
    float ndotl = saturate(dot(normalize(i.normal), toLight.xyz));
    return float4(albedo.rgb * (ambient.rgb + lightColor.rgb * ndotl), albedo.a);
}
