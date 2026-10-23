//! The dock: pinned apps first, then running ones, with a dot under each running app, the
//! hovered app's name above it, and icons that grow toward the pointer.
//!
//! The surface spans the bottom of the output and is taller than the pill, leaving room above
//! it. [`layout`] places everything; the view, the material shapes and the input region are all
//! derived from it, so they always line up.

use std::f32::consts::PI;
use std::rc::Rc;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Background, Border, Color, Length, Point, Rectangle, mouse};
use iced_widget::{Space, container, image, mouse_area, pin, stack, text};

use crate::app::{Element, Message};
use crate::apps::{App, AppIndex};
use crate::material::{self, Shape};
use crate::menu;
use crate::niri::{self, WindowInfo};

pub const ICON_SIZE: f32 = 48.0;
const DOT_SIZE: f32 = 4.0;
const DOT_GAP: f32 = 3.0;
const PADDING: f32 = 6.0;
const PADDING_X: f32 = PADDING + 2.0;
const SPACING: f32 = 4.0;
const RADIUS: f32 = 20.0;
const MARGIN: f32 = 6.0;
const PILL_HEIGHT: f32 = ICON_SIZE + DOT_GAP + DOT_SIZE + 2.0 * PADDING;

/// How many icon slots to either side of the pointer magnification reaches.
const INFLUENCE: f32 = 2.5;

const LABEL_TEXT_SIZE: f32 = 13.0;
const LABEL_PADDING_X: f32 = 10.0;
const LABEL_HEIGHT: f32 = 26.0;
/// Gap between the label and the top of the icon under it.
const LABEL_GAP: f32 = 10.0;
/// Gap between a menu and the top of the icon it belongs to.
const MENU_GAP: f32 = 8.0;

/// One launch bounce: up and back down, in seconds.
const LAUNCH_HOP: f32 = 0.7;
const LAUNCH_HEIGHT: f32 = ICON_SIZE * 0.5;
/// One attention bounce, then a pause as long as the hop.
const ATTENTION_HOP: f32 = 0.6;
const ATTENTION_HEIGHT: f32 = ICON_SIZE * 0.6;

pub struct Item {
    pub app: Rc<App>,
    /// Open windows of this app, most recently focused first.
    windows: Vec<WindowInfo>,
}

impl Item {
    pub fn is_running(&self) -> bool {
        !self.windows.is_empty()
    }

    pub fn wants_attention(&self) -> bool {
        self.windows.iter().any(|window| window.is_urgent)
    }
}

/// A parabolic hop from 0 up to 1 and back down over `period` seconds, repeating.
fn hop(t: f32, period: f32) -> f32 {
    let p = (t / period).fract();
    4.0 * p * (1.0 - p)
}

/// How far up a launching icon is, `t` seconds after the launch.
pub fn launch_bounce(t: f32) -> f32 {
    hop(t, LAUNCH_HOP) * LAUNCH_HEIGHT
}

/// How far up an icon asking for attention is: a hop, then a pause, repeating.
pub fn attention_bounce(t: f32) -> f32 {
    let t = t % (2.0 * ATTENTION_HOP);
    if t < ATTENTION_HOP {
        hop(t, ATTENTION_HOP) * ATTENTION_HEIGHT
    } else {
        0.0
    }
}

/// Where the pointer is, as far as the layout cares.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pointer {
    /// Pointer x in unmagnified pill coordinates, once it has been over the dock.
    pub base_x: Option<f32>,
    /// How far magnification has eased in, from 0 to 1.
    pub amount: f32,
    pub hovered: Option<usize>,
}

/// Where everything goes, in surface-local logical pixels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub pill: Rectangle,
    /// One per item, bottom-aligned; magnified icons stand up out of the pill.
    pub icons: Vec<Rectangle>,
    pub label: Option<Label>,
    /// How far each icon is lifted by a bounce. Only drawing uses it: the input region, the
    /// material and the label stay put, so bouncing doesn't touch the surface's state.
    pub bounce: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub rect: Rectangle,
    pub text: String,
}

/// Room above the pill for the largest icon and the label over it.
fn headroom(max_icon: f32) -> f32 {
    (max_icon - ICON_SIZE).max(0.0) + LABEL_GAP + LABEL_HEIGHT + 4.0
}

/// `max_icon` is the size of a fully magnified icon (the plain icon size without magnification).
pub fn settings(max_icon: f32) -> NewLayerShellSettings {
    let height = headroom(max_icon) + PILL_HEIGHT + MARGIN;
    NewLayerShellSettings {
        anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Top,
        size: Some((0, height.ceil() as u32)),
        // Only the pill reserves space; the room above it overlaps windows, and the input
        // region lets clicks there through.
        exclusive_zone: Some((PILL_HEIGHT + MARGIN) as i32),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-dock")),
        ..Default::default()
    }
}

