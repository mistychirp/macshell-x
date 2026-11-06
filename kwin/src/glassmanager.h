/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#pragma once

#include "qwayland-server-macshell-glass-v1.h"

#include <QElapsedTimer>
#include <QHash>
#include <QList>
#include <QObject>
#include <QPointer>
#include <QRect>
#include <QVector3D>
#include <QVector4D>

#include <optional>

namespace KWin
{
class Display;
class SurfaceInterface;
}

namespace Macshell
{

enum class Style {
    Clear,
    Frosted,
    /// Nothing is drawn; the shape only marks where brightness is measured.
    Transparent,
    Liquid,
    Bubble,
    Control,
    Backdrop,
    Card,
    Chip,
    /// Frosted, for dialogs, which have a lighter tint when they aren't active.
    Window,
    WindowInactive,
};

struct Shape {
    /// Surface-local logical rectangle.
    QRect rect;
    int radius = 0;
    Style style = Style::Clear;
    /// For a backdrop: how far it fades in from the left, top, right and bottom.
    QVector4D softEdges;
    /// For a shape with a tail: where its tip is from the left, how wide it is and how tall. It's
    /// part of `rect`, whose bottom it takes up.
    QVector3D tail;
};

class GlassSurface;

/// The macshell_glass_manager_v1 global: hands out a glass object per surface.
class GlassManager : public QObject, public QtWaylandServer::macshell_glass_manager_v1
{
    Q_OBJECT

public:
    explicit GlassManager(KWin::Display *display, QObject *parent = nullptr);

    GlassSurface *glassFor(KWin::SurfaceInterface *surface) const;

Q_SIGNALS:
    /// The surface's glass shapes changed (applied on commit), or its glass went away.
    void glassChanged(KWin::SurfaceInterface *surface);

protected:
    void macshell_glass_manager_v1_destroy(Resource *resource) override;
    void macshell_glass_manager_v1_get_glass(Resource *resource, uint32_t id, struct ::wl_resource *surface) override;

private:
    friend class GlassSurface;
    QHash<KWin::SurfaceInterface *, GlassSurface *> m_surfaces;
};

/// A surface's macshell_glass_v1 object: its double-buffered shapes and brightness tracking.
class GlassSurface : public QObject, public QtWaylandServer::macshell_glass_v1
{
    Q_OBJECT

public:
    GlassSurface(GlassManager *manager, KWin::SurfaceInterface *surface, wl_client *client, uint32_t id, int version);
    ~GlassSurface() override;

    const QList<Shape> &shapes() const;
    bool tracksBrightness() const;

    /// How long until the next brightness measurement is due; 0 when it's due now.
    int msUntilProbe() const;
    /// Takes a measurement, as relative luminance from 0 to 1, and sends it to the client if
    /// it changed noticeably.
    void reportBrightness(float luminance);

protected:
    void macshell_glass_v1_destroy_resource(Resource *resource) override;
    void macshell_glass_v1_destroy(Resource *resource) override;
    void macshell_glass_v1_clear(Resource *resource) override;
    void macshell_glass_v1_add_rounded_rect(Resource *resource, int32_t x, int32_t y, int32_t width, int32_t height, uint32_t radius, uint32_t style) override;
    void macshell_glass_v1_add_backdrop(Resource *resource, int32_t x, int32_t y, int32_t width, int32_t height, uint32_t fade_left, uint32_t fade_top, uint32_t fade_right, uint32_t fade_bottom) override;
    void macshell_glass_v1_add_rounded_rect_with_tail(Resource *resource, int32_t x, int32_t y, int32_t width, int32_t height, uint32_t radius, uint32_t style, int32_t tail_center, uint32_t tail_width, uint32_t tail_height) override;
    void macshell_glass_v1_add_rounded_rect_with_top_tail(Resource *resource, int32_t x, int32_t y, int32_t width, int32_t height, uint32_t radius, uint32_t style, int32_t tail_center, uint32_t tail_width, uint32_t tail_height) override;
    void macshell_glass_v1_track_brightness(Resource *resource) override;

private:
    void detach();

    QPointer<GlassManager> m_manager;
    QPointer<KWin::SurfaceInterface> m_surface;
    QList<Shape> m_pending;
    QList<Shape> m_current;
    bool m_dirty = false;

    bool m_tracking = false;
    QElapsedTimer m_lastProbe;
    /// The last luminance sent, out of 1000.
    std::optional<uint32_t> m_sent;
};

}
