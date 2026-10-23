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
# Glass behind the dock: "clear" (refraction only, the default) or "frosted" (over a blur).
glass = "clear"
```

The shell chooses the material behind its surfaces itself, so it needs no compositor rules:

- **Liquid glass** on the macshell niri fork, through our `macshell-glass-v1` protocol
  ([protocols/macshell-glass-v1.xml](protocols/macshell-glass-v1.xml)).
- **Blur** on stock niri 26.04+ and other compositors with `ext-background-effect-v1`.
- **Nothing** elsewhere; the dock then keeps a faint fill and border.

## Compositor

The niri fork is a git submodule at `compositor/niri`. It tracks the `macshell` branch of
[mistychirp/niri](https://github.com/mistychirp/niri), which is based on `v26.04`. So far it adds
liquid glass: the `macshell-glass-v1` protocol, a `glass` option for `background-effect` rules,
and glass settings in the `blur` section. See [contrib/niri-glass.kdl](contrib/niri-glass.kdl)
for tuning it.

The fork keeps a copy of the protocol XML in `resources/`; keep it in sync with `protocols/`.

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
