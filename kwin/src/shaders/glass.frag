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
// The same background, not blurred at all.
uniform sampler2D sharp;

// The shape's size and corner radius, in logical pixels.
uniform vec2 geoSize;
uniform float cornerRadius;
// Device pixels per logical pixel. The edge lines are sized in device pixels, to stay crisp.
uniform float deviceScale;

// The lens: how far it bends the background, in logical pixels, at one logical pixel in from the
// edge, and how far in it reaches. A strength of 0 means no lens.
uniform float lensStrength;
uniform float lensRadius;
// A lens that takes what's far in, right past the middle of a capsule, and mirrors it at the edge:
// measured from Tahoe, it bends by lensStrength * (1 - depth / lensRadius)^2.
uniform float lensMirrors;
// How much further red bends than green, and blue less, as a share of the bend: real glass splits
// light a little, which is the faint rainbow along a lens's edge. This, the highlight below and the
// noise are adapted from MacShell's shader (https://github.com/0-ss/MacShell, MIT; see
// LICENSES/MacShell-MIT.txt).
uniform float dispersion;
// How strong the soft highlight is just inside the rim, like the surface tension of something
// thick, for light coming from the top left.
uniform float meniscus;
// How much fine noise there is, which breaks up the banding of smooth blurs on an 8-bit screen.
uniform float grain;
// How much of a soft rim's color comes from what's behind it, rather than being a plain grey.
uniform float rimTint;
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
// A rim instead of an edge: how much its outer device pixel whitens where it's lit. 0 means no rim.
uniform float rim;
// How much of the rim's light depends on its direction, from 0 (even all around) to 1.
uniform float rimDirectional;
// How much the next device pixel whitens, relative to the outer one.
uniform float rimInner;
// Instead: a rim that's a logical pixel and a half wide, soft on the inside, the color of bright
// glass rather than white.
uniform float rimSoft;
// A backdrop instead of a shape: just the blur, fading in from each side over this many logical
// pixels (left, top, right, bottom), with no edge. All 0 means it isn't one.
uniform vec4 softEdges;
// A tail pointing down from the shape: where its tip is from the left, how wide it is and how tall,
// in logical pixels. The tail is part of geoSize. A height of 0 means none, and a negative one means
// it's on top, pointing up.
uniform vec3 tail;
uniform float opacity;

// How wide the disc of samples of a backdrop's fade gets, in logical pixels.
const float maxBackdropDisc = 8.0;

// Texture coordinates: 0 to 1 across the shape, with y pointing up.
in vec2 uv;

out vec4 fragColor;

