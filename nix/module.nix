# NixOS module: a "macshell" login session, KWin with the macshell shell and its glass plugin.
#
# Both are built from the system's own packages, so the plugin always matches the KWin it loads
# into. The session shares KWin's configuration (kwinrc, kxkbrc, shortcuts) with Plasma.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.macshell;

  macshell = pkgs.callPackage ./macshell.nix { };
  kwin-macshell = pkgs.kdePackages.callPackage ../kwin/package.nix { };
  inherit (pkgs.kdePackages) kwin polkit-kde-agent-1;

  # Runs inside KWin for as long as the session lasts: KWin exits when this does.
  startup = pkgs.writeShellScript "macshell-startup" ''
    # Hand KWin's environment (WAYLAND_DISPLAY and friends) to systemd and D-Bus activation,
    # so portals and services started on demand can reach the session.
    ${pkgs.dbus}/bin/dbus-update-activation-environment --systemd --all
    systemctl --user start macshell-session.target

    ${polkit-kde-agent-1}/libexec/polkit-kde-authentication-agent-1 &

    # The shell, restarted if it crashes.
    until ${macshell}/bin/macshell; do
      sleep 1
    done

    systemctl --user stop macshell-session.target
  '';

  session-script = pkgs.writeShellScript "macshell-session" ''
    export XDG_SESSION_TYPE=wayland
    # KDE apps and portals behave as they do in Plasma.
    export XDG_CURRENT_DESKTOP=KDE
    export XDG_SESSION_DESKTOP=KDE
    export KDE_SESSION_VERSION=6
    export QT_PLUGIN_PATH=${kwin-macshell}/lib/qt-6/plugins''${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}

    exec ${kwin}/bin/kwin_wayland_wrapper --xwayland --exit-with-session ${startup}
  '';

  session =
    (pkgs.writeTextDir "share/wayland-sessions/macshell.desktop" ''
      [Desktop Entry]
      Name=macshell
      Comment=KWin with the macshell desktop
      Exec=${session-script}
      Type=Application
      DesktopNames=KDE
    '')
    // {
      providedSessions = [ "macshell" ];
    };
in
{
  options.programs.macshell = {
    enable = lib.mkEnableOption "the macshell desktop, a login session running KWin with macshell";
  };

  config = lib.mkIf cfg.enable {
    services.displayManager.sessionPackages = [ session ];

    environment.systemPackages = [
      # Its desktop entry, which KWin reads to let it manage windows, has to be in the
      # system's data dirs.
      macshell
      kwin-macshell
      # tuigreet lists the sessions it finds in the system profile.
      session
    ];

    systemd.user.targets.macshell-session = {
      description = "macshell session";
      bindsTo = [ "graphical-session.target" ];
      wants = [ "graphical-session-pre.target" ];
      after = [ "graphical-session-pre.target" ];
    };
  };
}
