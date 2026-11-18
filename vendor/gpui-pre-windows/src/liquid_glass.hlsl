// DEVHUB DirectX adaptation of 1234igor/gpui-liquid-glass, revision
// c002c4d3758f934fdfaf173e8214a24eb65a05cd, shaders.metal.
// Original additions Copyright (c) 2026 1234igor (MIT); GPUI is Apache-2.0.
// See ../../../licenses/gpui-liquid-glass-NOTICE.txt and accompanying licenses.
// Modified: HLSL/region-local textures, GPUI coverage, and dark theme response.
// Regular uses Gaussian frost; Clear samples the current frame without blur.

float3 liquid_glass_sharp(float2 device_pos) {
    float2 p = clamp(device_pos, bd_sharp_clamp.xy, bd_sharp_clamp.zw);
    return t_backdrop_sharp.SampleLevel(s_backdrop,
        (p - bd_sharp_origin) / bd_sharp_size, 0).rgb;
}

float3 liquid_glass_shade(float3 color, bool clear, float edge,
                         float2 normal, float interaction, bool light,
                         float rim_k) {
    float luma = dot(color, float3(0.2126, 0.7152, 0.0722));
    // Preserve neutral samples in light mode. The AppKit RGB fits were calibrated
    // together with a beige body tint, and turned our white canvas gray/cream.
    color = luma + (color - luma) * (light ? 1.0 : 1.05);
    color = color * 1.01 + 2.0 / 255.0;

    float top_left = saturate((-normal.x - normal.y) * 0.5 + 0.5);
    // DEVHUB: `rim_k` is the strength of "the rim on the unlit side (right and bottom edges)".
    // 0 = default (1.0) / positive = that multiplier / negative = off (0).
    // The top-right and top-left highlights (the top_left term) remain, so only the edge on the dark side can be turned off.
    float bright_rim = clear
        ? pow(edge, 6.0) * (0.12 * rim_k + 0.58 * top_left)
        : pow(edge, 5.0) * (0.06 * rim_k + 0.18 * top_left);
    float inner_glow = pow(edge, 1.7) * (clear ? 0.008 : 0.025);
    float dark_rim = pow(edge, 4.0) * (1.0 - top_left)
        * (light ? (clear ? 0.035 : 0.02) : (clear ? 0.13 : 0.04));
    float3 body_tint = light
        ? float3(1.0, 1.0, 1.0)
        : float3(0.055, 0.067, 0.085);
    // The upstream fit targets light AppKit. A smoked tint gives dark-mode
    // text the same legibility without turning every panel into a white slab.
    float tint = (clear ? (light ? 0.0 : 0.16) : 0.56)
        + inner_glow + interaction * 0.025;
    color = lerp(color, body_tint, tint);
    color = lerp(color, float3(1.0, 1.0, 1.0), bright_rim) * (1.0 - dark_rim);

    return color;
}

float3 liquid_glass_material(Backdrop b, float2 p, float sdf) {
    bool clear = b.glass.x > 1.5;
    bool light = b.glass.z > 0.5;
    float scale = max(b.glass.w, 0.01);
    float bevel = (clear ? 16.0 : 15.0) * scale;
    float edge = saturate(1.0 + sdf / bevel);
    float lens = smoothstep(0.0, 1.0, edge);
    float2 normal = backdrop_normal(p, b.bounds, b.corner_radii);
    float2 sample_pixel = p - normal * (clear ? 26.0 : 3.2) * scale * lens;
    float3 sample_color = clear ? liquid_glass_sharp(sample_pixel) : backdrop_sample(sample_pixel);
    // DEVHUB: strength of the right and bottom edge rim. `Backdrop::rim[1]` is not used
    // for any other purpose on this path, so it is used here. 0 = default (1.0) / negative = off.
    float rim_k = b.rim.y < 0.0 ? 0.0 : (b.rim.y > 0.0 ? b.rim.y : 1.0);
    float3 color = liquid_glass_shade(sample_color, clear, edge, normal, b.glass.y, light, rim_k);

    if (clear && light) {
        color = saturate(color);
        // AppKit can adapt foreground vibrancy; our light navigation uses fixed
        // dark text. Lift only dark samples, retaining their unblurred detail.
        float luminance = dot(saturate(color), float3(0.2126, 0.7152, 0.0722));
        float veil = saturate((0.60 - luminance) / max(1.0 - luminance, 0.001));
        color = lerp(color, float3(1.0, 1.0, 1.0), veil);
    }
    float glow = b.glass.y * (0.055 + pow(edge, 2.2) * 0.12);
    return saturate(lerp(color, float3(1.0, 1.0, 1.0), glow));
}