// Signed distance to a rounded box centered at the origin.
float sdRoundedBox(vec2 p, vec2 halfSize, float r)
{
    vec2 q = abs(p) - halfSize + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Signed distance to an isosceles triangle with its tip at the origin, widening toward +y.
float sdTriangle(vec2 p, vec2 q)
{
    p.x = abs(p.x);
    vec2 a = p - q * clamp(dot(p, q) / dot(q, q), 0.0, 1.0);
    vec2 b = p - q * vec2(clamp(p.x / q.x, 0.0, 1.0), 1.0);
    float k = sign(q.y);
    float d = min(dot(a, a), dot(b, b));
    float s = max(k * (p.x * q.y - p.y * q.x), k * (p.y - q.y));
    return sqrt(d) * sign(s);
}

// Signed distance to the shape, which may have a tail flaring out of its bottom.
float sdShape(vec2 p, vec2 halfSize, float r)
{
    if (tail.z == 0.0) {
        return sdRoundedBox(p, halfSize, r);
    }
    // A tail on top is one at the bottom, upside down.
    if (tail.z < 0.0) {
        p.y = -p.y;
    }
    float tailHeight = abs(tail.z);
    float body = sdRoundedBox(p + vec2(0.0, tailHeight * 0.5), vec2(halfSize.x, halfSize.y - tailHeight * 0.5), r);
    // The tip is rounded by pulling the triangle in and growing it back. It carries on into the
    // body at the same angle, so that the join is a clean one to smooth.
    const float tipRadius = 1.5;
    vec2 tip = vec2(p.x + halfSize.x - tail.x, halfSize.y - tipRadius * 1.6 - p.y);
    vec2 reach = vec2(tail.y * 0.5, tailHeight) * 1.5;
    float triangle = sdTriangle(tip, reach) - tipRadius;
    // A smooth union, which flares the tail's base into the body.
    const float flare = 5.0;
    float h = clamp(0.5 + 0.5 * (triangle - body) / flare, 0.0, 1.0);
    return mix(triangle, body, h) - flare * h * (1.0 - h);
}

// Samples the background at shape coordinates (0 to 1, y pointing down).
vec4 sampleGeo(vec2 geo)
{
    return texture(sampler, vec2(geo.x, 1.0 - geo.y));
}

// Noise with a triangular distribution, around 0, the same for every channel.
float noise()
{
    vec3 p = fract(gl_FragCoord.xyx * 0.1031);
    p += dot(p, p.yzx + 33.33);
    float first = fract((p.x + p.y) * p.z);
    p = fract((gl_FragCoord.xyx * 1.7 + 17.0) * 0.1031);
    p += dot(p, p.yzx + 33.33);
    return first + fract((p.x + p.y) * p.z) - 1.0;
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

    if (softEdges != vec4(0.0)) {
        vec2 near = geo * geoSize;
        vec2 far = geoSize - near;
        float amount = smoothstep(0.0, softEdges.x, near.x) * smoothstep(0.0, softEdges.y, near.y)
            * smoothstep(0.0, softEdges.z, far.x) * smoothstep(0.0, softEdges.w, far.y);
        // The blur grows instead of fading in, so nothing sharp shows through it: a disc of samples
        // that widens, handing over to the real blur before its samples could be told apart.
        const int samples = 16;
        float radius = amount * maxBackdropDisc;
        vec3 disc = vec3(0.0);
        for (int i = 0; i < samples; ++i) {
            float r = sqrt((float(i) + 0.5) / float(samples)) * radius;
            float angle = float(i) * 2.39996;
            vec2 at = geo + vec2(cos(angle), sin(angle)) * r / geoSize;
            disc += texture(sharp, vec2(at.x, 1.0 - at.y)).rgb;
        }
        vec3 color = mix(disc / float(samples), sampleGeo(geo).rgb, smoothstep(0.3, 0.85, amount));
        color *= mix(1.0, brightness, amount);
        // Only so the very first pixels don't pick up rounding.
        color += noise() * grain * amount;
        fragColor = vec4(color, 1.0) * clamp(amount * 20.0, 0.0, 1.0) * opacity;
        return;
    }

    float d = sdShape(p, halfSize, cornerRadius);

    // Antialiased coverage over one device pixel.
    float coverage = clamp(0.5 - d * deviceScale, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }

    // Outward surface normal from the SDF gradient.
    vec2 e = vec2(0.5, 0.0);
    vec2 n = vec2(
        sdShape(p + e.xy, halfSize, cornerRadius) - sdShape(p - e.xy, halfSize, cornerRadius),
        sdShape(p + e.yx, halfSize, cornerRadius) - sdShape(p - e.yx, halfSize, cornerRadius));
    n = length(n) > 0.0 ? normalize(n) : vec2(0.0);

    // The lens bends the background toward the center, most at the edge. Thin shapes get a
    // proportionally smaller one.
    vec2 refracted = geo;
    if (lensMirrors > 0.0) {
        // Measured on capsules 62 pixels tall; smaller shapes get a smaller one.
        float fit = min(1.0, min(geoSize.x, geoSize.y) / 62.0);
        float falloff = max(1.0 - max(-d, 0.0) / (lensRadius * fit), 0.0);
        refracted = geo - n * fit * lensStrength * falloff * falloff / geoSize;
    } else if (lensStrength > 0.0) {
        float fit = min(1.0, 0.35 * min(geoSize.x, geoSize.y) / lensRadius);
        float depth = max(-d, 0.5);
        float bend = fit * lensStrength * max(pow(depth / fit, -0.75) - pow(lensRadius, -0.75), 0.0);
        refracted = geo - n * bend / geoSize;
    }

    vec4 sampled = sampleGeo(refracted);
    if (dispersion > 0.0) {
        vec2 bent = refracted - geo;
        sampled.r = sampleGeo(geo + bent * (1.0 + dispersion)).r;
        sampled.b = sampleGeo(geo + bent * (1.0 - dispersion)).b;
    }
    vec4 color = sampled * colorMatrix;
    // The copied background is opaque wherever there's anything behind it.
    color.a = 1.0;
    color.rgb *= brightness;
    color.rgb = mix(color.rgb, tint.rgb, tint.a);

    if (meniscus > 0.0) {
        float along = (max(-d, 0.0) - lensRadius * 0.5) / (lensRadius * 0.2 + 1.0);
        float facing = dot(n, normalize(vec2(-0.55, -0.83)));
        float light = pow(max(facing, 0.0), 1.6) + 0.55 * pow(max(-facing, 0.0), 2.2);
        color.rgb += exp(-along * along) * meniscus * (0.35 + light);
    }

    // The edge, with no dimming: it shows the surroundings, not what's right behind it.
    vec3 edge = mix(min(texture(wide, uv).rgb * edgeGain, vec3(1.0)), vec3(1.0), edgeWhite);
    float pixels = -d * deviceScale;
    if (rimSoft > 0.0) {
        float lit = mix(0.12, 1.0, smoothstep(0.25, 0.72, abs(dot(n, vec2(0.7071)))));
        // Half a logical pixel in full, then fading out by one and a half.
        float line = clamp(1.4 - max(-d, 0.0), 0.0, 1.0);
        vec3 rimColor = mix(vec3(0.85), clamp(color.rgb * 2.0 + 0.18, 0.0, 1.0), rimTint);
        color.rgb = mix(color.rgb, rimColor, line * lit * rim);
    } else if (rim > 0.0) {
        // Light falls along the top-left to bottom-right diagonal: the rim is even across those two
        // quadrants and almost gone at the other two corners, measured around Tahoe's buttons.
        float lit = mix(0.12, 1.0, smoothstep(0.25, 0.72, abs(dot(n, vec2(0.7071)))));
        lit = mix(1.0, lit, rimDirectional);
        float line = band(pixels, 0.0, 1.0) + band(pixels, 1.0, 2.0) * rimInner;
        color.rgb = mix(color.rgb, vec3(1.0), line * lit * rim);
    } else if (edgeFade > 0.0) {
        color.rgb = mix(color.rgb, edge, 1.0 - smoothstep(0.0, edgeFade, -d));
    } else {
        color.rgb = mix(color.rgb, edge, band(pixels, 1.0, 3.0));
        color.rgb = mix(color.rgb, vec3(0.0), band(pixels, 0.0, 1.0) * outerAlpha);
    }

    color.rgb += noise() * grain;
    fragColor = color * coverage * opacity;
}
