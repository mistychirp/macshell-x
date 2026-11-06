/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#include "glasseffect.h"
#include "arranger.h"
#include "window.h"
#include "glassmanager.h"

#include "core/rendertarget.h"
#include "core/renderviewport.h"
#include "effect/effecthandler.h"
#include "opengl/eglcontext.h"
#include "scene/backgroundeffectitem.h"
#include "wayland/surface.h"

#include <KConfigGroup>
#include <KSharedConfig>

#include <KGlobalAccel>

#include <QAction>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCall>

#include <QLoggingCategory>
#include <QMatrix4x4>
#include <QPointer>
#include <QTimer>

#include <algorithm>
#include <cmath>
#include <map>
#include <numbers>

Q_LOGGING_CATEGORY(MACSHELL_GLASS, "macshell_glass", QtWarningMsg)

using namespace KWin;

namespace Macshell
{

/// The app ID of the shell's dialogs, which the window manager places and keeps on top.
static constexpr QLatin1StringView s_dialogAppId("macshell-alert");

/// How long a new surface stays out of sight if its glass doesn't come.
static constexpr int s_awaitGlassMs = 200;

static bool isDialog(Style style)
{
    return style == Style::Window || style == Style::WindowInactive;
}

/// Brightness probes downsample the background to this many pixels before reading it back.
static constexpr QSize s_probeSize(32, 4);
/// How far past the edges of glass to repaint when it changes, in logical pixels.
static constexpr int s_edgeRepaint = 2;
/// How long glass a client let go of stays if its window turns out not to be closing.
static constexpr int s_releasedGlassMs = 1000;

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
    m_arranger = std::make_unique<Arranger>();
    connect(m_manager.get(), &GlassManager::glassChanged, this, [this](SurfaceInterface *surface) {
        if (EffectWindow *w = effects->findWindow(surface)) {
            updateWindow(w);
        }
    });

    // Glass can arrive before its window maps, and windows can go without dropping it.
    connect(effects, &EffectsHandler::windowAdded, this, &GlassEffect::placeDialog);
    connect(effects, &EffectsHandler::windowAdded, this, [this](EffectWindow *w) {
        if (w->surface() && m_manager->usesGlass(w->surface())) {
            m_awaitingGlass[w].start();
        }
        updateWindow(w);
    });
    connect(effects, &EffectsHandler::windowClosed, this, [this](EffectWindow *w) {
        if (auto it = m_windows.find(w); it != m_windows.end()) {
            it->second.closing = true;
            if (it->second.appear.kind != 0 && !effects->isScreenLocked()) {
                it->second.deletedRef.emplace(w);
                it->second.disappearing.start();
            }
            w->addRepaintFull();
            effects->addRepaintFull();
        }
    });
    connect(effects, &EffectsHandler::windowDeleted, this, [this](EffectWindow *w) {
        m_awaitingGlass.erase(w);
        if (auto it = m_windows.find(w); it != m_windows.end()) {
            effects->makeOpenGLContextCurrent();
            m_windows.erase(it);
            effects->addRepaintFull();
        }
    });
    connect(effects, &EffectsHandler::viewRemoved, this, [this](RenderView *view) {
        effects->makeOpenGLContextCurrent();
        for (auto &[window, glass] : m_windows) {
            glass.render.erase(view);
        }
    });

    m_saveSoon = new QTimer(this);
    m_saveSoon->setSingleShot(true);
    m_saveSoon->setInterval(500);
    connect(m_saveSoon, &QTimer::timeout, this, [] {
        KSharedConfig::openConfig(QStringLiteral("kwinrc"))->sync();
    });
    QDBusConnection::sessionBus().registerObject(QStringLiteral("/org/macshell/Glass"), this, QDBusConnection::ExportScriptableSlots);

