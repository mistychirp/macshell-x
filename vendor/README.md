# Vendored crates

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

The first commit that adds this directory contains the pristine 0.19.1 sources, so
`git diff <that commit> -- vendor/iced_layershell` shows the patch. Drop the vendored copy
once upstream ships the fix.
