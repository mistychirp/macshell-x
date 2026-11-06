# The macshell KWin effect: liquid glass for the shell, through our macshell-glass-v1 protocol.

cmake_minimum_required(VERSION 3.25)
project(kwin-macshell LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(ECM 6.0 REQUIRED NO_MODULE)
list(APPEND CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)

# Everything KWin::kwin links, which its CMake package leaves to us to find.
find_package(Qt6 REQUIRED COMPONENTS Core DBus Gui Quick Widgets)
# ECM's wrapper around qtwaylandscanner, which generates the protocol's server classes.
find_package(QtWaylandScanner REQUIRED)
find_package(KF6 REQUIRED COMPONENTS Config CoreAddons GlobalAccel)
find_package(KWin REQUIRED)
find_package(KDecoration3 REQUIRED)

kcoreaddons_add_plugin(macshell
    SOURCES
        src/main.cpp
        src/glasseffect.cpp
        src/glassmanager.cpp
        src/arranger.cpp
        src/glass.qrc
    INSTALL_NAMESPACE "kwin/effects/plugins"
)

ecm_add_qtwayland_server_protocol(macshell
    PROTOCOL ${CMAKE_CURRENT_SOURCE_DIR}/../protocols/macshell-glass-v1.xml
    BASENAME macshell-glass-v1
)

target_link_libraries(macshell PRIVATE
    KWin::kwin
    KF6::ConfigCore
    KF6::GlobalAccel
    Qt6::Core
    Qt6::DBus
    Qt6::Gui
    Wayland::Server
)

# The genie, a separate effect since it deforms windows off-screen.
kcoreaddons_add_plugin(macshell-genie
    SOURCES
        src/genie.cpp
    INSTALL_NAMESPACE "kwin/effects/plugins"
)
target_link_libraries(macshell-genie PRIVATE
    KWin::kwin
    KF6::ConfigCore
    Qt6::Core
    Qt6::DBus
    Qt6::Gui
)

# The window decoration: traffic lights and a title bar, loaded by KWin separately from the effect.
kcoreaddons_add_plugin(macshell-decoration
    SOURCES
        src/decoration/decoration.cpp
        src/decoration/decoration.qrc
    INSTALL_NAMESPACE "org.kde.kdecoration3"
)
# KWin looks decorations up by the name of their file.
set_target_properties(macshell-decoration PROPERTIES OUTPUT_NAME "org.kde.macshell")
target_link_libraries(macshell-decoration PRIVATE
    KDecoration3::KDecoration
    KF6::ConfigCore
    KF6::CoreAddons
    Qt6::DBus
    Qt6::Gui
)
