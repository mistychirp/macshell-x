/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#pragma once

#include "effect/effect.h"
#include "opengl/glutils.h"

#include <memory>
#include <unordered_map>
#include <vector>

namespace KWin
{
class BackgroundEffectItem;
class RenderView;
class SurfaceInterface;
}

namespace Macshell
{

class GlassManager;

/// Look of the glass, from the `Effect-macshell` group of kwinrc. The defaults are measured from
/// Tahoe's menus and dock.
struct GlassSettings {
    /// The blur under frosted and liquid glass: dual Kawase passes and their sample offset.
    int blurPasses = 2;
    float blurOffset = 2.0;

    /// The lens of liquid glass and bubbles: how far it bends the background, in logical pixels, at
    /// one logical pixel from the edge, and how far in it reaches.
    float lensStrength = 32.0;
    float lensRadius = 24.0;

    /// Frosted glass, for menus, popovers and alerts: flat and tinted, with a light line that's
    /// mostly the surroundings, inside a dark one.
    float frostSaturation = 1.5;
    float frostTint = 0.58;
    float frostEdgeWhite = 0.12;
    float frostOuterEdge = 0.8;

    /// Liquid glass, for controls like the dock: darkened a little, its edge fading into the
    /// surroundings, a touch brighter.
    float liquidBrightness = 0.89;
    float liquidSaturation = 1.15;
    float liquidEdgeFade = 2.0;
    float liquidEdgeGain = 1.06;

    /// Control modules, like Control Center's capsules: the background brightened rather than
    /// veiled, inside a rim that catches the light. No wide edge, so the lens shows right up to it.
    float controlBrightness = 1.15;
    float controlSaturation = 1.1;
    float controlRim = 0.55;

    /// Chips, for small buttons like a notification's ✕: a very wide blur under a light grey tint,
    /// inside a rim that's even all the way around.
    float chipTint = 0.6;
    float chipRim = 0.15;

    /// Backdrops, like the one behind Notification Center's message when it's empty: how far in
    /// from each side the blur takes to reach full strength, in logical pixels.
    float backdropFadeLeft = 190.0;
    float backdropFadeTop = 30.0;
    float backdropFadeRight = 100.0;
    float backdropFadeBottom = 110.0;

    /// Bubbles, for small labels like the dock's: tinted like menus, with liquid glass's edge,
    /// dimmed to stay legible.
    float bubbleTint = 0.5;
    float bubbleEdgeGain = 0.85;
};

/// What one glass shape keeps between frames, per render view.
struct ShapeRender {
    /// Level 0 is the background behind the shape at device resolution, cached: only the
    /// repainted part is copied each frame. Further levels are the blur chain: level 1 ends up
    /// with the blur, and the first level after the blur's own with the very wide one.
    std::vector<std::unique_ptr<KWin::GLTexture>> textures;
    std::vector<std::unique_ptr<KWin::GLFramebuffer>> framebuffers;
};

struct WindowGlass {
    std::unique_ptr<KWin::BackgroundEffectItem> item;
    std::unordered_map<KWin::RenderView *, std::vector<ShapeRender>> render;
    /// A repaint is scheduled for when the next brightness measurement is due.
    bool probeScheduled = false;
};

/// Glass behind the shapes clients ask for with macshell-glass-v1, the way Tahoe draws it: a flat
/// interior seen through a lens, with an edge that shows the surroundings.
class GlassEffect : public KWin::Effect
{
    Q_OBJECT

public:
    GlassEffect();
    ~GlassEffect() override;

    static bool supported();

    void reconfigure(ReconfigureFlags flags) override;
    void prePaintScreen(KWin::ScreenPrePaintData &data) override;
    void drawWindow(const KWin::RenderTarget &renderTarget, const KWin::RenderViewport &viewport, KWin::EffectWindow *w, int mask, const KWin::Region &deviceRegion, KWin::WindowPaintData &data) override;

    bool isActive() const override;
    bool blocksDirectScanout() const override;

    int requestedEffectChainPosition() const override
    {
        // Right after blur.
        return 21;
    }

private:
    void updateWindow(KWin::EffectWindow *w);
    void paintGlass(const KWin::RenderTarget &renderTarget, const KWin::RenderViewport &viewport, KWin::EffectWindow *w, const KWin::Region &deviceRegion, const KWin::WindowPaintData &data);
    bool ensureTextures(ShapeRender &render, const QSize &size, GLenum format, int levels);
    void frost(ShapeRender &render);
    float measureLuminance(ShapeRender &render, bool linear);

    std::unique_ptr<GlassManager> m_manager;
    GlassSettings m_settings;

    std::unique_ptr<KWin::GLShader> m_glassShader;
    std::unique_ptr<KWin::GLShader> m_downsampleShader;
    std::unique_ptr<KWin::GLShader> m_upsampleShader;

    /// Brightness probes downsample into this.
    std::unique_ptr<KWin::GLTexture> m_probeTexture;
    std::unique_ptr<KWin::GLFramebuffer> m_probeFramebuffer;

    KWin::RenderView *m_currentView = nullptr;
    std::unordered_map<KWin::EffectWindow *, WindowGlass> m_windows;
    bool m_valid = false;
};

}
