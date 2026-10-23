//! The dock: pinned apps first, then running ones, with a dot under each running app.

use std::rc::Rc;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Alignment, Background, Border, Color, Length, mouse};
use iced_widget::{Space, column, container, image, mouse_area, row, svg, text};

use crate::app::{Element, Message};
use crate::apps::{App, AppIndex};
use crate::niri::{self, WindowInfo};

const ICON_SIZE: f32 = 48.0;
const DOT_SIZE: f32 = 4.0;
const DOT_GAP: f32 = 3.0;
const PADDING: f32 = 6.0;
const MARGIN: f32 = 6.0;
const HEIGHT: u32 = (ICON_SIZE + DOT_GAP + DOT_SIZE + 2.0 * PADDING + MARGIN) as u32;

pub struct Item {
    pub app: Rc<App>,
    /// Open windows of this app, most recently focused first.
    windows: Vec<WindowInfo>,
}

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Top,
        size: Some((0, HEIGHT)),
        exclusive_zone: Some(HEIGHT as i32),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-dock")),
        ..Default::default()
    }
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
    let pill = container(row(items.iter().map(tile)).spacing(4))
        .padding([PADDING, PADDING + 2.0])
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.12))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.22),
                width: 1.0,
                radius: 20.0.into(),
            },
            ..Default::default()
        });

    container(pill)
        .center_x(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(iced_widget::core::Padding::ZERO.bottom(MARGIN))
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
