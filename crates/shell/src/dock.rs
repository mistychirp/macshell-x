//! The dock: pinned apps first, then running ones, with a dot under each running app and the
//! hovered app's name above it.
//!
//! The surface spans the bottom of the output and is taller than the pill, leaving room above
//! it. [`layout`] places everything; the view, the material shapes and the input region are all
//! derived from it, so they always line up.

use std::rc::Rc;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::text::{LineHeight, Paragraph as _, Shaping, Wrapping};
use iced_widget::core::{
    Alignment, Background, Border, Color, Font, Length, Point, Rectangle, Size, alignment, mouse,
};
use iced_widget::{Space, column, container, image, mouse_area, pin, row, stack, svg, text};

use crate::app::{Element, Message};
use crate::apps::{App, AppIndex};
use crate::material::{self, Shape};
use crate::niri::{self, WindowInfo};

const ICON_SIZE: f32 = 48.0;
const DOT_SIZE: f32 = 4.0;
const DOT_GAP: f32 = 3.0;
const PADDING: f32 = 6.0;
const PADDING_X: f32 = PADDING + 2.0;
const SPACING: f32 = 4.0;
const RADIUS: f32 = 20.0;
const MARGIN: f32 = 6.0;
const PILL_HEIGHT: f32 = ICON_SIZE + DOT_GAP + DOT_SIZE + 2.0 * PADDING;
/// Room above the pill for the name label (and later, magnified icons).
const HEADROOM: f32 = 96.0;
const SURFACE_HEIGHT: u32 = (HEADROOM + PILL_HEIGHT + MARGIN) as u32;

const LABEL_TEXT_SIZE: f32 = 13.0;
const LABEL_PADDING_X: f32 = 10.0;
const LABEL_HEIGHT: f32 = 26.0;
/// Gap between the label and the top of the pill.
const LABEL_GAP: f32 = 10.0;

pub struct Item {
    pub app: Rc<App>,
    /// Open windows of this app, most recently focused first.
    windows: Vec<WindowInfo>,
}

/// Where everything goes, in surface-local logical pixels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub pill: Rectangle,
    pub label: Option<Label>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub rect: Rectangle,
    pub text: String,
}

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Top,
        size: Some((0, SURFACE_HEIGHT)),
        // Only the pill reserves space; the room above it overlaps windows, and the input
        // region lets clicks there through.
        exclusive_zone: Some((PILL_HEIGHT + MARGIN) as i32),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-dock")),
        ..Default::default()
    }
}

pub fn layout(surface_width: f32, items: &[Item], hovered: Option<usize>) -> Layout {
    let n = items.len().max(1) as f32;
    let width = 2.0 * PADDING_X + n * ICON_SIZE + (n - 1.0) * SPACING;
    let pill = Rectangle {
        x: ((surface_width - width) / 2.0).round(),
        y: HEADROOM,
        width,
        height: PILL_HEIGHT,
    };

    let label = hovered.and_then(|index| {
        let item = items.get(index)?;
        let width = (measure_label(&item.app.name) + 2.0 * LABEL_PADDING_X).ceil();
        let icon_center =
            pill.x + PADDING_X + index as f32 * (ICON_SIZE + SPACING) + ICON_SIZE / 2.0;
        let x = (icon_center - width / 2.0)
            .round()
            .clamp(0.0, (surface_width - width).max(0.0));

        Some(Label {
            rect: Rectangle {
                x,
                y: pill.y - LABEL_GAP - LABEL_HEIGHT,
                width,
                height: LABEL_HEIGHT,
            },
            text: item.app.name.clone(),
        })
    });

    Layout { pill, label }
}

/// The item under `point`, given relative to the pill.
pub fn item_at(point: Point, item_count: usize) -> Option<usize> {
    // Each icon owns half of the spacing on either side, so there are no dead gaps.
    let slot = ICON_SIZE + SPACING;
    let x = point.x - PADDING_X + SPACING / 2.0;
    if x < 0.0 {
        return None;
    }
    let index = (x / slot) as usize;
    (index < item_count).then_some(index)
}

pub fn material_shapes(layout: &Layout, style: material::Style) -> Vec<Shape> {
    let mut shapes = vec![Shape {
        rect: rect_i32(layout.pill),
        radius: RADIUS as i32,
        style,
    }];
    if let Some(label) = &layout.label {
        shapes.push(Shape {
            rect: rect_i32(label.rect),
            radius: (LABEL_HEIGHT / 2.0) as i32,
            // Frosted keeps the name readable over busy backgrounds.
            style: material::Style::Frosted,
        });
    }
    shapes
}

/// The part of the surface that takes input: just the pill.
pub fn input_region(layout: &Layout) -> material::Rect {
    rect_i32(layout.pill)
}

fn rect_i32(rect: Rectangle) -> material::Rect {
    (
        rect.x as i32,
        rect.y as i32,
        rect.width as i32,
        rect.height as i32,
    )
}

fn measure_label(text: &str) -> f32 {
    iced_widget::graphics::text::Paragraph::with_text(iced_widget::core::Text {
        content: text,
        bounds: Size::INFINITE,
        size: LABEL_TEXT_SIZE.into(),
        line_height: LineHeight::default(),
        font: Font::default(),
        align_x: Default::default(),
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
    })
    .min_width()
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

pub fn view<'a>(
    items: &'a [Item],
    layout: &'a Layout,
    material: Option<material::Kind>,
) -> Element<'a> {
    // Glass brings its own rim and highlight; over blur (or nothing) the pill needs a faint
    // fill and border to read as a surface.
    let glass = material == Some(material::Kind::Glass);
    let (fill, border) = if glass { (0.0, 0.0) } else { (0.04, 0.14) };

    let pill = container(row(items.iter().map(tile)).spacing(SPACING))
        .padding([PADDING, PADDING_X])
        .width(layout.pill.width)
        .height(layout.pill.height)
        .style(move |_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, fill))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, border),
                width: 1.0,
                radius: RADIUS.into(),
            },
            ..Default::default()
        });
    let pill = mouse_area(pill)
        .on_move(Message::DockPointerMoved)
        .on_exit(Message::DockPointerLeft);

    let mut layers = vec![pin(pill).x(layout.pill.x).y(layout.pill.y).into()];

    if let Some(label) = &layout.label {
        // Over glass, a light dark tint keeps the frosted bubble legible; without it, the
        // bubble needs a fill of its own.
        let tint = if material.is_some() { 0.25 } else { 0.6 };
        let bubble = container(text(&label.text).size(LABEL_TEXT_SIZE))
            .center_x(label.rect.width)
            .center_y(label.rect.height)
            .style(move |_| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, tint))),
                border: Border {
                    radius: (LABEL_HEIGHT / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        layers.push(pin(bubble).x(label.rect.x).y(label.rect.y).into());
    }

    stack(layers)
        .width(Length::Fill)
        .height(Length::Fill)
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