    // Spotlight's shortcut. It's here because global shortcuts belong to whatever registers them, and
    // KWin's are the ones that can be changed along with all the others.
    auto *spotlight = new QAction(this);
    spotlight->setObjectName(QStringLiteral("Spotlight"));
    spotlight->setText(QStringLiteral("Spotlight Search"));
    KGlobalAccel::self()->setDefaultShortcut(spotlight, {Qt::META | Qt::Key_Space});
    KGlobalAccel::self()->setShortcut(spotlight, {Qt::META | Qt::Key_Space});
    connect(spotlight, &QAction::triggered, this, [] {
        const auto message = QDBusMessage::createMethodCall(QStringLiteral("org.macshell.Shell"),
                                                            QStringLiteral("/org/macshell/Windows"),
                                                            QStringLiteral("org.macshell.Windows"),
                                                            QStringLiteral("ToggleSpotlight"));
        QDBusConnection::sessionBus().asyncCall(message);
    });

    reconfigure(ReconfigureAll);
    for (EffectWindow *w : effects->stackingOrder()) {
        updateWindow(w);
    }
    m_valid = true;
}

GlassEffect::~GlassEffect()
{
    QDBusConnection::sessionBus().unregisterObject(QStringLiteral("/org/macshell/Glass"));
    effects->makeOpenGLContextCurrent();
    // Letting go of a closed window deletes it and its items, and tells us about it. So the items go
    // first, and the windows once nothing here refers to them.
    std::vector<EffectWindowDeletedRef> closed;
    for (auto &[window, glass] : m_windows) {
        glass.item.reset();
        glass.render.clear();
        if (glass.deletedRef) {
            closed.push_back(std::move(*glass.deletedRef));
            glass.deletedRef.reset();
        }
    }
    m_windows.clear();
}

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
    m_settings.frostPasses = std::clamp(group.readEntry("FrostPasses", defaults.frostPasses), 1, 5);
    m_settings.frostOffset = group.readEntry("FrostOffset", defaults.frostOffset);
    m_settings.frostSaturation = group.readEntry("FrostSaturation", defaults.frostSaturation);
    m_settings.frostTint = group.readEntry("FrostTint", defaults.frostTint);
    m_settings.frostEdgeWhite = group.readEntry("FrostEdgeWhite", defaults.frostEdgeWhite);
    m_settings.frostOuterEdge = group.readEntry("FrostOuterEdge", defaults.frostOuterEdge);
    m_settings.liquidBrightness = group.readEntry("LiquidBrightness", defaults.liquidBrightness);
    m_settings.liquidSaturation = group.readEntry("LiquidSaturation", defaults.liquidSaturation);
    m_settings.liquidEdgeFade = group.readEntry("LiquidEdgeFade", defaults.liquidEdgeFade);
    m_settings.liquidEdgeGain = group.readEntry("LiquidEdgeGain", defaults.liquidEdgeGain);
    m_settings.controlTint = group.readEntry("ControlTint", defaults.controlTint);
    m_settings.controlTintGrey = group.readEntry("ControlTintGrey", defaults.controlTintGrey);
    m_settings.controlSaturation = group.readEntry("ControlSaturation", defaults.controlSaturation);
    m_settings.controlRim = group.readEntry("ControlRim", defaults.controlRim);
    m_settings.backdropBrightness = group.readEntry("BackdropBrightness", defaults.backdropBrightness);
    m_settings.grain = group.readEntry("Grain", defaults.grain);
    m_settings.rimTint = group.readEntry("RimTint", defaults.rimTint);
    m_settings.glassIor = group.readEntry("GlassIor", defaults.glassIor);
    m_settings.glassIorSpread = group.readEntry("GlassIorSpread", defaults.glassIorSpread);
    m_settings.glassProfile = group.readEntry("GlassProfile", defaults.glassProfile);
    m_settings.glassShoulder = group.readEntry("GlassShoulder", defaults.glassShoulder);
    m_settings.glassGloss = group.readEntry("GlassGloss", defaults.glassGloss);
    m_settings.glassThickness = group.readEntry("GlassThickness", defaults.glassThickness);
    m_settings.glassBezel = group.readEntry("GlassBezel", defaults.glassBezel);
    m_settings.inactiveTint = group.readEntry("InactiveTint", defaults.inactiveTint);
    m_settings.chipTint = group.readEntry("ChipTint", defaults.chipTint);
    m_settings.chipRim = group.readEntry("ChipRim", defaults.chipRim);
    m_settings.bubbleTint = group.readEntry("BubbleTint", defaults.bubbleTint);
    m_settings.bubbleEdgeGain = group.readEntry("BubbleEdgeGain", defaults.bubbleEdgeGain);
    effects->addRepaintFull();
}

