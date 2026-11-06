# The macshell shell, with the desktop entry KWin needs to let it manage windows.
{
  lib,
  rustPlatform,
  pkg-config,
  libxkbcommon,
  wayland,
  vulkan-loader,
  libGL,
}:

rustPlatform.buildRustPackage {
  pname = "macshell";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ./..;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../assets
      ../crates
      ../vendor
      ../protocols
      ../contrib/macshell.desktop
      ../contrib/macshell-settings.desktop
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ libxkbcommon ];

  postInstall = ''
    install -Dm0644 contrib/macshell.desktop $out/share/applications/macshell.desktop
    substituteInPlace $out/share/applications/macshell.desktop \
      --replace-fail @macshell@ $out/bin/macshell
    install -Dm0644 contrib/macshell-settings.desktop $out/share/applications/macshell-settings.desktop
    substituteInPlace $out/share/applications/macshell-settings.desktop \
      --replace-fail @macshell@ $out/bin/macshell
  '';

  # wgpu and the Wayland client stack load these with dlopen. They go in the binary's runpath
  # rather than a wrapper script: KWin only grants window management to the executable its
  # desktop entry names, so the entry has to name the real binary.
  postFixup = ''
    patchelf --add-rpath ${
      lib.makeLibraryPath [
        wayland
        vulkan-loader
        libGL
        libxkbcommon
      ]
    } $out/bin/macshell
  '';

  meta = {
    description = "A macOS Tahoe-style desktop shell for KWin";
    mainProgram = "macshell";
    platforms = lib.platforms.linux;
  };
}