/// Unmagnified center of icon `index`, in pill coordinates.
fn base_center(index: usize) -> f32 {
    PADDING_X + index as f32 * (ICON_SIZE + SPACING) + ICON_SIZE / 2.0
}

fn icon_size(index: usize, pointer: &Pointer, max_icon: f32) -> f32 {
    let Some(base_x) = pointer.base_x else {
        return ICON_SIZE;
    };
    let distance = (base_x - base_center(index)).abs() / (INFLUENCE * (ICON_SIZE + SPACING));
    if distance >= 1.0 {
        return ICON_SIZE;
    }
    let falloff = (1.0 + (PI * distance).cos()) / 2.0;
    ICON_SIZE + (max_icon - ICON_SIZE) * falloff * pointer.amount
}

/// `bounce` holds each item's current bounce height (missing items don't bounce).
pub fn layout(
    surface_width: f32,
    items: &[Item],
    pointer: &Pointer,
    max_icon: f32,
    bounce: Vec<f32>,
) -> Layout {
    let sizes: Vec<f32> = (0..items.len())
        .map(|index| icon_size(index, pointer, max_icon))
        .collect();
    let icons_width = if sizes.is_empty() {
        ICON_SIZE
    } else {
        sizes.iter().sum::<f32>() + (sizes.len() - 1) as f32 * SPACING
    };

    let width = (2.0 * PADDING_X + icons_width).round();
    let pill = Rectangle {
        x: ((surface_width - width) / 2.0).round(),
        y: headroom(max_icon).round(),
        width,
        height: PILL_HEIGHT,
    };

    let icon_bottom = pill.y + PADDING + ICON_SIZE;
    let mut x = pill.x + PADDING_X;
    let icons: Vec<Rectangle> = sizes
        .iter()
        .map(|&size| {
            let rect = Rectangle {
                x,
                y: icon_bottom - size,
                width: size,
                height: size,
            };
            x += size + SPACING;
            rect
        })
        .collect();

    let label = pointer.hovered.and_then(|index| {
        let item = items.get(index)?;
        let icon = icons.get(index)?;
        let width =
            (menu::text_width(&item.app.name, LABEL_TEXT_SIZE) + 2.0 * LABEL_PADDING_X).ceil();
        let x = (icon.center_x() - width / 2.0)
            .round()
            .clamp(0.0, (surface_width - width).max(0.0));

        Some(Label {
            rect: Rectangle {
                x,
                y: (icon.y - LABEL_GAP - LABEL_HEIGHT).round(),
                width,
                height: LABEL_HEIGHT,
            },
            text: item.app.name.clone(),
        })
    });

    Layout {
        pill,
        icons,
        label,
        bounce,
    }
}

/// Horizontal extent of icon `index`'s slot: the icon plus half the spacing on either side, so
/// there are no dead gaps between icons.
fn slot(icon: &Rectangle) -> (f32, f32) {
    (icon.x - SPACING / 2.0, icon.x + icon.width + SPACING / 2.0)
}

/// Converts a surface x to unmagnified pill coordinates. Magnification is computed from this,
/// so a pointer that doesn't move keeps the dock still even though the icons around it grew.
pub fn base_x(layout: &Layout, surface_x: f32) -> f32 {
    let slot_width = ICON_SIZE + SPACING;
    let (Some(first), Some(last)) = (layout.icons.first(), layout.icons.last()) else {
        return surface_x - layout.pill.x;
    };

    let (first_start, _) = slot(first);
    if surface_x < first_start {
        return base_center(0) - slot_width / 2.0 - (first_start - surface_x);
    }
    for (index, icon) in layout.icons.iter().enumerate() {
        let (start, end) = slot(icon);
        if surface_x < end {
            let t = (surface_x - start) / (end - start);
            return base_center(index) - slot_width / 2.0 + t * slot_width;
        }
    }
    let (_, last_end) = slot(last);
    base_center(layout.icons.len() - 1) + slot_width / 2.0 + (surface_x - last_end)
}

