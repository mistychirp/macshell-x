//! macshell patch: what ordinary windows need that layer surfaces don't. Upstream can open an
//! `xdg_toplevel`, but gives it no app ID or size limits, and can't start an interactive move.

use std::cell::RefCell;
use std::collections::HashMap;

use layershellev::reexport::wayland_client::WlSeat;
use layershellev::reexport::xdg_toplevel::XdgToplevel;

use crate::actions::IcedXdgWindowSettings;
use crate::reexport::IcedId;

thread_local! {
    static PENDING: RefCell<HashMap<IcedId, IcedXdgWindowSettings>> = RefCell::new(HashMap::new());
    static SEAT: RefCell<Option<WlSeat>> = const { RefCell::new(None) };
    static KEYBOARD_TARGET: RefCell<Option<IcedId>> = const { RefCell::new(None) };
}

/// Sends the keyboard to this window, or stops doing so. `layershellev` sends keys to whichever
/// surface it heard of last, which goes for the pointer entering one too, so without this, typing in
/// a window stops working as soon as the pointer is over anything else.
pub fn set_keyboard_target(window: Option<IcedId>) {
    KEYBOARD_TARGET.with_borrow_mut(|target| *target = window);
}

pub(crate) fn keyboard_target() -> Option<IcedId> {
    KEYBOARD_TARGET.with_borrow(|target| *target)
}

/// The seat that moves are asked for on. `layershellev` keeps its own to itself, so the application
/// binds one on the same connection.
pub fn set_seat(seat: WlSeat) {
    SEAT.with_borrow_mut(|current| *current = Some(seat));
}

pub(crate) fn remember(id: IcedId, settings: IcedXdgWindowSettings) {
    PENDING.with_borrow_mut(|pending| {
        pending.insert(id, settings);
    });
}

/// Applies what was asked for when the window was opened, before its first buffer.
pub(crate) fn apply(id: IcedId, toplevel: &XdgToplevel) {
    let Some(settings) = PENDING.with_borrow_mut(|pending| pending.remove(&id)) else {
        return;
    };
    if let Some(app_id) = settings.app_id {
        toplevel.set_app_id(app_id.to_owned());
    }
    if settings.fixed_size
        && let Some((width, height)) = settings.size
    {
        toplevel.set_min_size(width as i32, height as i32);
        toplevel.set_max_size(width as i32, height as i32);
    }
}

pub(crate) fn start_move(toplevel: &XdgToplevel, serial: u32) {
    SEAT.with_borrow(|seat| match seat {
        Some(seat) => toplevel._move(seat, serial),
        None => tracing::warn!("a window wants to move, but no seat was set"),
    });
}
