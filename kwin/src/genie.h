/*
    SPDX-FileCopyrightText: 2026 the makshell contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#pragma once

#include "effect/effectwindow.h"
#include "effect/offscreeneffect.h"
#include "effect/timeline.h"

#include <QHash>
#include <QRect>
#include <QSet>
#include <QUuid>

namespace Makshell
{

/// The genie: a minimized window pours into its place in the dock, and pours back out when it's
/// restored. It deforms the live window on a grid. The shell says where the places are, over D-Bus.
class GenieEffect : public KWin::OffscreenEffect
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.makshell.Genie")

public:
    GenieEffect();
    ~GenieEffect() override;

    static bool supported();

    void prePaintScreen(KWin::ScreenPrePaintData &data) override;
    void prePaintWindow(KWin::RenderView *view, KWin::EffectWindow *w, KWin::WindowPrePaintData &data) override;
    void postPaintScreen() override;
    bool isActive() const override;

    int requestedEffectChainPosition() const override
    {
        // Before the glass, so that it paints into what gets deformed, and the window's frame and
        // contents move together.
        return 19;
    }

public Q_SLOTS:
    /// Where a minimized window's tile is, on the screen. With an empty id, where the next window to
    /// be minimized will go.
    Q_SCRIPTABLE void setTarget(const QString &id, int x, int y, int width, int height);
    /// This window goes and comes back with no animation, next time: an app being hidden has nowhere
    /// in the dock to go to.
    Q_SCRIPTABLE void skip(const QString &id);

protected:
    void apply(KWin::EffectWindow *window, int mask, KWin::WindowPaintData &data, KWin::WindowQuadList &quads) override;

private:
    struct Animation {
        KWin::EffectWindowVisibleRef visibleRef;
        KWin::TimeLine timeLine;
        /// Where it's going, from when it started, so that it doesn't jump as the dock makes room.
        QRectF target;
    };

    void windowAdded(KWin::EffectWindow *w);
    void animate(KWin::EffectWindow *w, bool minimizing);
    QRectF targetFor(KWin::EffectWindow *w) const;

    QHash<KWin::EffectWindow *, Animation> m_animations;
    QHash<QUuid, QRect> m_targets;
    QRect m_nextTarget;
    QSet<QUuid> m_skipped;
};

}
