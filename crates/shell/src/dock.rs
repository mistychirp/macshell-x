//! The dock: pinned apps first, then running ones, with a dot under each running app.

use std::rc::Rc;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Alignment, Background, Border, Color, Length, mouse};
use iced_widget::{Space, column, container, image, mouse_area, row, svg, text};

use crate::app::{Element, Message};
use crate::apps::{App, AppIndex};
use crate::blur::{self, Rect};
use crate::niri::{self, WindowInfo};

const ICON_SIZE: f32 = 48.0;
const DOT_SIZE: f32 = 4.0;
const DOT_GAP: f32 = 3.0;
const PADDING: f32 = 6.0;
const PADDING_X: f32 = PADDING + 2.0;
const SPACING: f32 = 4.0;
const RADIUS: f32 = 20.0;
const MARGIN: i32 = 6;
const HEIGHT: u32 = (ICON_SIZE + DOT_GAP + DOT_SIZE + 2.0 * PADDING) as u32;

pub struct Item {
    pub app: Rc<App>,
    /// Open windows of this app, most recently focused first.
    windows: Vec<WindowInfo>,
}

/// The surface is exactly the pill, so clicks beside it reach the windows behind.
pub fn settings(item_count: usize) -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Bottom,
        layer: Layer::Top,
        size: Some(size(item_count)),
        margin: Some((0, 0, MARGIN, 0)),
        exclusive_zone: Some(HEIGHT as i32),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-dock")),
        ..Default::default()
    }
}

pub fn size(item_count: usize) -> (u32, u32) {
    let n = item_count.max(1) as f32;
    let width = 2.0 * PADDING_X + n * ICON_SIZE + (n - 1.0) * SPACING;
    (width as u32, HEIGHT)
}

pub fn blur_region((width, height): (u32, u32)) -> Vec<Rect> {
    blur::rounded_rect(width as i32, height as i32, RADIUS as i32)
}

pub fn items(pinned: &[String], windows: &[WindowInfo], apps: &mut AppIndex) -> Vec<Item> {
    let mut items: Vec<Item> = pinned
        .iter()
        .map(|id| Item {
            app: apps.resolve(id),
            windows: Vec::new(),
        })
        .collect();

    for window in windows {
        let Some(app_id) = &window.app_id else {
            continue;
        };
        let app = apps.resolve(app_id);
        match items.iter_mut().find(|item| item.app.id == app.id) {
            Some(item) => item.windows.push(window.clone()),
            None => items.push(Item {
                app,
                windows: vec![window.clone()],
            }),
        }
    }

    for item in &mut items {
        item.windows
            .sort_by_key(|w| std::cmp::Reverse(w.focused_at));
    }
    items
}

/// Focuses the app's most recent window, or launches it if nothing is open.
pub fn activate(item: &Item) {
    if let Some(window) = item.windows.first() {
        niri::focus_window(window.id);
    } else if let Some(command) = &item.app.command {
        niri::spawn(command.clone());
    } else {
        log::warn!("don't know how to launch {}", item.app.id);
    }
}

pub fn view(items: &[Item]) -> Element<'_> {
    container(row(items.iter().map(tile)).spacing(SPACING))
        .padding([PADDING, PADDING_X])
        .width(Length::Fill)
        .height(Length::Fill)
        // Kept faint: the compositor provides the material (blur or glass) underneath, and
        // glass draws its own rim highlight.
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.04))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.14),
                width: 1.0,
                radius: RADIUS.into(),
            },
            ..Default::default()
        })
        .into()
}

fn tile(item: &Item) -> Element<'_> {
    let running = !item.windows.is_empty();
    let dot = container(Space::new())
        .width(DOT_SIZE)
        .height(DOT_SIZE)
        .style(move |_| container::Style {
            background: running.then_some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.8))),
            border: Border {
                radius: (DOT_SIZE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });

    mouse_area(
        column![icon(&item.app), dot]
            .spacing(DOT_GAP)
            .align_x(Alignment::Center),
    )
    .on_press(Message::DockClicked(item.app.id.clone()))
    .interaction(mouse::Interaction::Pointer)
    .into()
}

fn icon(app: &App) -> Element<'_> {
    match &app.icon {
        Some(path) if path.extension().is_some_and(|ext| ext == "svg") => {
            svg(svg::Handle::from_path(path))
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .into()
        }
        Some(path) => image(image::Handle::from_path(path))
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        // No icon found: a rounded tile with the app's initial.
        None => container(text(app.name.chars().next().unwrap_or('?').to_string()).size(22))
            .center(ICON_SIZE)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.2))),
                border: Border {
                    radius: 12.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    }
}