/// What the Settings app may change, besides the green button's click, which the decoration reads.
static const std::map<QString, float GlassSettings::*> s_tunable = {
    {QStringLiteral("GlassThickness"), &GlassSettings::glassThickness},
    {QStringLiteral("GlassBezel"), &GlassSettings::glassBezel},
    {QStringLiteral("GlassIor"), &GlassSettings::glassIor},
    {QStringLiteral("GlassIorSpread"), &GlassSettings::glassIorSpread},
    {QStringLiteral("GlassProfile"), &GlassSettings::glassProfile},
    {QStringLiteral("GlassGloss"), &GlassSettings::glassGloss},
    {QStringLiteral("Grain"), &GlassSettings::grain},
};
static const QString s_greenFullScreen = QStringLiteral("GreenFullScreen");

static KConfigGroup settingsGroup()
{
    return KSharedConfig::openConfig(QStringLiteral("kwinrc"))->group(QStringLiteral("Effect-macshell"));
}

double GlassEffect::setting(const QString &key) const
{
    if (key == s_greenFullScreen) {
        return settingsGroup().readEntry(key, 0.0);
    }
    const auto found = s_tunable.find(key);
    return found == s_tunable.end() ? 0.0 : m_settings.*(found->second);
}

void GlassEffect::setSetting(const QString &key, double value)
{
    if (key != s_greenFullScreen && !s_tunable.contains(key)) {
        return;
    }
    settingsGroup().writeEntry(key, value);
    m_saveSoon->start();
    reconfigure(ReconfigureAll);
}

void GlassEffect::resetSettings()
{
    KConfigGroup group = settingsGroup();
    for (const auto &[key, member] : s_tunable) {
        group.deleteEntry(key);
    }
    m_saveSoon->start();
    reconfigure(ReconfigureAll);
}

void GlassEffect::placeDialog(EffectWindow *w)
{
    if (!w->windowClass().contains(s_dialogAppId, Qt::CaseInsensitive)) {
        return;
    }
    // Clients can't place their windows or keep them on top. A dialog has no business being lost
    // behind others or listed among the apps.
    Window *window = w->window();
    window->setKeepAbove(true);
    window->setSkipTaskbar(true);
    window->setSkipSwitcher(true);
    window->setSkipPager(true);
    const RectF area = effects->clientArea(MaximizeArea, effects->activeScreen());
    window->move(area.center() - QPointF(w->width(), w->height()) / 2);
}

void GlassEffect::windowMoved(EffectWindow *w, const RectF &oldGeometry)
{
    auto it = m_windows.find(w);
    if (it == m_windows.end() || it->second.lastBounds.isEmpty()) {
        return;
    }
    const QPointF moved = w->frameGeometry().topLeft() - oldGeometry.topLeft();
    const Rect before = it->second.lastBounds;
    const Rect after = RectF(before).translated(moved).roundedOut();
    it->second.lastBounds = after;
    const Rect touched = before | after;
    effects->addRepaint(Rect(touched.x() - s_edgeRepaint, touched.y() - s_edgeRepaint, touched.width() + 2 * s_edgeRepaint, touched.height() + 2 * s_edgeRepaint));
}

