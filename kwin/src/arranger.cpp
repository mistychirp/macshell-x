/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#include "arranger.h"
#include "window.h"

#include "effect/effecthandler.h"
#include "effect/effectwindow.h"

#include <QDBusConnection>
#include <QUuid>

#include <algorithm>

using namespace KWin;

namespace Macshell
{

static const QString s_path = QStringLiteral("/org/macshell/Arranger");

Arranger::Arranger(QObject *parent)
    : QObject(parent)
{
    QDBusConnection::sessionBus().registerObject(s_path, this, QDBusConnection::ExportScriptableSlots);
}

Arranger::~Arranger()
{
    QDBusConnection::sessionBus().unregisterObject(s_path);
}

static bool canBeArranged(const EffectWindow *w)
{
    return w && w->isOnCurrentDesktop() && !w->isMinimized() && !w->isDeleted()
        && (w->isNormalWindow() || w->isDialog());
}

QString Arranger::windowUnderCursor(int &x, int &y)
{
    x = 0;
    y = 0;
    const QPointF cursor = effects->cursorPos();
    const QList<EffectWindow *> order = effects->stackingOrder();
    for (auto it = order.rbegin(); it != order.rend(); ++it) {
        EffectWindow *w = *it;
        if (canBeArranged(w) && w->frameGeometry().contains(cursor)) {
            const RectF frame = w->frameGeometry();
            x = int(frame.x());
            y = int(frame.y());
            return w->internalId().toString();
        }
    }
    return QString();
}

void Arranger::fullScreenUnderCursor()
{
    int x;
    int y;
    if (EffectWindow *w = effects->findWindow(QUuid(windowUnderCursor(x, y)))) {
        w->window()->setFullScreen(true);
    }
}

/// Takes a window out of anything that would stop it going somewhere, and puts it there.
static void place(EffectWindow *w, const RectF &rect)
{
    Window *window = w->window();
    if (window->isFullScreen()) {
        window->setFullScreen(false);
    }
    window->maximize(MaximizeRestore);
    window->moveResize(rect);
}

void Arranger::arrange(const QString &id, const QString &layout)
{
    EffectWindow *w = effects->findWindow(QUuid(id));
    if (!canBeArranged(w)) {
        return;
    }
    if (layout == QLatin1String("fullscreen")) {
        w->window()->setFullScreen(true);
        return;
    }

    const RectF area = effects->clientArea(MaximizeArea, w);
    const qreal halfWidth = area.width() / 2;
    const qreal halfHeight = area.height() / 2;
    const RectF left(area.x(), area.y(), halfWidth, area.height());
    const RectF right(area.x() + halfWidth, area.y(), halfWidth, area.height());
    const RectF topRight(area.x() + halfWidth, area.y(), halfWidth, halfHeight);
    const RectF bottomRight(area.x() + halfWidth, area.y() + halfHeight, halfWidth, halfHeight);
    const RectF topLeft(area.x(), area.y(), halfWidth, halfHeight);
    const RectF bottomLeft(area.x(), area.y() + halfHeight, halfWidth, halfHeight);

    QList<RectF> places;
    if (layout == QLatin1String("left")) {
        places = {left};
    } else if (layout == QLatin1String("right")) {
        places = {right};
    } else if (layout == QLatin1String("top")) {
        places = {RectF(area.x(), area.y(), area.width(), halfHeight)};
    } else if (layout == QLatin1String("bottom")) {
        places = {RectF(area.x(), area.y() + halfHeight, area.width(), halfHeight)};
    } else if (layout == QLatin1String("fill")) {
        places = {area};
    } else if (layout == QLatin1String("two")) {
        places = {left, right};
    } else if (layout == QLatin1String("three")) {
        places = {left, topRight, bottomRight};
    } else if (layout == QLatin1String("four")) {
        places = {topLeft, topRight, bottomLeft, bottomRight};
    } else {
        return;
    }

    place(w, places.takeFirst());
    // The rest go to the windows that were most recently in front.
    const QList<EffectWindow *> order = effects->stackingOrder();
    for (auto it = order.rbegin(); it != order.rend() && !places.isEmpty(); ++it) {
        if (*it != w && canBeArranged(*it) && (*it)->window()->isResizable()) {
            place(*it, places.takeFirst());
        }
    }
}

}

#include "moc_arranger.cpp"
