//! Client bindings for KDE's window management protocol
//! (`protocols/plasma-window-management.xml`, from plasma-wayland-protocols 1.21.0).

#![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
#![allow(non_upper_case_globals, non_snake_case, unused_imports)]
#![allow(missing_docs, clippy::all)]

use wayland_client;
use wayland_client::protocol::*;

pub mod __interfaces {
    // The generated interfaces refer to `wayland_backend` by name.
    use wayland_client::backend as wayland_backend;
    use wayland_client::protocol::__interfaces::*;
    wayland_scanner::generate_interfaces!("../../protocols/plasma-window-management.xml");
}
use self::__interfaces::*;

wayland_scanner::generate_client_code!("../../protocols/plasma-window-management.xml");
