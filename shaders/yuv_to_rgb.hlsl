// yuv_to_rgb.hlsl - NV12 bi-planar YUV 4:2:0 to RGB conversion HLSL shader

Texture2D<float> TextureY : register(t0);
Texture2D<float2> TextureUV : register(t1);
SamplerState SamplerLinear : register(s0);

struct VSInput {
    float2 position : POSITION;
    float2 texcoord : TEXCOORD;
};

struct PSInput {
    float4 position : SV_POSITION;
    float2 texcoord : TEXCOORD;
};

PSInput VSMain(VSInput input) {
    PSInput output;
    output.position = float4(input.position, 0.0f, 1.0f);
    output.texcoord = input.texcoord;
    return output;
}

float4 PSMain(PSInput input) : SV_TARGET {
    float y = TextureY.Sample(SamplerLinear, input.texcoord).r;
    float2 uv = TextureUV.Sample(SamplerLinear, input.texcoord).rg - float2(0.5f, 0.5f);

    // BT.601 YUV to RGB color space conversion matrix
    float r = y + 1.402f * uv.y;
    float g = y - 0.344136f * uv.x - 0.714136f * uv.y;
    float b = y + 1.772f * uv.x;

    return float4(saturate(float3(r, g, b)), 1.0f);
}