void GlassEffect::updateWindow(EffectWindow *w)
{
    GlassSurface *glass = w->surface() ? m_manager->glassFor(w->surface()) : nullptr;
    if (glass && !glass->shapes().isEmpty()) {
        m_awaitingGlass.erase(w);
    }
    if (!glass) {
        // A client lets go of its glass as it closes the surface; that glass still has a closing
        // animation to fade out in. If the window turns out to be staying, it goes after all.
        if (m_windows.contains(w) && !w->isDeleted()) {
            QTimer::singleShot(s_releasedGlassMs, this, [this, window = QPointer<EffectWindow>(w)]() {
                auto it = m_windows.find(window.data());
                if (!window || it == m_windows.end() || it->second.closing || window->isDeleted()
                    || (window->surface() && m_manager->glassFor(window->surface()))) {
                    return;
                }
                effects->makeOpenGLContextCurrent();
                m_windows.erase(it);
                window->addRepaintFull();
                effects->addRepaintFull();
            });
        }
        return;
    }
    if (glass->shapes().isEmpty()) {
        if (auto it = m_windows.find(w); it != m_windows.end()) {
            effects->makeOpenGLContextCurrent();
            m_windows.erase(it);
            w->addRepaintFull();
            // At a fractional scale, the edge of glass can land in a row of device pixels that the
            // window's own repaint misses, and stay on screen until something else repaints there.
            effects->addRepaintFull();
        }
        return;
    }

    // Shapes are surface-local; the scene item wants frame-local.
    const QPointF offset = w->bufferGeometry().topLeft() - w->frameGeometry().topLeft();
    RectF bounds;
    int passes = m_settings.blurPasses;
    float blurOffset = m_settings.blurOffset;
    for (const Shape &shape : glass->shapes()) {
        bounds |= RectF(Rect(shape.rect)).translated(offset);
        if (shape.style == Style::Frosted || isDialog(shape.style)) {
            passes = m_settings.frostPasses;
            blurOffset = m_settings.frostOffset;
        }
    }

    WindowGlass &data = m_windows[w];
    data.shapes = glass->shapes();
    if (glass->appear().kind != 0 && !data.appearing.isValid() && !w->isDeleted()) {
        data.appear = glass->appear();
        data.appearing.start();
    }
    if (!data.item) {
        data.item = std::make_unique<BackgroundEffectItem>(w->windowItem());
        connect(w, &EffectWindow::windowFrameGeometryChanged, this, &GlassEffect::windowMoved, Qt::UniqueConnection);
    }
    // The lens and the blur read the background well away from each pixel, so repaints under the
    // glass have to reach that far.
    const int blurReach = blurOffset * (1 << passes) * 2;
    data.item->setPixelsToExpandRepaintsBelowOpaqueRegions(std::ceil(m_settings.glassThickness * 2.4f) + blurReach);
    data.item->setEffectBoundingRect(bounds);
    w->addRepaintFull();

    // Past the edges of where the glass was and now is, for the same reason as when it goes away.
    const Rect now = bounds.translated(w->pos()).roundedOut();
    const Rect touched = data.lastBounds.isEmpty() ? now : now | data.lastBounds;
    data.lastBounds = now;
    effects->addRepaint(Rect(touched.x() - s_edgeRepaint, touched.y() - s_edgeRepaint, touched.width() + 2 * s_edgeRepaint, touched.height() + 2 * s_edgeRepaint));
}

/// How a surface looks at some point of coming or going.
struct AppearState {
    float scale = 1;
    float opacity = 1;
    float slide = 0;
    bool done = true;
};

