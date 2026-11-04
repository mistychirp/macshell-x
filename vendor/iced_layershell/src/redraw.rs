//! macshell patch: which windows an update changed. Upstream redraws every window after any
//! message, which is wasteful with a full-screen wallpaper among them. The application marks
//! the windows it changed from its update function; only those redraw. Widgets that need a
//! redraw of their own, like a hovered button, still ask for it themselves.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::reexport::IcedId;

thread_local! {
    static CHANGED: RefCell<HashSet<IcedId>> = RefCell::new(HashSet::new());
}

/// Marks a window as changed by the current update, so it gets redrawn.
pub fn mark_changed(id: IcedId) {
    CHANGED.with_borrow_mut(|changed| {
        changed.insert(id);
    });
}

/// Takes whether the window was marked since the last time this was asked.
pub(crate) fn take_changed(id: IcedId) -> bool {
    CHANGED.with_borrow_mut(|changed| changed.remove(&id))
}
