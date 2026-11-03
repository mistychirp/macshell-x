/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#pragma once

#include "effect/effect.h"
#include "opengl/glutils.h"

#include <QColor>

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

/// Look of the glass, from the `Effect-macshell` group of kwinrc.
struct GlassSettings {
    /// Width of the curved rim, in logical pixels.
    float bezel = 20.0;
    /// How far the rim bends the background at its outer edge, in logical pixels.
    float refraction = 16.0;
    /// Strength of the specular highlight along the rim.
    float highlight = 0.35;
    /// Tint layered over the glass.
    QColor tint = QColor(255, 255, 255, 0);

    /// The blur under frosted glass: dual Kawase passes and their sample offset.
    int frostPasses = 2;
    float frostOffset = 2.0;
    float frostSaturation = 1.8;
};

/// What one glass shape keeps between frames, per render view.
struct ShapeRender {
    /// Level 0 is the background behind the shape at device resolution, cached: only the
    /// repainted part is copied each frame. Further levels are the frost's blur chain.
    std::vector<std::unique_ptr<KWin::GLTexture>> textures;
    std::vector<std::unique_ptr<KWin::GLFramebuffer>> framebuffers;
};

struct WindowGlass {
    std::unique_ptr<KWin::BackgroundEffectItem> item;
    std::unordered_map<KWin::RenderView *, std::vector<ShapeRender>> render;
    /// A repaint is scheduled for when the next brightness measurement is due.
    bool probeScheduled = false;
};

/// Liquid glass behind the shapes clients ask for with macshell-glass-v1: the background is
/// refracted along a curved rim, optionally frosted, and lit by a specular highlight.
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
