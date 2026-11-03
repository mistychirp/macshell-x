# The macshell KWin effect, built against nixpkgs' KWin.
{
  lib,
  stdenv,
  cmake,
  extra-cmake-modules,
  wayland-scanner,
  kwin,
  qtbase,
  qtdeclarative,
  qtwayland,
  kconfig,
  kcoreaddons,
  kwindowsystem,
  wayland,
  libepoxy,
  libdrm,
  vulkan-headers,
}:

stdenv.mkDerivation {
  pname = "kwin-macshell";
  version = "0.1.0";

  # The plugin reads the protocol XML from the repository root.
  src = lib.fileset.toSource {
    root = ./..;
    fileset = lib.fileset.unions [
      ./CMakeLists.txt
      ./src
      ../protocols/macshell-glass-v1.xml
    ];
  };
  sourceRoot = "source/kwin";

  nativeBuildInputs = [
    cmake
    extra-cmake-modules
    wayland-scanner
  ];
  buildInputs = [
    kwin
    qtbase
    qtdeclarative
    qtwayland
    kconfig
    kcoreaddons
    kwindowsystem
    wayland
    libepoxy
    libdrm
    vulkan-headers
  ];

  # A KWin plugin, not an application: nothing to wrap.
  dontWrapQtApps = true;

  meta = {
    description = "Liquid glass for macshell, as a KWin effect";
    license = lib.licenses.gpl3Plus;
    platforms = lib.platforms.linux;
  };
}
