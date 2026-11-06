# The macshell shell, with the desktop entry KWin needs to let it manage windows.
{
  lib,
  rustPlatform,
  runtimeShell,
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
    cat > $out/bin/macshell-settings <<EOF
    #!${runtimeShell}
    exec $out/bin/macshell settings "\$@"
    EOF
    chmod +x $out/bin/macshell-settings

    # KWin only reads the first desktop entry naming an executable, so a second one for this binary
    # can take window management away from the shell.
    entries=$(grep -lE "^Exec=$out/bin/macshell( |\$)" $out/share/applications/*.desktop | wc -l)
    if [ "$entries" != 1 ]; then
      echo "expected one desktop entry to run $out/bin/macshell, found $entries" >&2
      exit 1
    fi
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
