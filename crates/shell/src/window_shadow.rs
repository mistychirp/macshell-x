//! The shadow around windows the shell opens itself, which have no decoration to bring one. KWin
//! draws it from nine-patch pieces, which keeps it right as the window moves and as what's behind it
//! repaints, which an effect's own painting wouldn't.

use std::io::Write;
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicU32, Ordering};

use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::{Dispatch, QueueHandle};

use crate::shadow_protocol::org_kde_kwin_shadow::OrgKdeKwinShadow;

// How far the shadow reaches out of the window, in logical pixels. Measured from Tahoe's alert, it
// falls off over about 30 pixels at the sides and 55 below.
const LEFT: usize = 56;
const TOP: usize = 40;
const RIGHT: usize = 56;
const BOTTOM: usize = 72;
/// How far the corner pieces reach into the window, past which the shadow's edges are straight.
const INNER: usize = 64;
const CANVAS_WIDTH: usize = LEFT + 2 * INNER + 2 + RIGHT;
const CANVAS_HEIGHT: usize = TOP + 2 * INNER + 2 + BOTTOM;
const OFFSET_Y: f32 = 12.0;
/// Where the shadow stops abruptly otherwise.
const FADE: f32 = 8.0;

/// The nine-patch pieces of one shadow.
pub struct Tiles {
    buffers: [WlBuffer; 8],
}

impl Tiles {
    /// Draws them. An active window casts a wider, darker shadow, measured from Tahoe's alert.
    pub fn new<D>(shm: &WlShm, queue: &QueueHandle<D>, radius: f32, active: bool) -> Option<Self>
    where
        D: Dispatch<wayland_client::protocol::wl_shm_pool::WlShmPool, ()>
            + Dispatch<WlBuffer, ()>
            + 'static,
    {
        let (sigma, strength) = if active { (20.0, 0.62) } else { (12.0, 0.3) };
        let canvas = canvas(radius, sigma, strength);

        // Left, top left, top, top right, right, bottom right, bottom, bottom left.
        let middle_x = LEFT + INNER;
        let middle_y = TOP + INNER;
        let right_x = CANVAS_WIDTH - RIGHT - INNER;
        let bottom_y = CANVAS_HEIGHT - BOTTOM - INNER;
        let pieces = [
            (0, middle_y, LEFT, 1),
            (0, 0, LEFT + INNER, TOP + INNER),
            (middle_x, 0, 1, TOP),
            (right_x, 0, RIGHT + INNER, TOP + INNER),
            (CANVAS_WIDTH - RIGHT, middle_y, RIGHT, 1),
            (right_x, bottom_y, RIGHT + INNER, BOTTOM + INNER),
            (middle_x, CANVAS_HEIGHT - BOTTOM, 1, BOTTOM),
            (0, bottom_y, LEFT + INNER, BOTTOM + INNER),
        ];

        let mut bytes = Vec::new();
        let mut placed = Vec::new();
        for (x, y, width, height) in pieces {
            placed.push((bytes.len(), width, height));
            for row in y..y + height {
                for column in x..x + width {
                    // Premultiplied black, as little-endian ARGB.
                    let alpha = (canvas[row * CANVAS_WIDTH + column] * 255.0).round() as u8;
                    bytes.extend_from_slice(&[0, 0, 0, alpha]);
                }
            }
        }

        let file = anonymous_file(&bytes)
            .inspect_err(|err| log::warn!("can't store a window shadow: {err}"))
            .ok()?;
        let pool = shm.create_pool(file.as_fd(), bytes.len() as i32, queue, ());
        let buffers: Vec<WlBuffer> = placed
            .into_iter()
            .map(|(offset, width, height)| {
                pool.create_buffer(
                    offset as i32,
                    width as i32,
                    height as i32,
                    width as i32 * 4,
                    Format::Argb8888,
                    queue,
                    (),
                )
            })
            .collect();
        pool.destroy();
        Some(Self {
            buffers: buffers.try_into().ok()?,
        })
    }