/// A spring from 0 to 1, in the terms Apple describes them in: how long it takes to get most of the
/// way there, and how little it overshoots, with 1 being not at all.
static float spring(float seconds, float response, float damping)
{
    const float omega = 2 * std::numbers::pi_v<float> / response;
    if (damping >= 1) {
        return 1 - std::exp(-omega * seconds) * (1 + omega * seconds);
    }
    const float decay = damping * omega;
    const float ringing = omega * std::sqrt(1 - damping * damping);
    return 1 - std::exp(-decay * seconds) * (std::cos(ringing * seconds) + decay / ringing * std::sin(ringing * seconds));
}

static AppearState appearState(const WindowGlass &glass)
{
    AppearState state;
    if (glass.appear.kind == 0) {
        return state;
    }
    constexpr int spring_ = 1;
    constexpr int slide = 3;
    if (glass.disappearing.isValid()) {
        const float seconds = glass.disappearing.elapsed() / 1000.0f;
        const float duration = glass.appear.kind == slide ? 0.22f : 0.14f;
        const float t = std::min(seconds / duration, 1.0f);
        const float out = t * t;
        state.opacity = glass.appear.kind == slide ? 1 - out * out : 1 - t;
        state.scale = glass.appear.kind == slide ? 1 : 1 - 0.06f * out;
        state.slide = glass.appear.kind == slide ? out : 0;
        state.done = t >= 1;
        return state;
    }
    const float seconds = glass.appearing.elapsed() / 1000.0f;
    switch (glass.appear.kind) {
    case spring_: {
        // Springs out of the point, overshooting a little.
        const float value = spring(seconds, 0.36f, 0.78f);
        state.scale = 0.82f + 0.18f * value;
        state.opacity = std::min(seconds / 0.1f, 1.0f);
        state.done = seconds > 0.75f;
        break;
    }
    case slide: {
        const float value = spring(seconds, 0.32f, 1.0f);
        state.slide = 1 - value;
        state.done = seconds > 0.55f;
        break;
    }
    default: {
        const float value = spring(seconds, 0.2f, 1.0f);
        state.scale = 0.95f + 0.05f * value;
        state.opacity = std::min(seconds / 0.09f, 1.0f);
        state.done = seconds > 0.34f;
        break;
    }
    }
    return state;
}

void GlassEffect::prePaintScreen(ScreenPrePaintData &data)
{
    m_currentView = data.view;
    for (const auto &[window, glass] : m_windows) {
        if (!appearState(glass).done) {
            data.mask |= PAINT_SCREEN_WITH_TRANSFORMED_WINDOWS;
            break;
        }
    }
    effects->prePaintScreen(data);
}

void GlassEffect::paintScreen(const RenderTarget &renderTarget, const RenderViewport &viewport, int mask, const Region &deviceRegion, LogicalOutput *screen)
{
    m_screenTarget = &renderTarget;
    effects->paintScreen(renderTarget, viewport, mask, deviceRegion, screen);
    m_screenTarget = nullptr;
}

void GlassEffect::prePaintWindow(RenderView *view, EffectWindow *w, WindowPrePaintData &data)
{
    if (m_awaitingGlass.contains(w)) {
        data.setTranslucent();
    }
    if (auto it = m_windows.find(w); it != m_windows.end() && !appearState(it->second).done) {
        data.setTranslucent();
        data.setTransformed();
    }
    effects->prePaintWindow(view, w, data);
}

void GlassEffect::paintWindow(const RenderTarget &renderTarget, const RenderViewport &viewport, EffectWindow *w, int mask, const Region &deviceRegion, WindowPaintData &data)
{
    if (auto it = m_awaitingGlass.find(w); it != m_awaitingGlass.end()) {
        if (it->second.elapsed() < s_awaitGlassMs) {
            data.multiplyOpacity(0);
        } else {
            m_awaitingGlass.erase(it);
        }
    }
    if (auto it = m_windows.find(w); it != m_windows.end() && it->second.appear.kind != 0) {
        const AppearState state = appearState(it->second);
        if (!state.done || it->second.disappearing.isValid()) {
            const QPointF origin = it->second.appear.origin;
            data.multiplyOpacity(state.opacity);
            data.setXScale(data.xScale() * state.scale);
            data.setYScale(data.yScale() * state.scale);
            data.translate(origin.x() * (1 - state.scale) + state.slide * w->width(), origin.y() * (1 - state.scale));
        }
    }
    effects->paintWindow(renderTarget, viewport, w, mask, deviceRegion, data);
}

