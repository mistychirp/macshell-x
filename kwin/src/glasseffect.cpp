/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#include "glasseffect.h"
#include "glassmanager.h"

#include "core/rendertarget.h"
#include "core/renderviewport.h"
#include "effect/effecthandler.h"
#include "opengl/eglcontext.h"
#include "scene/backgroundeffectitem.h"
#include "wayland/surface.h"

#include <KConfigGroup>
#include <KSharedConfig>

#include <QLoggingCategory>
#include <QMatrix4x4>
#include <QPointer>
#include <QTimer>

#include <algorithm>
#include <cmath>

Q_LOGGING_CATEGORY(MACSHELL_GLASS, "macshell_glass", QtWarningMsg)

using namespace KWin;

namespace Macshell
{

/// Brightness probes downsample the background to this many pixels before reading it back.
static constexpr QSize s_probeSize(32, 4);

/// Saturation as a color matrix, applied to row vectors like KWin's blur does.
static QMatrix4x4 saturationMatrix(float saturation)
{
    const float r = (1.0f - saturation) * 0.2126f;
    const float g = (1.0f - saturation) * 0.7152f;
    const float b = (1.0f - saturation) * 0.0722f;
    return QMatrix4x4(r + saturation, r, r, 0.0f,
                      g, g + saturation, g, 0.0f,
                      b, b, b + saturation, 0.0f,
                      0.0f, 0.0f, 0.0f, 1.0f);
}

static float srgbToLinear(float c)
{
    return c <= 0.04045f ? c / 12.92f : std::pow((c + 0.055f) / 1.055f, 2.4f);
}

GlassEffect::GlassEffect()
{
    m_glassShader = ShaderManager::instance()->generateShaderFromFile(ShaderTrait::MapTexture,
                                                                      QStringLiteral(":/effects/macshell/shaders/vertex.vert"),
                                                                      QStringLiteral(":/effects/macshell/shaders/glass.frag"));
    m_downsampleShader = ShaderManager::instance()->generateShaderFromFile(ShaderTrait::MapTexture,
                                                                           QStringLiteral(":/effects/macshell/shaders/vertex.vert"),
                                                                           QStringLiteral(":/effects/macshell/shaders/downsample.frag"));
    m_upsampleShader = ShaderManager::instance()->generateShaderFromFile(ShaderTrait::MapTexture,
                                                                         QStringLiteral(":/effects/macshell/shaders/vertex.vert"),
                                                                         QStringLiteral(":/effects/macshell/shaders/upsample.frag"));
    if (!m_glassShader || !m_downsampleShader || !m_upsampleShader) {
        qCWarning(MACSHELL_GLASS) << "failed to load the glass shaders";
        return;
    }

    m_manager = std::make_unique<GlassManager>(effects->waylandDisplay());
    connect(m_manager.get(), &GlassManager::glassChanged, this, [this](SurfaceInterface *surface) {
        if (EffectWindow *w = effects->findWindow(surface)) {
            updateWindow(w);
        }
    });

    // Glass can arrive before its window maps, and windows can go without dropping it.
    connect(effects, &EffectsHandler::windowAdded, this, &GlassEffect::updateWindow);
    connect(effects, &EffectsHandler::windowDeleted, this, [this](EffectWindow *w) {
        if (auto it = m_windows.find(w); it != m_windows.end()) {
            effects->makeOpenGLContextCurrent();
            m_windows.erase(it);
        }
    });
    connect(effects, &EffectsHandler::viewRemoved, this, [this](RenderView *view) {
        effects->makeOpenGLContextCurrent();
        for (auto &[window, glass] : m_windows) {
            glass.render.erase(view);
        }
    });

    reconfigure(ReconfigureAll);
    for (EffectWindow *w : effects->stackingOrder()) {
        updateWindow(w);
    }
    m_valid = true;
}

GlassEffect::~GlassEffect() = default;

bool GlassEffect::supported()
{
    return effects->isOpenGLCompositing();
}

void GlassEffect::reconfigure(ReconfigureFlags flags)
{
    Q_UNUSED(flags)
    const KConfigGroup group = KSharedConfig::openConfig(QStringLiteral("kwinrc"))->group(QStringLiteral("Effect-macshell"));
    const GlassSettings defaults;
    m_settings.blurPasses = std::clamp(group.readEntry("BlurPasses", defaults.blurPasses), 1, 4);
    m_settings.blurOffset = group.readEntry("BlurOffset", defaults.blurOffset);
    m_settings.frostSaturation = group.readEntry("FrostSaturation", defaults.frostSaturation);
    m_settings.frostTint = group.readEntry("FrostTint", defaults.frostTint);
    m_settings.frostEdge = group.readEntry("FrostEdge", defaults.frostEdge);
    m_settings.frostOuterEdge = group.readEntry("FrostOuterEdge", defaults.frostOuterEdge);
    m_settings.liquidRim = group.readEntry("LiquidRim", defaults.liquidRim);
    m_settings.liquidRefraction = group.readEntry("LiquidRefraction", defaults.liquidRefraction);
    m_settings.liquidBrightness = group.readEntry("LiquidBrightness", defaults.liquidBrightness);
    m_settings.liquidSaturation = group.readEntry("LiquidSaturation", defaults.liquidSaturation);
    m_settings.liquidEdgeBoost = group.readEntry("LiquidEdgeBoost", defaults.liquidEdgeBoost);
    m_settings.liquidEdgeWhite = group.readEntry("LiquidEdgeWhite", defaults.liquidEdgeWhite);
    effects->addRepaintFull();
}

void GlassEffect::updateWindow(EffectWindow *w)
{
    GlassSurface *glass = w->surface() ? m_manager->glassFor(w->surface()) : nullptr;
    if (!glass || glass->shapes().isEmpty()) {
        if (auto it = m_windows.find(w); it != m_windows.end()) {
            effects->makeOpenGLContextCurrent();
            m_windows.erase(it);
            w->addRepaintFull();
        }
        return;
    }

    // Shapes are surface-local; the scene item wants frame-local.
    const QPointF offset = w->bufferGeometry().topLeft() - w->frameGeometry().topLeft();
    RectF bounds;
    for (const Shape &shape : glass->shapes()) {
        bounds |= RectF(Rect(shape.rect)).translated(offset);
    }

    WindowGlass &data = m_windows[w];
    if (!data.item) {
        data.item = std::make_unique<BackgroundEffectItem>(w->windowItem());
    }
    // Refraction and frost read the background a little way off each pixel, so repaints
    // under the glass have to reach that far.
    const int blurReach = m_settings.blurOffset * (1 << m_settings.blurPasses) * 2;
    data.item->setPixelsToExpandRepaintsBelowOpaqueRegions(std::ceil(m_settings.liquidRefraction) + blurReach);
    data.item->setEffectBoundingRect(bounds);
    w->addRepaintFull();
}

void GlassEffect::prePaintScreen(ScreenPrePaintData &data)
{
    m_currentView = data.view;
    effects->prePaintScreen(data);
}

void GlassEffect::drawWindow(const RenderTarget &renderTarget, const RenderViewport &viewport, EffectWindow *w, int mask, const Region &deviceRegion, WindowPaintData &data)
{
    paintGlass(renderTarget, viewport, w, deviceRegion, data);
    effects->drawWindow(renderTarget, viewport, w, mask, deviceRegion, data);
}

bool GlassEffect::ensureTextures(ShapeRender &render, const QSize &size, GLenum format, int levels)
{
    if (int(render.textures.size()) == levels && render.textures[0]->size() == size && render.textures[0]->internalFormat() == format) {
        return true;
    }
    render.framebuffers.clear();
    render.textures.clear();
    for (int i = 0; i < levels; ++i) {
        auto texture = GLTexture::allocate(format, QSize(std::max(1, size.width() >> i), std::max(1, size.height() >> i)));
        if (!texture) {
            qCWarning(MACSHELL_GLASS) << "failed to allocate a glass texture";
            render.textures.clear();
            render.framebuffers.clear();
            return false;
        }
        texture->setFilter(GL_LINEAR);
        texture->setWrapMode(GL_CLAMP_TO_EDGE);
        auto framebuffer = std::make_unique<GLFramebuffer>(texture.get());
        if (!framebuffer->valid()) {
            qCWarning(MACSHELL_GLASS) << "failed to create a glass framebuffer";
            render.textures.clear();
            render.framebuffers.clear();
            return false;
        }
        render.textures.push_back(std::move(texture));
        render.framebuffers.push_back(std::move(framebuffer));
    }
    return true;
}

/// Uploads a quad covering a whole `size` texture, for offscreen passes.
static void uploadQuad(GLVertexBuffer *vbo, const QSize &size)
{
    vbo->reset();
    vbo->setAttribLayout(std::span(GLVertexBuffer::GLVertex2DLayout), sizeof(GLVertex2D));
    if (auto result = vbo->map<GLVertex2D>(6)) {
        auto map = *result;
        const float w = size.width();
        const float h = size.height();
        map[0] = GLVertex2D{.position = QVector2D(0, 0), .texcoord = QVector2D(0, 1)};
        map[1] = GLVertex2D{.position = QVector2D(w, h), .texcoord = QVector2D(1, 0)};
        map[2] = GLVertex2D{.position = QVector2D(0, h), .texcoord = QVector2D(0, 0)};
        map[3] = GLVertex2D{.position = QVector2D(0, 0), .texcoord = QVector2D(0, 1)};
        map[4] = GLVertex2D{.position = QVector2D(w, 0), .texcoord = QVector2D(1, 1)};
        map[5] = GLVertex2D{.position = QVector2D(w, h), .texcoord = QVector2D(1, 0)};
        vbo->unmap();
    }
}

void GlassEffect::frost(ShapeRender &render)
{
    // Dual Kawase, as in KWin's blur: halve the background each pass, then double it back up
    // to level 1, which the glass samples from.
    GLVertexBuffer *vbo = GLVertexBuffer::streamingBuffer();
    uploadQuad(vbo, render.textures[0]->size());
    vbo->bindArrays();

    QMatrix4x4 projection;
    projection.ortho(QRectF(QPointF(), QSizeF(render.textures[0]->size())));

    ShaderManager::instance()->pushShader(m_downsampleShader.get());
    m_downsampleShader->setUniform(GLShader::Mat4Uniform::ModelViewProjectionMatrix, projection);
    m_downsampleShader->setUniform("offset", m_settings.blurOffset);
    for (size_t i = 1; i < render.framebuffers.size(); ++i) {
        GLTexture *read = render.textures[i - 1].get();
        m_downsampleShader->setUniform("halfpixel", QVector2D(0.5 / read->width(), 0.5 / read->height()));
        read->bind();
        GLFramebuffer::pushFramebuffer(render.framebuffers[i].get());
        vbo->draw(GL_TRIANGLES, 0, 6);
        GLFramebuffer::popFramebuffer();
    }
    ShaderManager::instance()->popShader();

    ShaderManager::instance()->pushShader(m_upsampleShader.get());
    m_upsampleShader->setUniform(GLShader::Mat4Uniform::ModelViewProjectionMatrix, projection);
    m_upsampleShader->setUniform("offset", m_settings.blurOffset);
    for (size_t i = render.framebuffers.size() - 1; i > 1; --i) {
        GLTexture *read = render.textures[i].get();
        m_upsampleShader->setUniform("halfpixel", QVector2D(0.5 / read->width(), 0.5 / read->height()));
        read->bind();
        GLFramebuffer::pushFramebuffer(render.framebuffers[i - 1].get());
        vbo->draw(GL_TRIANGLES, 0, 6);
        GLFramebuffer::popFramebuffer();
    }
    ShaderManager::instance()->popShader();

    vbo->unbindArrays();
}

float GlassEffect::measureLuminance(ShapeRender &render, bool linear)
{
    if (!m_probeTexture) {
        m_probeTexture = GLTexture::allocate(GL_RGBA8, s_probeSize);
        if (!m_probeTexture) {
            return -1;
        }
        m_probeTexture->setFilter(GL_LINEAR);
        m_probeFramebuffer = std::make_unique<GLFramebuffer>(m_probeTexture.get());
    }

    // Each probe pixel averages a few background pixels; spread over the whole shape, that's
    // plenty to tell light from dark.
    GLFramebuffer::pushFramebuffer(render.framebuffers[0].get());
    m_probeFramebuffer->blitFromFramebuffer(Rect(QPoint(), render.textures[0]->size()), Rect(QPoint(), s_probeSize));
    GLFramebuffer::popFramebuffer();

    const QImage image = m_probeTexture->toImage().convertToFormat(QImage::Format_RGBA8888);
    double total = 0;
    for (int y = 0; y < image.height(); ++y) {
        const uchar *line = image.constScanLine(y);
        for (int x = 0; x < image.width(); ++x) {
            float rgb[3];
            for (int c = 0; c < 3; ++c) {
                const float value = line[x * 4 + c] / 255.0f;
                // A float target holds linear light already; 8-bit ones hold sRGB.
                rgb[c] = linear ? value : srgbToLinear(value);
            }
            total += 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        }
    }
    return total / (image.width() * image.height());
}

void GlassEffect::paintGlass(const RenderTarget &renderTarget, const RenderViewport &viewport, EffectWindow *w, const Region &deviceRegion, const WindowPaintData &data)
{
    auto it = m_windows.find(w);
    if (it == m_windows.end() || !w->surface()) {
        return;
    }
    GlassSurface *glass = m_manager->glassFor(w->surface());
    if (!glass) {
        return;
    }
    const QList<Shape> &shapes = glass->shapes();
    std::vector<ShapeRender> &renders = it->second.render[m_currentView];
    renders.resize(shapes.size());

    const GLenum format = renderTarget.texture() ? renderTarget.texture()->internalFormat() : GL_RGBA8;
    const bool linear = format == GL_RGBA16F || format == GL_RGBA32F;
    const QPointF offset = w->bufferGeometry().topLeft() - w->frameGeometry().topLeft();
    const float opacity = w->opacity() * data.opacity();
    const float scale = std::min(data.xScale(), data.yScale());

    for (qsizetype i = 0; i < shapes.size(); ++i) {
        const Shape &shape = shapes[i];
        const bool probe = i == 0 && glass->tracksBrightness();
        if (shape.style == Style::Transparent && !probe) {
            continue;
        }

        // The shape in global logical coordinates, following the window's transform like blur.
        RectF local = RectF(Rect(shape.rect)).translated(offset);
        local = RectF(local.x() * data.xScale() + data.xTranslation(),
                      local.y() * data.yScale() + data.yTranslation(),
                      local.width() * data.xScale(),
                      local.height() * data.yScale());
        const RectF logical = local.translated(w->pos());
        const Rect deviceRect = viewport.mapToDeviceCoordinatesAligned(logical);
        if (deviceRect.isEmpty()) {
            continue;
        }

        ShapeRender &render = renders[i];
        const bool blurred = shape.style == Style::Frosted || shape.style == Style::Liquid;
        const int levels = blurred ? m_settings.blurPasses + 1 : 1;
        const bool fresh = render.textures.empty() || render.textures[0]->size() != deviceRect.size();
        if (!ensureTextures(render, deviceRect.size(), format, levels)) {
            continue;
        }

        // Copy the part of the background that was just repainted; the rest is cached from
        // earlier frames. Copying more would pick up last frame's glass.
        const Rect logicalBounds = logical.roundedOut();
        const Region logicalDirty = deviceRegion == Region::infinite()
            ? Region(logicalBounds)
            : viewport.mapFromDeviceCoordinatesContained(deviceRegion) & logicalBounds;
        for (const Rect &dirty : logicalDirty.rects()) {
            const Rect destination = viewport.mapToDeviceCoordinatesAligned(dirty).translated(-deviceRect.topLeft());
            render.framebuffers[0]->blitFromRenderTarget(renderTarget, viewport, dirty, destination);
        }
        if (fresh && logicalDirty.boundingRect() != logicalBounds) {
            // New textures start out empty; fill them in on the next frame.
            effects->addRepaint(logicalBounds);
        }

        if (probe) {
            if (const int wait = glass->msUntilProbe(); wait == 0) {
                const float luminance = measureLuminance(render, linear);
                if (luminance >= 0) {
                    glass->reportBrightness(luminance);
                }
            } else if (!it->second.probeScheduled) {
                // Measure again once it's due, in case nothing else repaints by then.
                it->second.probeScheduled = true;
                QTimer::singleShot(wait, this, [this, window = QPointer<EffectWindow>(w), logicalBounds]() {
                    if (auto it = m_windows.find(window.data()); window && it != m_windows.end()) {
                        it->second.probeScheduled = false;
                    }
                    effects->addRepaint(logicalBounds);
                });
            }
        }
        if (shape.style == Style::Transparent) {
            continue;
        }

        if (blurred) {
            frost(render);
        }

        // Draw where the window is being repainted, in device pixels relative to the shape.
        const Region deviceDirty = deviceRegion == Region::infinite() ? Region(deviceRect) : deviceRegion & deviceRect;
        if (deviceDirty.isEmpty()) {
            continue;
        }
        GLVertexBuffer *vbo = GLVertexBuffer::streamingBuffer();
        vbo->reset();
        vbo->setAttribLayout(std::span(GLVertexBuffer::GLVertex2DLayout), sizeof(GLVertex2D));
        const int vertexCount = deviceDirty.rects().size() * 6;
        if (auto result = vbo->map<GLVertex2D>(vertexCount)) {
            auto map = *result;
            size_t index = 0;
            const float width = deviceRect.width();
            const float height = deviceRect.height();
            for (const Rect &rect : deviceDirty.rects()) {
                const RectF r = RectF(rect.translated(-deviceRect.topLeft()));
                const auto vertex = [&](float x, float y) {
                    return GLVertex2D{.position = QVector2D(x, y), .texcoord = QVector2D(x / width, 1.0f - y / height)};
                };
                map[index++] = vertex(r.left(), r.top());
                map[index++] = vertex(r.right(), r.bottom());
                map[index++] = vertex(r.left(), r.bottom());
                map[index++] = vertex(r.left(), r.top());
                map[index++] = vertex(r.right(), r.top());
                map[index++] = vertex(r.right(), r.bottom());
            }
            vbo->unmap();
        } else {
            qCWarning(MACSHELL_GLASS) << "failed to map the vertex buffer";
            continue;
        }
        vbo->bindArrays();

        QMatrix4x4 projection = viewport.projectionMatrix();
        projection.translate(deviceRect.x(), deviceRect.y());

        const bool frosted = shape.style == Style::Frosted;
        const bool liquid = shape.style == Style::Liquid;
        const float saturation = frosted ? m_settings.frostSaturation : liquid ? m_settings.liquidSaturation : 1.0f;
        ShaderManager::instance()->pushShader(m_glassShader.get());
        m_glassShader->setUniform(GLShader::Mat4Uniform::ModelViewProjectionMatrix, projection);
        m_glassShader->setUniform("geoSize", QVector2D(logical.width(), logical.height()));
        m_glassShader->setUniform("cornerRadius", float(shape.radius * scale));
        m_glassShader->setUniform("deviceScale", float(viewport.scale()));
        // Clear glass is liquid glass without the blur or the darkening.
        m_glassShader->setUniform("rimWidth", frosted ? 0.0f : m_settings.liquidRim);
        m_glassShader->setUniform("refraction", m_settings.liquidRefraction);
        m_glassShader->setUniform("brightness", liquid ? m_settings.liquidBrightness : 1.0f);
        m_glassShader->setUniform("colorMatrix", saturationMatrix(saturation));
        // Measured from Tahoe's menus: a dark tint with a hint of purple, under the edge lines.
        m_glassShader->setUniform("tint", frosted ? QVector4D(0.09, 0.06, 0.1, m_settings.frostTint) : QVector4D());
        // Menus whiten evenly along two pixels; liquid glass brightens its own color, mostly in
        // the outer pixel, and more where it faces the light.
        m_glassShader->setUniform("edgeBoost", frosted ? 1.0f : m_settings.liquidEdgeBoost);
        m_glassShader->setUniform("edgeWhite", frosted ? m_settings.frostEdge : m_settings.liquidEdgeWhite);
        m_glassShader->setUniform("edgeInner", frosted ? 1.0f : 0.35f);
        m_glassShader->setUniform("edgeLit", frosted ? 0.0f : 0.8f);
        m_glassShader->setUniform("outerAlpha", frosted ? m_settings.frostOuterEdge : 0.0f);
        m_glassShader->setUniform("opacity", opacity);

        render.textures[blurred ? 1 : 0]->bind();
        glEnable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
        vbo->draw(GL_TRIANGLES, 0, vertexCount);
        glDisable(GL_BLEND);
        ShaderManager::instance()->popShader();
        vbo->unbindArrays();
    }
}

bool GlassEffect::isActive() const
{
    return m_valid && !m_windows.empty() && !effects->isScreenLocked();
}

bool GlassEffect::blocksDirectScanout() const
{
    return false;
}

}

#include "moc_glasseffect.cpp"
