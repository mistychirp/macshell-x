# macshell-x

A macOS Tahoe-style desktop for [KWin](https://invent.kde.org/plasma/kwin), KDE's compositor,
running without the rest of Plasma.

The plan has two halves:

- **Shell** (`crates/shell`): dock, menu bar, control center and widgets. It's built with iced on
  layer-shell and talks to KWin over Wayland protocols and D-Bus. It runs on stock KWin 6.
- **KWin plugins** (`kwin`): for things the shell can't do from outside, such as the liquid glass
  shader, titlebars with traffic lights and minimize animations. So far: liquid glass.

## Development

```sh
nix develop
cargo run
```

## Running on KWin

On NixOS, the flake's module adds a "macshell" login session: KWin with the shell and its glass
plugin, both built from the system's own packages, so the plugin always matches the KWin it loads
into. The session shares KWin's configuration (`kwinrc`, `kxkbrc`, shortcuts) with Plasma.

```nix
# In your flake: inputs.macshell.url = "github:mistychirp/macshell-x";
imports = [ inputs.macshell.nixosModules.default ];
programs.macshell.enable = true;
```

Elsewhere, install the desktop entry yourself. KWin only lets the shell follow and manage windows
if a desktop entry grants it, with
`X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` and an `Exec` that is the absolute
path of the shell binary. Install [contrib/macshell.desktop](contrib/macshell.desktop) with
`@macshell@` replaced by that path, somewhere in `$XDG_DATA_DIRS/applications`.

Keyboard layouts come from KWin's `org.kde.keyboard` D-Bus interface.

## Configuration

`~/.config/macshell/config.toml`:

```toml
# Desktop entry id of the app that opens folders and the trash. Defaults to the system's
# default app for folders (`xdg-mime query default inode/directory`).
file_manager = "org.kde.dolphin"

[desktop]
# Wallpaper image (PNG or JPEG), filling the screen. Without one, the shell leaves the background
# to whatever else draws it; KWin doesn't draw one itself.
wallpaper = "~/Pictures/wallpaper.png"

[dock]
# Desktop entry ids (the .desktop file name without the extension), in dock order.
# The dock rewrites this when you drag icons around or use Keep in Dock.
pinned = ["org.kde.dolphin", "vivaldi-stable", "Alacritty"]
# Grow icons toward the pointer, up to this size (icons are 48).
magnification = true
magnified_size = 80

[menubar]
# Icon name or path of the logo that opens the system menu, drawn as a white silhouette.
# Defaults to the distro's logo (`LOGO` in /etc/os-release).
logo = "nix-snowflake"
```

The shell chooses the material behind its surfaces itself, so it needs no compositor rules:

- **Liquid glass** where the compositor offers our `macshell-glass-v1` protocol
  ([protocols/macshell-glass-v1.xml](protocols/macshell-glass-v1.xml)).
- **Blur** on stock KWin 6 and other compositors with `ext-background-effect-v1`.
- **Nothing** elsewhere; the dock then keeps a faint fill and border.

## KWin plugin

`kwin/` is a KWin effect that renders liquid glass for the shell. It serves our
`macshell-glass-v1` protocol: clear, liquid, frosted, bubble and control glass shapes behind surfaces, transparent
shapes that only measure, and brightness tracking for the menu bar's text color.

KWin plugins must be built against the exact KWin they load into. The flake exposes the plugin as
`packages.kwin-macshell`; to build it against your system's KWin, use the nixpkgs your system
uses:

```sh
nix build .#kwin-macshell --override-input nixpkgs github:NixOS/nixpkgs/<your system's rev>
```

KWin finds it through `QT_PLUGIN_PATH` (its `lib/qt-6/plugins`). It's enabled by default once
found. The glass can be tuned in `kwinrc`; these are the defaults:

```ini
[Effect-macshell]
# The blur under frosted and liquid glass: dual Kawase passes and their sample offset.
BlurPasses=2
BlurOffset=2
# The lens of liquid glass and bubbles: how far it bends the background, in logical pixels, at one
# logical pixel from the edge, and how far in it reaches.
LensStrength=32
LensRadius=24
# Frosted glass (menus, popovers, alerts): saturation, how strong its dark tint is, how much the
# light line along the edge, which otherwise shows the surroundings, whitens, and the dark line
# outside it.
FrostSaturation=1.5
FrostTint=0.58
FrostEdgeWhite=0.12
FrostOuterEdge=0.8
# Liquid glass (the dock): how much it darkens the background, saturation, and its edge, which
# fades into the surroundings over this many logical pixels, brightened by the gain.
LiquidBrightness=0.89
LiquidSaturation=1.15
LiquidEdgeFade=2
LiquidEdgeGain=1.06
# Control modules (Control Center): how much they brighten the background, saturation, and how much
# the rim along the edge whitens where it catches the light.
ControlBrightness=1.15
ControlSaturation=1.1
ControlRim=0.55
# Chips (small buttons, like a notification's close button): how strong the grey tint is, and how
# much the rim whitens.
ChipTint=0.6
ChipRim=0.15
# Backdrops (behind Notification Center's message when it's empty): how far in from each side the
# blur takes to reach full strength.
BackdropFadeLeft=190
BackdropFadeTop=30
BackdropFadeRight=100
BackdropFadeBottom=110
# Bubbles (the dock's labels): how strong the dark tint is, and how the edge is dimmed.
BubbleTint=0.5
BubbleEdgeGain=0.85
```

To try it without leaving your session, run KWin nested. KWin needs `wp_single_pixel_buffer_manager_v1`
and pointer constraints from its host, so a nested KWin can't run inside niri, but works inside a
nested sway.

## Notes

- We depend on iced's component crates (`iced_widget`, `iced_futures`, `iced_renderer`) instead of
  `iced`, because `iced` hard-depends on winit and layer-shell surfaces don't use it. That means
  re-enabling by hand what `iced` would turn on: the `wgpu-bare` renderer feature (without it you
  silently get CPU rendering) and the `image` crate's PNG codec.
- `iced_layershell` is vendored with a small patch (see [vendor/README.md](vendor/README.md)).
- `winit-core` / `winit-common` are pinned to `0.31.0-beta.2` in `Cargo.lock`, because
  `iced_exdevtools` 0.19.1 doesn't compile against beta.3. Don't `cargo update` them until
  iced_layershell ships a fix.
