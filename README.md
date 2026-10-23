# macshell-x

A macOS Tahoe-style desktop shell for [niri](https://github.com/niri-wm/niri).

The plan has two halves:

- **Shell** (`crates/shell`): dock, menu bar, control center and widgets. It's built with iced on
  layer-shell and talks to niri over IPC and Wayland protocols. It runs on stock niri.
- **Compositor**: a soft fork of niri, added later for things the shell can't do from outside,
  such as the liquid glass shader, titlebars with traffic lights, minimize and window animations.

## Development

```sh
nix develop
cargo run
```

## Configuration

`~/.config/macshell/config.toml`:

```toml
[dock]
# Desktop entry ids (the .desktop file name without the extension), in dock order.
pinned = ["org.kde.dolphin", "vivaldi-stable", "Alacritty"]
```

The dock asks niri for blur behind its pill itself (`ext-background-effect-v1`, niri 26.04+), so
it needs no config. For other surfaces, you can force blur from your niri config:

```kdl
include "~/Projects/macshell-x/contrib/niri.kdl"
```

## Compositor

The niri fork is a git submodule at `compositor/niri`. It tracks the `macshell` branch of
[mistychirp/niri](https://github.com/mistychirp/niri), which is based on `v26.04`. So far it adds
a `glass` option to `background-effect`; see [contrib/niri-glass.kdl](contrib/niri-glass.kdl).

```sh
git clone --recurse-submodules https://github.com/mistychirp/macshell-x.git
# or, in an existing clone:
git submodule update --init
```

Inside the submodule, `upstream` should point at niri-wm/niri for rebasing onto new releases.

Build it in nixpkgs' niri build environment, not niri's own flake. The flake pins an older
nixpkgs, so its glibc doesn't match the system's Mesa drivers, and EGL fails to load at runtime:

```sh
cd compositor/niri
nix develop nixpkgs#niri --command cargo build
```

To try it without leaving your session, run it nested with a test config. It opens as a window:

```sh
./target/debug/niri -c path/to/test.kdl
```

## Notes

- We depend on iced's component crates (`iced_widget`, `iced_futures`, `iced_renderer`) instead of
  `iced`, because `iced` hard-depends on winit and layer-shell surfaces don't use it. That means
  re-enabling by hand what `iced` would turn on: the `wgpu-bare` renderer feature (without it you
  silently get CPU rendering) and the `image` crate's PNG codec.
- `iced_layershell` is vendored with a small patch (see [vendor/README.md](vendor/README.md)).
- `winit-core` / `winit-common` are pinned to `0.31.0-beta.2` in `Cargo.lock`, because
  `iced_exdevtools` 0.19.1 doesn't compile against beta.3. Don't `cargo update` them until
  iced_layershell ships a fix.
