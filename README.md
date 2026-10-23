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

To blur the shell's surfaces on stock niri (26.04+), add this to your niri config:

```kdl
include "~/Projects/macshell-x/contrib/niri.kdl"
```

## Notes

- We depend on iced's component crates (`iced_widget`, `iced_futures`, `iced_renderer`) instead of
  `iced`, because `iced` hard-depends on winit and layer-shell surfaces don't use it.
- `winit-core` / `winit-common` are pinned to `0.31.0-beta.2` in `Cargo.lock`, because
  `iced_exdevtools` 0.19.1 doesn't compile against beta.3. Don't `cargo update` them until
  iced_layershell ships a fix.
