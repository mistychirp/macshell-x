/*
    SPDX-FileCopyrightText: 2026 the makshell contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#pragma once

#include <QObject>
#include <QString>

namespace Makshell
{

/// Arranges windows for the shell, over D-Bus, on KWin's own service. Clients can't move other
/// clients' windows, but anything running inside KWin can.
class Arranger : public QObject
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.makshell.Arranger")

public:
    explicit Arranger(QObject *parent = nullptr);
    ~Arranger() override;

public Q_SLOTS:
    /// The window under the pointer: its id, and where its top left corner is on the screen. The id
    /// is empty if there's none.
    Q_SCRIPTABLE QString windowUnderCursor(int &x, int &y);

    /// Puts a window somewhere: "left", "right", "top" or "bottom" half of the screen, "fill" it, or
    /// "fullscreen". With "two", "three" or "four", the window shares the screen with as many of the
    /// windows most recently in front of it.
    Q_SCRIPTABLE void arrange(const QString &id, const QString &layout);
    /// Makes the window under the pointer full screen, for the green button's click.
    Q_SCRIPTABLE void fullScreenUnderCursor();
};

}
