//! macshell patch: one opacity for whatever's drawn inside [`multiply`], so that a group of widgets
//! can fade together. Everything the renderer draws is scaled by the current opacity on its way in.

use std::cell::Cell;

thread_local! {
    static OPACITY: Cell<f32> = const { Cell::new(1.0) };
}

/// Draws with the current opacity multiplied by `by`, for as long as `draw` runs.
pub fn multiply<T>(by: f32, draw: impl FnOnce() -> T) -> T {
    let before = OPACITY.get();
    OPACITY.set(before * by);
    let result = draw();
    OPACITY.set(before);
    result
}

pub(crate) fn current() -> f32 {
    OPACITY.get()
}
