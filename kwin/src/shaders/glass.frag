// SPDX-FileCopyrightText: 2026 the macshell-x contributors
// SPDX-License-Identifier: GPL-3.0-or-later
#version 140

// Glass the way Tahoe draws it, measured from macOS screenshots: a flat interior with no glow.
// Tahoe's glass is a lens: each pixel shows the background from further in, by an amount that grows
// sharply toward the edge, and the very edge shows a wide average of the surroundings. That's why
// an edge can be pink right above purple wallpaper, and why it doesn't read as an outline.

// The background behind the shape (frosted ones: blurred), stretched over the shape.
uniform sampler2D sampler;
// The same background, blurred very widely.
uniform sampler2D wide;

// The shape's size and corner radius, in logical pixels.
uniform vec2 geoSize;
uniform float cornerRadius;
// Device pixels per logical pixel. The edge lines are sized in device pixels, to stay crisp.
uniform float deviceScale;

// The lens: how far it bends the background, in logical pixels, at one logical pixel in from the
// edge, and how far in it reaches. A strength of 0 means no lens.
uniform float lensStrength;
uniform float lensRadius;
// Multiplies the background's color; below 1 darkens it.
uniform float brightness;
// Saturation of the background.
uniform mat4 colorMatrix;
// A color laid over the background: rgb, and how opaque it is.
uniform vec4 tint;
// The edge shows the wide blur, multiplied by edgeGain and mixed with white by edgeWhite. It either
// fades in over edgeFade logical pixels, or, with edgeFade 0, is a crisp two-device-pixel line
// inside a one-device-pixel dark line that darkens by outerAlpha.
uniform float edgeFade;
uniform float edgeGain;
uniform float edgeWhite;
uniform float outerAlpha;
// A rim instead of an edge, for control modules: how much its outer device pixel whitens where it's
// lit. The next one does about a third as much. 0 means no rim.
uniform float rim;
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

// 1 between two distances from the edge, in device pixels, antialiased.
float band(float depth, float from, float to)
{
    return clamp(depth - from + 0.5, 0.0, 1.0) * clamp(to - depth + 0.5, 0.0, 1.0);
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

    // The lens bends the background toward the center, most at the edge. Thin shapes get a
    // proportionally smaller one.
    vec2 refracted = geo;
    if (lensStrength > 0.0) {
        float fit = min(1.0, 0.35 * min(geoSize.x, geoSize.y) / lensRadius);
        float depth = max(-d, 0.5);
        float bend = fit * lensStrength * max(pow(depth / fit, -0.75) - pow(lensRadius, -0.75), 0.0);
        refracted = geo - n * bend / geoSize;
    }

    vec4 color = sampleGeo(refracted) * colorMatrix;
    // The copied background is opaque wherever there's anything behind it.
    color.a = 1.0;
    color.rgb *= brightness;
    color.rgb = mix(color.rgb, tint.rgb, tint.a);

    // The edge, with no dimming: it shows the surroundings, not what's right behind it.
    vec3 edge = mix(min(texture(wide, uv).rgb * edgeGain, vec3(1.0)), vec3(1.0), edgeWhite);
    float pixels = -d * deviceScale;
    if (rim > 0.0) {
        // Light falls along the top-left to bottom-right diagonal: the rim is even across those two
        // quadrants and almost gone at the other two corners, measured around Tahoe's buttons.
        float lit = mix(0.12, 1.0, smoothstep(0.25, 0.72, abs(dot(n, vec2(0.7071)))));
        float line = band(pixels, 0.0, 1.0) + band(pixels, 1.0, 2.0) * 0.33;
        color.rgb = mix(color.rgb, vec3(1.0), line * lit * rim);
    } else if (edgeFade > 0.0) {
        color.rgb = mix(color.rgb, edge, 1.0 - smoothstep(0.0, edgeFade, -d));
    } else {
        color.rgb = mix(color.rgb, edge, band(pixels, 1.0, 3.0));
        color.rgb = mix(color.rgb, vec3(0.0), band(pixels, 0.0, 1.0) * outerAlpha);
    }

    fragColor = color * coverage * opacity;
}
