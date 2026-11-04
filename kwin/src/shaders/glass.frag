// SPDX-FileCopyrightText: 2026 the macshell-x contributors
// SPDX-License-Identifier: GPL-3.0-or-later
#version 140

// Glass the way Tahoe draws it: a flat interior, with no glow, finished by crisp lines right at
// the edge. Measured from macOS screenshots.

// The background behind the shape (frosted ones: blurred), stretched over the shape.
uniform sampler2D sampler;

// The shape's size and corner radius, in logical pixels.
uniform vec2 geoSize;
uniform float cornerRadius;
// Device pixels per logical pixel. The edge lines are sized in device pixels, to stay crisp.
uniform float deviceScale;

// Width of the band along the edge that bends the background, in logical pixels, and how far it
// bends it at the outer edge. A width of 0 means none.
uniform float rimWidth;
uniform float refraction;
// Multiplies the background's color; below 1 darkens it.
uniform float brightness;
// Saturation of the background.
uniform mat4 colorMatrix;
// A color laid over the background: rgb, and how opaque it is.
uniform vec4 tint;
// The light line along the edge is the glass's own color, brightened: multiplied by edgeBoost,
// then mixed with white by edgeWhite. That way it takes on the color behind it. It's two device
// pixels wide, the inner one at edgeInner strength.
uniform float edgeBoost;
uniform float edgeWhite;
uniform float edgeInner;
// How much stronger the line gets where the edge faces the light, along the top-left to
// bottom-right diagonal, and weaker elsewhere.
uniform float edgeLit;
// The dark one-device-pixel line outside the light one: how strongly it darkens. 0 means none.
uniform float outerAlpha;
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

    // Light refracts toward the center, so the band shows content from further in, bending most
    // at the edge. Thin shapes get a proportionally thinner band.
    vec2 refracted = geo;
    if (rimWidth > 0.0) {
        float width = max(min(rimWidth, 0.3 * min(geoSize.x, geoSize.y)), 0.001);
        float t = clamp(-d / width, 0.0, 1.0);
        float bend = refraction * (width / rimWidth) * pow(1.0 - t, 2.5);
        refracted = geo - n * bend / geoSize;
    }

    vec4 color = sampleGeo(refracted) * colorMatrix;
    // The copied background is opaque wherever there's anything behind it.
    color.a = 1.0;
    color.rgb *= brightness;
    color.rgb = mix(color.rgb, tint.rgb, tint.a);

    // The edge lines, from the outside in: dark, then light, strong then weaker.
    float depth = -d * deviceScale;
    float outerWidth = outerAlpha > 0.0 ? 1.0 : 0.0;
    float facing = abs(dot(n, normalize(vec2(-1.0, -1.0))));
    float line = band(depth, outerWidth, outerWidth + 1.0)
        + band(depth, outerWidth + 1.0, outerWidth + 2.0) * edgeInner;
    vec3 bright = mix(min(color.rgb * edgeBoost, vec3(1.0)), vec3(1.0), edgeWhite);
    color.rgb = mix(color.rgb, bright, clamp(line * (1.0 + edgeLit * (facing - 0.7)), 0.0, 1.0));
    color.rgb = mix(color.rgb, vec3(0.0), band(depth, 0.0, outerWidth) * outerAlpha);

    fragColor = color * coverage * opacity;
}
