/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

import QtQuick
import org.kde.kirigami as Kirigami
import org.kde.kwin as KWin

// What Alt+Tab shows, like macOS's: the running apps' icons in a row, on glass, with the name of the
// one that's selected under it. Its colors are its own, so that it doesn't matter what the Plasma theme
// and the color scheme are, which on their own can come out white on white.
KWin.TabBoxSwitcher {
    id: tabBox

    Instantiator {
        active: tabBox.visible

        delegate: Window {
            id: window

            // The glass effect goes by this, to put glass behind it.
            title: "macshell-switcher"
            flags: Qt.Popup | Qt.X11BypassWindowManagerHint
            color: "transparent"
            visible: true

            readonly property int padding: 14
            readonly property int labelHeight: 30
            readonly property int largestCell: 124
            readonly property int slots: Math.max(icons.count, 1)
            readonly property int cell: Math.min(largestCell,
                Math.floor((tabBox.screenGeometry.width * 0.9 - 2 * padding) / slots))

            width: Math.max(cell * slots, 240) + 2 * padding
            height: cell + labelHeight + 2 * padding
            x: tabBox.screenGeometry.x + (tabBox.screenGeometry.width - width) / 2
            y: tabBox.screenGeometry.y + (tabBox.screenGeometry.height - height) / 2

            // What apps put in their titles goes before their names, as in "A file — Kate".
            function appName(caption) {
                return caption.split(/ [—–-] /).pop().trim();
            }

            ListView {
                id: icons

                x: Math.round((window.width - contentWidth) / 2)
                y: window.padding
                width: window.width - 2 * window.padding
                height: window.cell
                orientation: ListView.Horizontal
                interactive: false
                keyNavigationEnabled: true
                keyNavigationWraps: true
                focus: true
                model: tabBox.model
                currentIndex: tabBox.currentIndex
                highlightMoveDuration: 0

                highlight: Rectangle {
                    color: Qt.rgba(1, 1, 1, 0.16)
                    radius: Math.round(window.cell * 0.17)
                }

                delegate: MouseArea {
                    readonly property string name: window.appName(model.caption)

                    width: window.cell
                    height: window.cell

                    onClicked: tabBox.model.activate(index)

                    Kirigami.Icon {
                        anchors.centerIn: parent
                        width: Math.round(window.cell * 0.82)
                        height: width
                        source: model.icon
                    }
                }

                onCurrentIndexChanged: tabBox.currentIndex = currentIndex
            }

            Text {
                // Under the selected one, as far as there's room.
                x: {
                    const under = icons.x + (icons.currentIndex + 0.5) * window.cell - width / 2;
                    return Math.round(Math.max(window.padding,
                        Math.min(under, window.width - window.padding - width)));
                }
                y: window.padding + window.cell + 4
                width: Math.min(implicitWidth, window.width - 2 * window.padding)
                height: window.labelHeight - 4
                verticalAlignment: Text.AlignVCenter
                color: "white"
                elide: Text.ElideRight
                textFormat: Text.PlainText
                font.family: "Inter"
                font.pixelSize: 15
                font.weight: Font.DemiBold
                text: icons.currentItem ? icons.currentItem.name
                    : icons.count === 0 ? "No Open Windows" : ""
            }

            onSceneGraphError: () => {
                // This has to be here: without a handler, QtQuick ends the whole compositor over a
                // graphics reset.
            }
        }
    }
}
