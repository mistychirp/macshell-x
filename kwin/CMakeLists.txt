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
find_package(KF6 REQUIRED COMPONENTS Config CoreAddons)
find_package(KWin REQUIRED)

kcoreaddons_add_plugin(macshell
    SOURCES
        src/main.cpp
        src/glasseffect.cpp
        src/glassmanager.cpp
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
    Qt6::Core
    Qt6::Gui
    Wayland::Server
)
