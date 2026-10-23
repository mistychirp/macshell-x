//! Compositor-side background blur through `ext-background-effect-v1`.
//!
//! The shell shares its Wayland connection with iced_layershell, so the `wl_surface` behind an
//! iced window (reached with `window::run`) works with the protocol objects bound here.

use raw_window_handle::RawWindowHandle;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::{
    self, ExtBackgroundEffectManagerV1,
};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

/// A rectangle in surface-local logical coordinates: `(x, y, width, height)`.
pub type Rect = (i32, i32, i32, i32);

pub struct Blur {
    connection: Connection,
    compositor: WlCompositor,
    manager: ExtBackgroundEffectManagerV1,
    queue: QueueHandle<Globals>,
    // Never dispatched after setup, but owns the queue our objects live on.
    _event_queue: EventQueue<Globals>,
}

/// A surface with a background effect attached.
pub struct BlurredSurface {
    surface: WlSurface,
    effect: ExtBackgroundEffectSurfaceV1,
}

#[derive(Default)]
struct Globals {
    blur_supported: bool,
}

impl Blur {
    /// Binds the protocol on `connection`. Returns `None` when the compositor can't blur.
    pub fn new(connection: &Connection) -> Option<Self> {
        let (globals, mut event_queue) = registry_queue_init::<Globals>(connection)
            .inspect_err(|err| log::warn!("can't list Wayland globals: {err}"))
            .ok()?;
        let queue = event_queue.handle();

        let manager: ExtBackgroundEffectManagerV1 = globals
            .bind(&queue, 1..=1, ())
            .inspect_err(|err| log::info!("no background effect support: {err}"))
            .ok()?;
        let compositor: WlCompositor = globals.bind(&queue, 1..=6, ()).ok()?;

        // Wait for the manager's capabilities event.
        let mut state = Globals::default();
        event_queue.roundtrip(&mut state).ok()?;
        if !state.blur_supported {
            log::info!("compositor has background effects, but not blur");
            return None;
        }

        Some(Self {
            connection: connection.clone(),
            compositor,
            manager,
            queue,
            _event_queue: event_queue,
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn attach(&self, surface: WlSurface) -> BlurredSurface {
        let effect = self
            .manager
            .get_background_effect(&surface, &self.queue, ());
        BlurredSurface { surface, effect }
    }

    /// Replaces the blurred region and commits it right away.
    pub fn set_region(&self, target: &BlurredSurface, rects: &[Rect]) {
        let region = self.compositor.create_region(&self.queue, ());
        for &(x, y, width, height) in rects {
            region.add(x, y, width, height);
        }
        target.effect.set_blur_region(Some(&region));
        region.destroy();

        // The region is double-buffered. Committing without a new buffer only applies it, and
        // between iced's frames there's no other pending state we could flush by accident.
        target.surface.commit();
        if let Err(err) = self.connection.flush() {
            log::error!("can't flush Wayland connection: {err}");
        }
    }
}

/// Wraps the `wl_surface` of an iced window. Call it from inside `iced_runtime::window::run`.
pub fn surface_of(
    connection: &Connection,
    window: &dyn iced_runtime::window::Window,
) -> Option<WlSurface> {
    let RawWindowHandle::Wayland(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: the pointer is a live `wl_surface` proxy from iced_layershell, on `connection`.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }
        .ok()?;
    WlSurface::from_id(connection, id).ok()
}

/// Approximates a rounded rectangle with rectangles, since a `wl_region` can't hold curves.
pub fn rounded_rect(width: i32, height: i32, radius: i32) -> Vec<Rect> {
    let radius = radius.min(width / 2).min(height / 2);
    let r = radius as f32;

    // How far each corner row is inset from the side, merged into runs of equal inset.
    let mut runs: Vec<(i32, i32, i32)> = Vec::new(); // (first row, rows, inset)
    for row in 0..radius {
        let dy = r - row as f32 - 0.5;
        let inset = (r - (r * r - dy * dy).sqrt()).round() as i32;
        match runs.last_mut() {
            Some((_, rows, last_inset)) if *last_inset == inset => *rows += 1,
            _ => runs.push((row, 1, inset)),
        }
    }

    let mut rects = Vec::with_capacity(runs.len() * 2 + 1);
    for &(row, rows, inset) in &runs {
        rects.push((inset, row, width - 2 * inset, rows));
        rects.push((inset, height - row - rows, width - 2 * inset, rows));
    }
    rects.push((0, radius, width, height - 2 * radius));
    rects
}

impl Dispatch<WlRegistry, GlobalListContents> for Globals {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Globals {
    fn event(
        state: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            state.blur_supported = flags.into_result().is_ok_and(|flags| {
                flags.contains(ext_background_effect_manager_v1::Capability::Blur)
            });
        }
    }
}

delegate_noop!(Globals: WlCompositor);
delegate_noop!(Globals: WlRegion);
delegate_noop!(Globals: ExtBackgroundEffectSurfaceV1);