/// The item whose slot contains `point` (in surface coordinates).
pub fn item_at(layout: &Layout, point: Point) -> Option<usize> {
    let bottom = layout.pill.y + layout.pill.height;
    layout.icons.iter().position(|icon| {
        let (start, end) = slot(icon);
        (start..end).contains(&point.x) && (icon.y..bottom).contains(&point.y)
    })
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

/// The parts of the surface that take input: the pill, plus magnified icons above it.
pub fn input_region(layout: &Layout) -> Vec<material::Rect> {
    let mut rects = vec![rect_i32(layout.pill)];
    rects.extend(
        layout
            .icons
            .iter()
            .filter(|icon| icon.y < layout.pill.y)
            .map(|&icon| {
                let (start, end) = slot(&icon);
                let top = icon.y.floor();
                rect_i32(Rectangle {
                    x: start,
                    y: top,
                    width: end - start,
                    height: layout.pill.y - top,
                })
            }),
    );
    rects
}

fn rect_i32(rect: Rectangle) -> material::Rect {
    (
        rect.x as i32,
        rect.y as i32,
        rect.width.ceil() as i32,
        rect.height.ceil() as i32,
    )
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

/// What a dock menu item does.
#[derive(Debug, Clone)]
pub enum MenuAction {
    FocusWindow(u64),
    /// Launches the app with this id, bouncing its icon until it opens a window.
    Open(String),
    /// Runs an extra command, like a desktop action, without bouncing.
    Launch(Vec<String>),
    /// Pins or unpins the app with this id.
    TogglePin(String),
    /// Closes these windows.
    Quit(Vec<u64>),
}

/// The right-click menu for an item: its windows, its desktop actions, then dock and app
/// controls.
pub fn menu_entries(item: &Item, pinned: bool) -> Vec<menu::Entry> {
    let mut entries = Vec::new();

    for window in &item.windows {
        let title = window.title.as_deref().unwrap_or(&item.app.name);
        entries.push(
            menu::Entry::item(title, Message::DockMenu(MenuAction::FocusWindow(window.id)))
                .checked(window.is_focused),
        );
    }
    if !entries.is_empty() {
        entries.push(menu::Entry::Separator);
    }

    if !item.app.actions.is_empty() {
        for action in &item.app.actions {
            entries.push(menu::Entry::item(
                &action.name,
                Message::DockMenu(MenuAction::Launch(action.command.clone())),
            ));
        }
        entries.push(menu::Entry::Separator);
    }

    entries.push(
        menu::Entry::item(
            "Keep in Dock",
            Message::DockMenu(MenuAction::TogglePin(item.app.id.clone())),
        )
        .checked(pinned),
    );
    entries.push(menu::Entry::Separator);

    if item.windows.is_empty() {
        if item.app.command.is_some() {
            entries.push(menu::Entry::item(
                "Open",
                Message::DockMenu(MenuAction::Open(item.app.id.clone())),
            ));
        }
    } else {
        let ids = item.windows.iter().map(|window| window.id).collect();
        entries.push(menu::Entry::item(
            "Quit",
            Message::DockMenu(MenuAction::Quit(ids)),
        ));
    }

    // Drop a trailing separator, e.g. for an app with no way to launch it.
    if matches!(entries.last(), Some(menu::Entry::Separator)) {
        entries.pop();
    }
    entries
}

/// Where a menu for icon `index` attaches: just above the icon, centered on it.
pub fn menu_anchor(layout: &Layout, index: usize) -> Option<(i32, i32, i32, i32)> {
    let icon = layout.icons.get(index)?;
    Some((
        icon.x as i32,
        (icon.y - MENU_GAP) as i32,
        icon.width.ceil() as i32,
        1,
    ))
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

    let pill = container(Space::new())
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

    let mut layers: Vec<Element<'a>> = vec![pin(pill).x(layout.pill.x).y(layout.pill.y).into()];

    let dot_y = layout.pill.y + PADDING + ICON_SIZE + DOT_GAP;
    for (index, (item, rect)) in items.iter().zip(&layout.icons).enumerate() {
        let lift = layout.bounce.get(index).copied().unwrap_or(0.0);
        let icon = mouse_area(icon(&item.app, rect.width))
            .on_press(Message::DockClicked(item.app.id.clone()))
            .on_right_press(Message::DockMenuRequested(item.app.id.clone()))
            .interaction(mouse::Interaction::Pointer);
        layers.push(pin(icon).x(rect.x).y(rect.y - lift).into());

        if !item.windows.is_empty() {
            layers.push(
                pin(dot())
                    .x(rect.center_x() - DOT_SIZE / 2.0)
                    .y(dot_y)
                    .into(),
            );
        }
    }

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

    // The input region limits where these fire to the pill and the icons above it.
    mouse_area(stack(layers).width(Length::Fill).height(Length::Fill))
        .on_move(Message::DockPointerMoved)
        .on_exit(Message::DockPointerLeft)
        .into()
}

fn dot<'a>() -> Element<'a> {
    container(Space::new())
        .width(DOT_SIZE)
        .height(DOT_SIZE)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.8))),
            border: Border {
                radius: (DOT_SIZE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn icon(app: &App, size: f32) -> Element<'_> {
    match &app.icon {
        Some(handle) => image(handle.clone()).width(size).height(size).into(),
        // No icon found: a rounded tile with the app's initial.
        None => {
            container(text(app.name.chars().next().unwrap_or('?').to_string()).size(size * 0.45))
                .center(size)
                .style(move |_| container::Style {
                    background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.2))),
                    border: Border {
                        radius: (size / 4.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
        }
    }
}