void GlassEffect::postPaintScreen()
{
    // Letting go of a closed window can delete it, which changes m_windows, so that waits for the
    // end of the loop.
    if (!m_awaitingGlass.empty()) {
        effects->addRepaintFull();
    }
    std::vector<EffectWindowDeletedRef> finished;
    for (auto &[window, glass] : m_windows) {
        if (!appearState(glass).done) {
            effects->addRepaintFull();
        } else if (glass.disappearing.isValid() && glass.deletedRef) {
            finished.push_back(std::move(*glass.deletedRef));
            glass.deletedRef.reset();
            effects->addRepaintFull();
        }
    }
    finished.clear();
    // A closing window only repaints itself, but glass reaches a little past it at a fractional
    // scale. Without this, that row of pixels stays as it was until the window's gone.
    for (const auto &[window, glass] : m_windows) {
        if (glass.closing && !glass.lastBounds.isEmpty()) {
            const Rect bounds = glass.lastBounds;
            effects->addRepaint(Rect(bounds.x() - s_edgeRepaint, bounds.y() - s_edgeRepaint, bounds.width() + 2 * s_edgeRepaint, bounds.height() + 2 * s_edgeRepaint));
        }
    }
    effects->postPaintScreen();
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

void GlassEffect::frost(ShapeRender &render, int passes, float offset)
{
    // Dual Kawase, as in KWin's blur: halve the background each pass, then double it back up. All
    // the way down, then back up to the first level past the blur's own for the wide blur; then
    // from the blur's own last level, which that didn't touch, up to level 1 for the blur.
    GLVertexBuffer *vbo = GLVertexBuffer::streamingBuffer();
    uploadQuad(vbo, render.textures[0]->size());
    vbo->bindArrays();

    QMatrix4x4 projection;
    projection.ortho(QRectF(QPointF(), QSizeF(render.textures[0]->size())));

    ShaderManager::instance()->pushShader(m_downsampleShader.get());
    m_downsampleShader->setUniform(GLShader::Mat4Uniform::ModelViewProjectionMatrix, projection);
    m_downsampleShader->setUniform("offset", offset);
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
    m_upsampleShader->setUniform("offset", offset);
    const auto upsample = [&](size_t from, size_t to) {
        for (size_t i = from; i > to; --i) {
            GLTexture *read = render.textures[i].get();
            m_upsampleShader->setUniform("halfpixel", QVector2D(0.5 / read->width(), 0.5 / read->height()));
            read->bind();
            GLFramebuffer::pushFramebuffer(render.framebuffers[i - 1].get());
            vbo->draw(GL_TRIANGLES, 0, 6);
            GLFramebuffer::popFramebuffer();
        }
    };
    upsample(render.framebuffers.size() - 1, passes + 1);
    upsample(passes, 1);
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
    if (it == m_windows.end()) {
        return;
    }
    // Gone once the surface is, while the window finishes closing.
    GlassSurface *glass = w->surface() ? m_manager->glassFor(w->surface()) : nullptr;
    const QList<Shape> &shapes = it->second.shapes;
    std::vector<ShapeRender> &renders = it->second.render[m_currentView];
    renders.resize(shapes.size());

    const GLenum format = renderTarget.texture() ? renderTarget.texture()->internalFormat() : GL_RGBA8;
    const bool linear = format == GL_RGBA16F || format == GL_RGBA32F;
    const QPointF offset = w->bufferGeometry().topLeft() - w->frameGeometry().topLeft();
    const float opacity = w->opacity() * data.opacity();
    const float scale = std::min(data.xScale(), data.yScale());

    for (qsizetype i = 0; i < shapes.size(); ++i) {
        const Shape &shape = shapes[i];
        const bool probe = glass && i == 0 && glass->tracksBrightness();
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

        const bool frosted = shape.style == Style::Frosted || isDialog(shape.style);
        const bool control = shape.style == Style::Control;
        ShapeRender &render = renders[i];
        // Controls sit on a backdrop, which has done the blurring for them.
        const bool blurred = shape.style != Style::Clear && shape.style != Style::Transparent && !control;
        const int passes = frosted ? m_settings.frostPasses : m_settings.blurPasses;
        // Beyond the blur's own levels: four more halvings for the wide blur.
        const int wideLevel = passes + 1;
        const int levels = blurred ? wideLevel + 4 : 1;
        // Nothing behind it here to read, so it keeps what it has, which may be a different size.
        const bool offscreen = &renderTarget != m_screenTarget;
        if (offscreen && render.textures.empty()) {
            continue;
        }
        const bool fresh = render.textures.empty() || render.textures[0]->size() != deviceRect.size();
        if (!offscreen && !ensureTextures(render, deviceRect.size(), format, levels)) {
            continue;
        }

        // Copy the part of the background that was just repainted; the rest is cached from
        // earlier frames. Copying more would pick up last frame's glass.
        const Rect logicalBounds = logical.roundedOut();
        const Region logicalDirty = offscreen ? Region()
            : deviceRegion == Region::infinite()
            ? Region(logicalBounds)
            : viewport.mapFromDeviceCoordinatesContained(deviceRegion) & logicalBounds;
        for (const Rect &dirty : logicalDirty.rects()) {
            const Rect destination = viewport.mapToDeviceCoordinatesAligned(dirty).translated(-deviceRect.topLeft());
            render.framebuffers[0]->blitFromRenderTarget(renderTarget, viewport, dirty, destination);
        }
        if (!offscreen && fresh && logicalDirty.boundingRect() != logicalBounds) {
            // New textures start out empty; fill them in on the next frame.
            effects->addRepaint(logicalBounds);
        }

        if (probe && !offscreen) {
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

        if (blurred && !offscreen) {
            frost(render, passes, frosted ? m_settings.frostOffset : m_settings.blurOffset);
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
        // The projection is in global device coordinates, and the rectangle is relative to the target.
        const QPointF targetOrigin = viewport.renderRect().topLeft() * viewport.scale();
        projection.translate(deviceRect.x() + targetOrigin.x(), deviceRect.y() + targetOrigin.y());

        const bool liquid = shape.style == Style::Liquid;
        const bool bubble = shape.style == Style::Bubble;
        const bool chip = shape.style == Style::Chip;
        const bool backdrop = shape.style == Style::Backdrop;
        const float saturation = control ? m_settings.controlSaturation : liquid ? m_settings.liquidSaturation : blurred ? m_settings.frostSaturation : 1.0f;
        const float tintAlpha = shape.style == Style::WindowInactive ? m_settings.inactiveTint
            : frosted || shape.style == Style::Card                  ? m_settings.frostTint
            : bubble                                                 ? m_settings.bubbleTint
            : control                                                ? m_settings.controlTint
                                                                     : 0.0f;
        ShaderManager::instance()->pushShader(m_glassShader.get());
        m_glassShader->setUniform(GLShader::Mat4Uniform::ModelViewProjectionMatrix, projection);
        m_glassShader->setUniform("geoSize", QVector2D(logical.width(), logical.height()));
        m_glassShader->setUniform("cornerRadius", float(shape.radius * scale));
        m_glassShader->setUniform("deviceScale", float(viewport.scale()));
        m_glassShader->setUniform("wide", 1);
        m_glassShader->setUniform("sharp", 2);
        m_glassShader->setUniform("brightness", backdrop ? m_settings.backdropBrightness : liquid ? m_settings.liquidBrightness : 1.0f);
        m_glassShader->setUniform("colorMatrix", saturationMatrix(saturation));
        // Measured from Tahoe: menus have a dark tint with a hint of purple, under the edge, and its
        // small buttons a light grey one.
        const QVector3D tintColor = chip   ? QVector3D(0.21, 0.23, 0.27)
            : control                       ? QVector3D(m_settings.controlTintGrey, m_settings.controlTintGrey, m_settings.controlTintGrey)
                                            : QVector3D(0.09, 0.06, 0.1);
        m_glassShader->setUniform("tint", QVector4D(tintColor, chip ? m_settings.chipTint : tintAlpha));
        m_glassShader->setUniform("rim", control ? m_settings.controlRim : chip ? m_settings.chipRim : 0.0f);
        m_glassShader->setUniform("rimDirectional", control ? 1.0f : 0.0f);
        m_glassShader->setUniform("rimInner", 1.0f);
        m_glassShader->setUniform("rimSoft", control ? 1.0f : 0.0f);
        const bool lens = control || liquid || bubble || shape.style == Style::Card;
        m_glassShader->setUniform("thickness", lens ? m_settings.glassThickness : 0.0f);
        m_glassShader->setUniform("bezel", m_settings.glassBezel);
        m_glassShader->setUniform("shoulder", m_settings.glassShoulder);
        m_glassShader->setUniform("profile", m_settings.glassProfile);
        m_glassShader->setUniform("ior", m_settings.glassIor);
        m_glassShader->setUniform("iorSpread", m_settings.glassIorSpread);
        m_glassShader->setUniform("gloss", m_settings.glassGloss);
        m_glassShader->setUniform("grain", blurred || control ? m_settings.grain : 0.0f);
        m_glassShader->setUniform("rimTint", m_settings.rimTint);
        m_glassShader->setUniform("softEdges", shape.softEdges);
        m_glassShader->setUniform("tail", QVector3D(shape.tail.x() * data.xScale(), shape.tail.y() * data.xScale(), shape.tail.z() * data.yScale()));
        m_glassShader->setUniform("edgeFade", frosted ? 0.0f : m_settings.liquidEdgeFade);
        m_glassShader->setUniform("edgeGain", bubble ? m_settings.bubbleEdgeGain : frosted ? 1.0f : m_settings.liquidEdgeGain);
        m_glassShader->setUniform("edgeWhite", frosted ? m_settings.frostEdgeWhite : 0.0f);
        m_glassShader->setUniform("outerAlpha", frosted ? m_settings.frostOuterEdge : 0.0f);
        m_glassShader->setUniform("opacity", opacity);

        glActiveTexture(GL_TEXTURE2);
        render.textures[0]->bind();
        glActiveTexture(GL_TEXTURE1);
        render.textures[blurred ? wideLevel : 0]->bind();
        glActiveTexture(GL_TEXTURE0);
        render.textures[chip ? wideLevel : blurred ? 1 : 0]->bind();
        glEnable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
        vbo->draw(GL_TRIANGLES, 0, vertexCount);
        glDisable(GL_BLEND);
        glActiveTexture(GL_TEXTURE2);
        glBindTexture(GL_TEXTURE_2D, 0);
        glActiveTexture(GL_TEXTURE1);
        glBindTexture(GL_TEXTURE_2D, 0);
        glActiveTexture(GL_TEXTURE0);
        ShaderManager::instance()->popShader();
        vbo->unbindArrays();
    }
}

bool GlassEffect::isActive() const
{
    return m_valid && (!m_windows.empty() || !m_awaitingGlass.empty()) && !effects->isScreenLocked();
}

bool GlassEffect::blocksDirectScanout() const
{
    return false;
}

}

#include "moc_glasseffect.cpp"
