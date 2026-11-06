/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#pragma once

#include <KDecoration3/Decoration>
#include <KDecoration3/DecorationButton>
#include <KDecoration3/DecorationButtonGroup>

#include <QPointer>
#include <QTimer>
#include <QVariant>

namespace Macshell
{

/// A window's title bar, the way Tahoe draws it: traffic lights on the left, whatever the user's
/// button layout says, a centered title, and rounded corners all the way around.
class Decoration : public KDecoration3::Decoration
{
    Q_OBJECT

public:
    explicit Decoration(QObject *parent = nullptr, const QVariantList &args = QVariantList());

    bool init() override;
    void paint(QPainter *painter, const QRectF &repaintArea) override;

    /// Whether the pointer's over any of the traffic lights. They all show their glyphs then.
    bool buttonsHovered() const;

private:
    void updateGeometry();
    void updateButtons();
    void updateShadow();
    void zoomHoverChanged(bool hovered);

    KDecoration3::DecorationButtonGroup *m_buttons = nullptr;
    QPointer<KDecoration3::DecorationButton> m_zoom;
    QTimer m_zoomHover;
    bool m_zoomMenuOpen = false;
};

/// One of the traffic lights.
class Button : public KDecoration3::DecorationButton
{
    Q_OBJECT

public:
    Button(KDecoration3::DecorationButtonType type, Decoration *decoration, QObject *parent = nullptr);

    void paint(QPainter *painter, const QRectF &repaintArea) override;
};

}
