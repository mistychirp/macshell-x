// SPDX-FileCopyrightText: 2026 the macshell-x contributors
// SPDX-License-Identifier: GPL-3.0-or-later
#version 140

// The background behind the shape (frosted ones: blurred), stretched over the shape.
uniform sampler2D sampler;

// The shape's size and corner radius, in logical pixels.
uniform vec2 geoSize;
uniform float cornerRadius;
// Device pixels per logical pixel, for antialiasing.
uniform float deviceScale;

// Width of the curved rim, in logical pixels.
uniform float bezel;
// How far the rim bends the background inward at its outer edge, in logical pixels.
uniform float refraction;
// Strength of the specular highlight along the rim.
uniform float highlight;
// Premultiplied tint layered over the glass.
uniform vec4 glassTint;
// Saturation for frosted glass; identity otherwise.
uniform mat4 colorMatrix;
uniform float opacity;

// Texture coordinates: 0 to 1 across the shape, with y pointing up.
in vec2 uv;

out vec4 fragColor;

// Signed distance to a rounded box centered at the origin.
float sdRoundedBox(vec2 p, vec2 halfSize, float r)
{
    vec2 q = abs(p) - halfSize + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Samples the background at shape coordinates (0 to 1, y pointing down).
vec4 sampleGeo(vec2 geo)
{
    return texture(sampler, vec2(geo.x, 1.0 - geo.y));
}

void main(void)
{
    vec2 geo = vec2(uv.x, 1.0 - uv.y);
    vec2 halfSize = geoSize * 0.5;
    vec2 p = geo * geoSize - halfSize;

    float d = sdRoundedBox(p, halfSize, cornerRadius);

    // Antialiased coverage over one device pixel.
    float coverage = clamp(0.5 - d * deviceScale, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }

    // Outward surface normal from the SDF gradient.
    vec2 e = vec2(0.5, 0.0);
    vec2 n = vec2(
        sdRoundedBox(p + e.xy, halfSize, cornerRadius) - sdRoundedBox(p - e.xy, halfSize, cornerRadius),
        sdRoundedBox(p + e.yx, halfSize, cornerRadius) - sdRoundedBox(p - e.yx, halfSize, cornerRadius));
    n = length(n) > 0.0 ? normalize(n) : vec2(0.0);

    // Thin shapes, like a menu bar, get a proportionally thinner rim, so it doesn't swallow
    // the whole shape.
    float rimWidth = max(min(bezel, 0.3 * min(geoSize.x, geoSize.y)), 0.001);
    float rimRefraction = refraction * rimWidth / max(bezel, 0.001);

    // 0 at the outer edge, 1 where the rim meets the flat top.
    float t = clamp(-d / rimWidth, 0.0, 1.0);
    // A convex rim is steepest at the edge, so it bends light the most there.
    float bend = rimRefraction * pow(1.0 - t, 2.5);

    // Light refracts toward the center, so the rim shows content from further in.
    // Channels bend slightly differently for a hint of dispersion.
    vec2 offset = -n * bend / geoSize;
    vec4 color;
    color.r = sampleGeo(geo + offset * 1.06).r;
    color.ga = sampleGeo(geo + offset).ga;
    color.b = sampleGeo(geo + offset * 0.94).b;
    color = color * colorMatrix;
    // The copied background is opaque wherever there's anything behind it.
    color.a = 1.0;

    // Tint, premultiplied over the background.
    color = glassTint + color * (1.0 - glassTint.a);

    // Specular highlight: light from the top-left catches the rim, plus a faint bounce on the
    // opposite side.
    vec2 light = normalize(vec2(-1.0, -1.4));
    float facing = dot(n, light);
    float rim = pow(1.0 - t, 3.0);
    float spec = rim * (max(facing, 0.0) + 0.35 * max(-facing, 0.0));
    // A crisp one-pixel line right at the edge.
    float edge = clamp(1.0 - (-d * deviceScale), 0.0, 1.0);
    color.rgb += vec3(highlight * (spec + 0.5 * edge));

    fragColor = color * coverage * opacity;
}
