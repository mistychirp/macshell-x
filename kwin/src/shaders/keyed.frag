// SPDX-FileCopyrightText: 2026 the macshell-x contributors
// SPDX-License-Identifier: GPL-3.0-or-later
#version 140

// A window with only its background see-through. Pixels the color of its background, or close to it,
// let what's behind show; text, icons and pictures stay as they are. The edges of text, which are
// partly background, are partly see-through, which keeps them smooth.

#include "colormanagement.glsl"
#include "saturation.glsl"

// The window, drawn by itself, with its shadow around it.
uniform sampler2D sampler;
uniform vec4 modulation;
// Where the window itself is in that, without the shadow: its left, top, right and bottom.
uniform vec4 frame;
// Its background colors, as they're stored, and how many there are.
uniform vec3 keys[4];
uniform int keyCount;
// How opaque its background is drawn.
uniform float backgroundOpacity;
// 0: as it is, since it's see-through by itself. 1: only its background is see-through. 2: all of it is.
uniform int mode;

in vec2 texcoord0;

out vec4 fragColor;

void main(void)
{
    vec4 result = texture(sampler, texcoord0);
    float keep = mode == 2 ? backgroundOpacity : 1.0;
    if (mode == 1 && result.a > 0.0 && all(greaterThanEqual(texcoord0, frame.xy)) && all(lessThanEqual(texcoord0, frame.zw))) {
        vec3 color = result.rgb / result.a;
        float nearest = 10.0;
        for (int i = 0; i < 4; ++i) {
            if (i < keyCount) {
                nearest = min(nearest, distance(color, keys[i]));
            }
        }
        // Within a few steps of a background color it's background; a little further, it's partly.
        keep = mix(backgroundOpacity, 1.0, smoothstep(0.015, 0.09, nearest));
    }

    result = encodingToNits(result, sourceNamedTransferFunction, sourceTransferFunctionParams.x, sourceTransferFunctionParams.y);
    result.rgb = (colorimetryTransform * vec4(result.rgb, 1.0)).rgb;
    result = adjustSaturation(result);
    result *= modulation * keep;
    result.rgb = doTonemapping(result.rgb);
    fragColor = nitsToDestinationEncoding(result);
}
