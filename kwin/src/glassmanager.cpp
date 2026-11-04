/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#include "glassmanager.h"

#include "wayland/display.h"
#include "wayland/surface.h"

#include <algorithm>

namespace Macshell
{

static constexpr int s_version = 2;

/// Reading pixels back stalls the GPU, so brightness is measured at most this often.
static constexpr int s_probeIntervalMs = 500;
/// How much the luminance (out of 1000) has to change before the client hears about it.
static constexpr uint32_t s_brightnessStep = 20;

GlassManager::GlassManager(KWin::Display *display, QObject *parent)
    : QObject(parent)
    , QtWaylandServer::macshell_glass_manager_v1(*display, s_version)
{
}

GlassSurface *GlassManager::glassFor(KWin::SurfaceInterface *surface) const
{
    return m_surfaces.value(surface);
}

void GlassManager::macshell_glass_manager_v1_destroy(Resource *resource)
{
    wl_resource_destroy(resource->handle);
}

void GlassManager::macshell_glass_manager_v1_get_glass(Resource *resource, uint32_t id, struct ::wl_resource *surface)
{
    KWin::SurfaceInterface *surf = KWin::SurfaceInterface::get(surface);
    if (m_surfaces.contains(surf)) {
        wl_resource_post_error(resource->handle, error_glass_exists, "the surface already has a glass object");
        return;
    }
    m_surfaces.insert(surf, new GlassSurface(this, surf, resource->client(), id, resource->version()));
}

GlassSurface::GlassSurface(GlassManager *manager, KWin::SurfaceInterface *surface, wl_client *client, uint32_t id, int version)
    : QtWaylandServer::macshell_glass_v1(client, id, version)
    , m_manager(manager)
    , m_surface(surface)
{
    // Shapes are double-buffered: they take effect on the surface's next commit.
    connect(surface, &KWin::SurfaceInterface::committed, this, [this]() {
        if (!m_dirty) {
            return;
        }
        m_dirty = false;
        m_current = m_pending;
        if (m_manager && m_surface) {
            Q_EMIT m_manager->glassChanged(m_surface);
        }
    });
    // Inert once the surface is gone. (The QPointer is already null when this fires.)
    connect(surface, &QObject::destroyed, this, [this, surface]() {
        if (m_manager) {
            m_manager->m_surfaces.remove(surface);
        }
    });
}

GlassSurface::~GlassSurface()
{
    detach();
}

void GlassSurface::detach()
{
    if (m_manager && m_surface) {
        m_manager->m_surfaces.remove(m_surface);
        m_current.clear();
        Q_EMIT m_manager->glassChanged(m_surface);
    }
    m_surface = nullptr;
}

const QList<Shape> &GlassSurface::shapes() const
{
    return m_current;
}

bool GlassSurface::tracksBrightness() const
{
    return m_tracking;
}

int GlassSurface::msUntilProbe() const
{
    if (!m_lastProbe.isValid()) {
        return 0;
    }
    return std::max<qint64>(0, s_probeIntervalMs - m_lastProbe.elapsed());
}

void GlassSurface::reportBrightness(float luminance)
{
    m_lastProbe.start();
    const uint32_t value = std::clamp(luminance, 0.0f, 1.0f) * 1000.0f + 0.5f;
    if (m_sent && std::abs(int(*m_sent) - int(value)) < int(s_brightnessStep)) {
        return;
    }
    m_sent = value;
    send_brightness(value);
}

void GlassSurface::macshell_glass_v1_destroy_resource(Resource *resource)
{
    Q_UNUSED(resource)
    delete this;
}

void GlassSurface::macshell_glass_v1_destroy(Resource *resource)
{
    detach();
    wl_resource_destroy(resource->handle);
}

void GlassSurface::macshell_glass_v1_clear(Resource *resource)
{
    Q_UNUSED(resource)
    m_pending.clear();
    m_dirty = true;
}

void GlassSurface::macshell_glass_v1_add_rounded_rect(Resource *resource, int32_t x, int32_t y, int32_t width, int32_t height, uint32_t radius, uint32_t style)
{
    if (width <= 0 || height <= 0) {
        wl_resource_post_error(resource->handle, error_invalid_shape, "glass shapes need a positive width and height");
        return;
    }
    Style shapeStyle = Style::Clear;
    if (style == style_frosted) {
        shapeStyle = Style::Frosted;
    } else if (style == style_transparent) {
        shapeStyle = Style::Transparent;
    } else if (style == style_liquid) {
        shapeStyle = Style::Liquid;
    } else if (style == style_bubble) {
        shapeStyle = Style::Bubble;
    } else if (style == style_control) {
        shapeStyle = Style::Control;
    }
    m_pending.append(Shape{
        .rect = QRect(x, y, width, height),
        .radius = int(std::min<uint32_t>(radius, std::min(width, height) / 2)),
        .style = shapeStyle,
    });
    m_dirty = true;
}

void GlassSurface::macshell_glass_v1_track_brightness(Resource *resource)
{
    Q_UNUSED(resource)
    m_tracking = true;
}

}
