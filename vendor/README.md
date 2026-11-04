# Vendored crates

## iced_wgpu 0.14.0 (MIT)

Copied from crates.io and patched in place. Upstream: https://github.com/iced-rs/iced

Patch: skip surface formats with fewer than 8 bits of alpha (`src/window/compositor.rs`). Upstream
takes the first format the driver lists, which on Mesa is `Rgb10a2Unorm`. That has 2 bits of alpha,
so every translucent pixel of a surface, including the edges of text and icons, snaps to 0, 1/3, 2/3
or 1. It doesn't show on an opaque window, but all of the shell's surfaces are translucent.

The first commit that adds this directory contains the pristine 0.14.0 sources, so
`git diff <that commit> -- vendor/iced_wgpu` shows the patch.

## iced_layershell 0.19.1 (MIT)

Copied from crates.io and patched in place. Upstream: https://github.com/waycrate/exwlshelleventloop

Patch: implement `window::Action::Run`, so `iced_runtime::window::run` can reach a layer
surface's raw window handle. Without it the action is silently dropped. Each window now keeps
its `Arc<WindowWrapper>` (`src/multi_window/window_manager.rs`), and `Run` hands it to the
closure (`src/multi_window.rs`).

Patch: redraw only the windows an update changed (`src/redraw.rs`, `src/multi_window.rs`). Upstream
redraws every window after any message, including a full-screen wallpaper at pointer rate. The
application marks changed windows with `redraw::mark_changed`.

Patch: set the cursor whenever the pointer enters a surface (`restore_cursor` in
`src/multi_window.rs`). Upstream only sets it when a surface's interaction changes, so after a window
edge's resize cursor, the desktop and the bars went on showing it.

Patch: map cursors the way the shell wants them (`mouse_interaction` in `src/conversion.rs`). The hand
and grab cursors of iced's buttons, sliders and mouse areas stay the arrow, like on macOS, and `Idle`
is the arrow rather than upstream's busy cursor.

The first commit that adds this directory contains the pristine 0.19.1 sources, so
`git diff <that commit> -- vendor/iced_layershell` shows the patch. Drop the vendored copy
once upstream ships the fix.