    /// Hands the pieces to a shadow, which applies with the surface's next commit.
    pub fn attach(&self, shadow: &OrgKdeKwinShadow) {
        let [
            left,
            top_left,
            top,
            top_right,
            right,
            bottom_right,
            bottom,
            bottom_left,
        ] = &self.buffers;
        shadow.attach_left(left);
        shadow.attach_top_left(top_left);
        shadow.attach_top(top);
        shadow.attach_top_right(top_right);
        shadow.attach_right(right);
        shadow.attach_bottom_right(bottom_right);
        shadow.attach_bottom(bottom);
        shadow.attach_bottom_left(bottom_left);
        shadow.set_left_offset(LEFT as f64);
        shadow.set_top_offset(TOP as f64);
        shadow.set_right_offset(RIGHT as f64);
        shadow.set_bottom_offset(BOTTOM as f64);
        shadow.commit();
    }
}

/// How much shadow there is at each pixel of a window just big enough for the corners, with nothing
/// where the window itself is: glass has to show what's behind it, not this.
fn canvas(radius: f32, sigma: f32, strength: f32) -> Vec<f32> {
    let window = (
        LEFT as f32,
        TOP as f32,
        (CANVAS_WIDTH - LEFT - RIGHT) as f32,
        (CANVAS_HEIGHT - TOP - BOTTOM) as f32,
    );
    let distance = |x: f32, y: f32| {
        let half = (window.2 / 2.0, window.3 / 2.0);
        let qx = (x - window.0 - half.0).abs() - half.0 + radius;
        let qy = (y - window.1 - half.1).abs() - half.1 + radius;
        qx.max(qy).min(0.0) + qx.max(0.0).hypot(qy.max(0.0)) - radius
    };

    let mut pixels = vec![0.0f32; CANVAS_WIDTH * CANVAS_HEIGHT];
    for y in 0..CANVAS_HEIGHT {
        for x in 0..CANVAS_WIDTH {
            let inside = 0.5 - distance(x as f32 + 0.5, y as f32 + 0.5 - OFFSET_Y);
            pixels[y * CANVAS_WIDTH + x] = inside.clamp(0.0, 1.0);
        }
    }
    // Three box blurs are as good as a Gaussian.
    let box_radius = ((sigma * sigma * 4.0 + 1.0f32).sqrt() as usize) / 2;
    for _ in 0..3 {
        pixels = box_blur(&pixels, CANVAS_WIDTH, CANVAS_HEIGHT, box_radius, true);
        pixels = box_blur(&pixels, CANVAS_WIDTH, CANVAS_HEIGHT, box_radius, false);
    }

    for y in 0..CANVAS_HEIGHT {
        for x in 0..CANVAS_WIDTH {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let outside = (0.5 + distance(px, py)).clamp(0.0, 1.0);
            let border = px
                .min(py)
                .min(CANVAS_WIDTH as f32 - px)
                .min(CANVAS_HEIGHT as f32 - py);
            pixels[y * CANVAS_WIDTH + x] *= strength * outside * (border / FADE).min(1.0);
        }
    }
    pixels
}

fn box_blur(
    pixels: &[f32],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
) -> Vec<f32> {
    let (lines, length) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    let at = |line: usize, along: usize| {
        if horizontal {
            line * width + along
        } else {
            along * width + line
        }
    };
    let window = (2 * radius + 1) as f32;
    let mut blurred = vec![0.0; pixels.len()];
    for line in 0..lines {
        let mut sum = 0.0;
        // Past the canvas there's no shadow to speak of.
        for along in 0..=radius.min(length - 1) {
            sum += pixels[at(line, along)];
        }
        for along in 0..length {
            blurred[at(line, along)] = sum / window;
            if along + radius + 1 < length {
                sum += pixels[at(line, along + radius + 1)];
            }
            if along >= radius {
                sum -= pixels[at(line, along - radius)];
            }
        }
    }
    blurred
}

/// A file with these contents that's gone once the compositor and we let go of it.
pub fn anonymous_file(bytes: &[u8]) -> std::io::Result<std::fs::File> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = directory.join(format!(
        "makshell-shadow-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    file.write_all(bytes)?;
    Ok(file)
}
