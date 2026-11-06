//! The dock: pinned apps first, then running ones, with a dot under each running app, the
//! hovered app's name above it, and icons that grow toward the pointer. After a separator
//! come Downloads, minimized windows and Trash.
//!
//! The surface spans the bottom of the output and is taller than the pill, leaving room above
//! it. [`layout`] places everything; the view, the material shapes and the input region are all
//! derived from it, so they always line up.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Background, Border, Color, ContentFit, Length, Point, Rectangle, Vector};
use iced_widget::{Space, container, image, mouse_area, pin, stack, text};

use std::collections::BTreeMap;

use crate::app::{Element, Message};
use crate::apps;
use crate::apps::{App, AppIndex};
use crate::autostart;
use crate::fonts;
use crate::icons::Glyph;
use crate::kwin::{self, WindowInfo};
use crate::material::{self, Shape};
use crate::menu;
use crate::places::Place;
use crate::running::Process;

pub const ICON_SIZE: f32 = 48.0;
const DOT_SIZE: f32 = 4.0;
const DOT_GAP: f32 = 3.0;
/// Above and below icons, which sit in the middle of the pill, with a running app's dot under them.
const PADDING_Y: f32 = 10.0;
const PADDING_X: f32 = 8.0;
const SPACING: f32 = 4.0;
const RADIUS: f32 = 20.0;
const MARGIN: f32 = 6.0;
const PILL_HEIGHT: f32 = ICON_SIZE + 2.0 * PADDING_Y;
/// What the dock keeps windows out of, at the bottom of the screen.
pub const RESERVED_HEIGHT: f32 = PILL_HEIGHT + MARGIN;

/// Extra room between the apps and the places, where the separator line goes.
const SEPARATOR_GAP: f32 = 12.0;
/// A minimized window's app icon, as a share of its tile.
const TILE_BADGE: f32 = 0.42;
const BADGE_SIZE: f32 = 21.0;
const BADGE_TEXT_SIZE: f32 = 12.0;
const BADGE_PADDING: f32 = 11.0;
const BADGE_OVERHANG: f32 = 5.0;
const SEPARATOR_HEIGHT: f32 = ICON_SIZE * 0.75;

/// How many icon slots to either side of the pointer magnification reaches.
const INFLUENCE: f32 = 2.5;

const LABEL_TEXT_SIZE: f32 = 13.0;
const LABEL_PADDING_X: f32 = 10.0;
const LABEL_HEIGHT: f32 = 26.0;
/// Gap between the label and the top of the icon under it.
const LABEL_GAP: f32 = 10.0;
/// A window's title can be long.
const LABEL_MAX_WIDTH: f32 = 300.0;
/// Gap between a menu and the top of the icon it belongs to.
const MENU_GAP: f32 = 26.0;

/// How far above the pill the pointer has to drag a pinned icon to remove it.
const REMOVE_DISTANCE: f32 = 40.0;
/// Room above the pill for an icon dragged toward removal, with its label.
const DRAG_HEADROOM: f32 = REMOVE_DISTANCE + ICON_SIZE + LABEL_GAP + LABEL_HEIGHT + 4.0;

/// One launch bounce: up and back down, in seconds.
const LAUNCH_HOP: f32 = 0.7;
const LAUNCH_HEIGHT: f32 = ICON_SIZE * 0.5;
/// One attention bounce, then a pause as long as the hop.
const ATTENTION_HOP: f32 = 0.6;
const ATTENTION_HEIGHT: f32 = ICON_SIZE * 0.6;

#[derive(Debug)]
pub struct Item {
    pub app: Rc<App>,
    /// Open windows of this app, most recently focused first.
    windows: Vec<WindowInfo>,
    /// Its processes, which may well outlast its windows.
    processes: Vec<Process>,
}

impl Item {
    pub fn is_running(&self) -> bool {
        self.has_windows() || !self.processes.is_empty()
    }

    pub fn has_windows(&self) -> bool {
        !self.windows.is_empty()
    }

    pub fn processes(&self) -> &[Process] {
        &self.processes
    }

    pub fn wants_attention(&self) -> bool {
        self.windows.iter().any(|window| window.is_urgent)
    }

    pub fn window_ids(&self) -> Vec<u64> {
        self.windows.iter().map(|window| window.id).collect()
    }
}

/// A minimized window, which the dock shows as a picture of itself.
#[derive(Debug)]
pub struct Minimized {
    pub id: u64,
    pub title: String,
    /// The window's picture, once it's been captured.
    pub image: Option<image::Handle>,
    pub app: Rc<App>,
}

/// Which slot a place is in: Downloads comes before the minimized windows, and Trash after.
pub fn place_slot(layout: &Layout, offset: usize) -> usize {
    layout.apps + if offset == 0 { 0 } else { 1 + layout.minimized }
}

/// Where the next window to be minimized will end up, on the surface: the Trash's slot, once the pill
/// has grown to fit one more and stayed in the middle.
pub fn next_minimized_tile(layout: &Layout) -> Option<Rectangle> {
    let trash = *layout.icons.get(place_slot(layout, 1))?;
    Some(Rectangle {
        x: trash.x - (ICON_SIZE + SPACING) / 2.0,
        ..trash
    })
}

/// Which slot a minimized window is in.
pub fn minimized_slot(layout: &Layout, index: usize) -> usize {
    layout.apps + 1 + index
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
    /// One per slot: the apps, then the places. Bottom-aligned; magnified icons stand up out
    /// of the pill.
    pub icons: Vec<Rectangle>,
    /// How many of the slots are apps; the rest are [`Place::ALL`] and the minimized windows.
    pub apps: usize,
    pub minimized: usize,
    /// Horizontal center of the line between the apps and the places.
    pub separator: f32,
    pub label: Option<Label>,
    /// How far each icon is lifted by a bounce. Only drawing uses it: the input region, the
    /// material and the label stay put, so bouncing doesn't touch the surface's state.
    pub bounce: Vec<f32>,
    /// The item being dragged, if any; its icon floats under the pointer.
    pub dragged: Option<usize>,
    /// What releasing the drag would do.
    pub drop: Option<Drop>,
}

/// An icon being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    pub index: usize,
    /// Pointer position, in surface coordinates.
    pub pointer: Point,
    /// Where the icon was grabbed, from its top-left corner at its unmagnified size.
    pub grab: Vector,
    /// Whether dragging it out of the dock removes it, i.e. it's pinned.
    pub removable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    /// Keep the app in the dock at this position.
    Slot(usize),
    /// Unpin the app.
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub rect: Rectangle,
    pub text: String,
}

/// Room above the pill: for the largest icon and the label over it, or for dragging an icon
/// out, whichever needs more.
fn headroom(max_icon: f32) -> f32 {
    ((max_icon - ICON_SIZE).max(0.0) + LABEL_GAP + LABEL_HEIGHT + 4.0).max(DRAG_HEADROOM)
}

/// How long the dock takes to make room for a new tile or close up behind one, from Tahoe's.
const ROOM_TIME: Duration = Duration::from_millis(330);

/// How tall the dock's surface is. `max_icon` is the size of a fully magnified icon.
pub fn surface_height(max_icon: f32) -> f32 {
    (headroom(max_icon) + PILL_HEIGHT + MARGIN).ceil()
}

/// `max_icon` is the size of a fully magnified icon (the plain icon size without magnification).
pub fn settings(max_icon: f32) -> NewLayerShellSettings {
    let height = surface_height(max_icon);
    NewLayerShellSettings {
        anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Top,
        size: Some((0, height as u32)),
        // Only the pill reserves space; the room above it overlaps windows, and the input
        // region lets clicks there through.
        exclusive_zone: Some(RESERVED_HEIGHT as i32),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-dock")),
        ..Default::default()
    }
}

/// Unmagnified center of slot `index`, in pill coordinates, with `apps` app slots before the
/// separator.
fn base_center(index: usize, apps: usize) -> f32 {
    let gap = if index >= apps { SEPARATOR_GAP } else { 0.0 };
    PADDING_X + index as f32 * (ICON_SIZE + SPACING) + gap + ICON_SIZE / 2.0
}

fn icon_size(index: usize, apps: usize, pointer: &Pointer, max_icon: f32) -> f32 {
    let Some(base_x) = pointer.base_x else {
        return ICON_SIZE;
    };
    let distance = (base_x - base_center(index, apps)).abs() / (INFLUENCE * (ICON_SIZE + SPACING));
    if distance >= 1.0 {
        return ICON_SIZE;
    }
    let falloff = (1.0 + (PI * distance).cos()) / 2.0;
    ICON_SIZE + (max_icon - ICON_SIZE) * falloff * pointer.amount
}

/// How the dock's slots are opening and closing right now, as tiles come and go.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Room {
    /// How much of its room each slot has so far. Missing ones have it all.
    pub grown: HashMap<usize, f32>,
    /// Gaps closing where tiles were: which slot each one's before, and how much of a slot's left.
    pub closing: Vec<(usize, f32)>,
}

/// What each slot is, to tell which tiles came and went.
pub fn slot_keys(items: &[Item], minimized: &[Minimized]) -> Vec<String> {
    let mut keys: Vec<String> = items
        .iter()
        .map(|item| format!("app {}", item.app.id))
        .collect();
    keys.push(String::from("downloads"));
    keys.extend(
        minimized
            .iter()
            .map(|window| format!("window {}", window.id)),
    );
    keys.push(String::from("trash"));
    keys
}

/// Keeps track of slots opening and closing, to make room for new tiles, like Tahoe's.
#[derive(Debug, Default)]
pub struct RoomTracker {
    slots: Vec<String>,
    growing: HashMap<String, Instant>,
    /// Where tiles were: the slot that came next, which is still there, and since when.
    closing: Vec<(String, Instant)>,
}

fn room_eased(started: Instant, now: Instant) -> f32 {
    let t =
        (now.saturating_duration_since(started).as_secs_f32() / ROOM_TIME.as_secs_f32()).min(1.0);
    t * t * (3.0 - 2.0 * t)
}

impl RoomTracker {
    /// Takes in what the slots are now. If not `animate`, they're just there, or gone.
    pub fn update(&mut self, keys: Vec<String>, animate: bool, now: Instant) {
        if keys == self.slots {
            return;
        }
        if animate {
            for key in &keys {
                if !self.slots.contains(key) {
                    self.growing.insert(key.clone(), now);
                }
            }
            for (index, key) in self.slots.iter().enumerate() {
                if !keys.contains(key)
                    && let Some(next) = self.slots[index + 1..]
                        .iter()
                        .find(|next| keys.contains(next))
                {
                    self.closing.push((next.clone(), now));
                }
            }
        } else {
            self.growing.clear();
            self.closing.clear();
        }
        self.slots = keys;
    }

    pub fn is_animating(&self) -> bool {
        !self.growing.is_empty() || !self.closing.is_empty()
    }

    /// The room as it is now, forgetting what's over.
    pub fn room(&mut self, now: Instant) -> Room {
        self.growing
            .retain(|_, started| now.saturating_duration_since(*started) < ROOM_TIME);
        self.closing
            .retain(|(_, started)| now.saturating_duration_since(*started) < ROOM_TIME);
        let slot_of = |key: &str| self.slots.iter().position(|slot| slot == key);
        Room {
            grown: self
                .growing
                .iter()
                .filter_map(|(key, started)| Some((slot_of(key)?, room_eased(*started, now))))
                .collect(),
            closing: self
                .closing
                .iter()
                .filter_map(|(key, started)| Some((slot_of(key)?, 1.0 - room_eased(*started, now))))
                .collect(),
        }
    }
}

/// `bounce` holds each item's current bounce height (missing items don't bounce).
#[allow(clippy::too_many_arguments)]
pub fn layout(
    surface_width: f32,
    items: &[Item],
    minimized: &[Minimized],
    pointer: &Pointer,
    max_icon: f32,
    bounce: Vec<f32>,
    drag: Option<&Drag>,
    room: &Room,
) -> Layout {
    if let Some(drag) = drag
        && drag.index < items.len()
    {
        return drag_layout(surface_width, items.len(), minimized.len(), drag, max_icon);
    }

    let apps = items.len();
    let grown = |index: usize| room.grown.get(&index).copied().unwrap_or(1.0);
    let sizes: Vec<f32> = (0..apps + Place::ALL.len() + minimized.len())
        .map(|index| icon_size(index, apps, pointer, max_icon) * grown(index))
        .collect();
    let gap_before = |index: usize| -> f32 {
        room.closing
            .iter()
            .filter(|(slot, _)| *slot == index)
            .map(|(_, left)| left * (ICON_SIZE + SPACING))
            .sum()
    };
    let spacing: f32 = (0..sizes.len())
        .map(|index| SPACING * grown(index) + gap_before(index))
        .sum();
    // The last slot, which is always there, has nothing after it.
    let icons_width = sizes.iter().sum::<f32>() + spacing - SPACING + SEPARATOR_GAP;

    let width = (2.0 * PADDING_X + icons_width).round();
    let pill = Rectangle {
        x: ((surface_width - width) / 2.0).round(),
        y: headroom(max_icon).round(),
        width,
        height: PILL_HEIGHT,
    };

    let icon_bottom = pill.y + PADDING_Y + ICON_SIZE;
    let mut x = pill.x + PADDING_X;
    let mut separator = 0.0;
    let icons: Vec<Rectangle> = sizes
        .iter()
        .enumerate()
        .map(|(index, &size)| {
            x += gap_before(index);
            if index == apps {
                separator = x - SPACING / 2.0 + SEPARATOR_GAP / 2.0;
                x += SEPARATOR_GAP;
            }
            let rect = Rectangle {
                x,
                y: icon_bottom - size,
                width: size,
                height: size,
            };
            x += size + SPACING * grown(index);
            rect
        })
        .collect();

    let label = pointer.hovered.and_then(|index| {
        let name = match items.get(index) {
            Some(item) => item.app.name.clone(),
            None => match index - apps {
                0 => Place::ALL.first()?.name().to_owned(),
                offset if offset <= minimized.len() => menu::fit(
                    &minimized[offset - 1].title,
                    LABEL_TEXT_SIZE,
                    fonts::REGULAR,
                    LABEL_MAX_WIDTH,
                ),
                _ => Place::ALL.get(1)?.name().to_owned(),
            },
        };
        let icon = icons.get(index)?;
        let width = (menu::text_width(&name, LABEL_TEXT_SIZE, fonts::REGULAR)
            + 2.0 * LABEL_PADDING_X)
            .ceil();
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
            text: name,
        })
    });

    Layout {
        pill,
        icons,
        apps,
        minimized: minimized.len(),
        separator,
        label,
        bounce,
        dragged: None,
        drop: None,
    }
}

/// While dragging: icons at their plain size, a gap where the dragged one would land, and the
/// dragged icon itself under the pointer.
fn drag_layout(
    surface_width: f32,
    count: usize,
    minimized: usize,
    drag: &Drag,
    max_icon: f32,
) -> Layout {
    let pill_y = headroom(max_icon).round();
    let removing = drag.removable && drag.pointer.y < pill_y - REMOVE_DISTANCE;
    // Dragging out to remove closes the gap; otherwise the dragged icon keeps a slot.
    let app_slots = if removing { count - 1 } else { count };
    let trailing = Place::ALL.len() + minimized;
    let slots = app_slots + trailing;

    let width =
        2.0 * PADDING_X + slots as f32 * ICON_SIZE + (slots - 1) as f32 * SPACING + SEPARATOR_GAP;
    let pill = Rectangle {
        x: ((surface_width - width) / 2.0).round(),
        y: pill_y,
        width,
        height: PILL_HEIGHT,
    };

    let dragged = Rectangle {
        x: drag.pointer.x - drag.grab.x,
        y: (drag.pointer.y - drag.grab.y).max(0.0),
        width: ICON_SIZE,
        height: ICON_SIZE,
    };
    let slot_x = |slot: usize| pill.x + PADDING_X + slot as f32 * (ICON_SIZE + SPACING);
    let target = (!removing).then(|| {
        let slot = (dragged.x - slot_x(0)) / (ICON_SIZE + SPACING);
        slot.round().clamp(0.0, (count - 1) as f32) as usize
    });

    let mut slot = 0;
    let mut icons: Vec<Rectangle> = (0..count)
        .map(|index| {
            if index == drag.index {
                return dragged;
            }
            if Some(slot) == target {
                slot += 1;
            }
            let rect = Rectangle {
                x: slot_x(slot),
                y: pill.y + PADDING_Y,
                width: ICON_SIZE,
                height: ICON_SIZE,
            };
            slot += 1;
            rect
        })
        .collect();

    let separator = slot_x(app_slots) - SPACING / 2.0 + SEPARATOR_GAP / 2.0;
    icons.extend((0..trailing).map(|index| Rectangle {
        x: slot_x(app_slots + index) + SEPARATOR_GAP,
        y: pill.y + PADDING_Y,
        width: ICON_SIZE,
        height: ICON_SIZE,
    }));

    let label = removing.then(|| {
        let text = String::from("Remove");
        let width = (menu::text_width(&text, LABEL_TEXT_SIZE, fonts::REGULAR)
            + 2.0 * LABEL_PADDING_X)
            .ceil();
        Label {
            rect: Rectangle {
                x: (dragged.center_x() - width / 2.0)
                    .round()
                    .clamp(0.0, (surface_width - width).max(0.0)),
                y: (dragged.y - LABEL_GAP - LABEL_HEIGHT).round().max(0.0),
                width,
                height: LABEL_HEIGHT,
            },
            text,
        }
    });

    Layout {
        pill,
        icons,
        apps: count,
        minimized,
        separator,
        label,
        bounce: Vec::new(),
        dragged: Some(drag.index),
        drop: Some(target.map_or(Drop::Remove, Drop::Slot)),
    }
}

/// Where icon `index` of `layout` was grabbed, at the icon's unmagnified size.
pub fn grab_offset(layout: &Layout, index: usize, pointer: Point) -> Option<Vector> {
    let icon = layout.icons.get(index)?;
    let scale = ICON_SIZE / icon.width;
    Some(Vector::new(
        (pointer.x - icon.x) * scale,
        (pointer.y - icon.y) * scale,
    ))
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
        return base_center(0, layout.apps) - slot_width / 2.0 - (first_start - surface_x);
    }
    for (index, icon) in layout.icons.iter().enumerate() {
        let (start, end) = slot(icon);
        if surface_x < end {
            let t = (surface_x - start) / (end - start);
            return base_center(index, layout.apps) - slot_width / 2.0 + t * slot_width;
        }
    }
    let (_, last_end) = slot(last);
    base_center(layout.icons.len() - 1, layout.apps) + slot_width / 2.0 + (surface_x - last_end)
}

/// The item whose slot contains `point` (in surface coordinates).
pub fn item_at(layout: &Layout, point: Point) -> Option<usize> {
    let bottom = layout.pill.y + layout.pill.height;
    layout.icons.iter().position(|icon| {
        let (start, end) = slot(icon);
        (start..end).contains(&point.x) && (icon.y..bottom).contains(&point.y)
    })
}

pub fn material_shapes(layout: &Layout) -> Vec<Shape> {
    let mut shapes = vec![Shape {
        rect: rect_i32(layout.pill),
        radius: RADIUS as i32,
        style: material::Style::Liquid,
    }];
    if let Some(label) = &layout.label {
        shapes.push(Shape {
            rect: rect_i32(label.rect),
            radius: (LABEL_HEIGHT / 2.0) as i32,
            style: material::Style::Bubble,
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
            .enumerate()
            // A dragged icon gets pointer events through the implicit grab anyway.
            .filter(|&(index, icon)| icon.y < layout.pill.y && layout.dragged != Some(index))
            .map(|(_, icon)| icon)
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

pub fn items(
    pinned: &[String],
    windows: &[WindowInfo],
    running: &BTreeMap<String, Vec<Process>>,
    apps: &mut AppIndex,
) -> Vec<Item> {
    let mut items: Vec<Item> = pinned
        .iter()
        .map(|id| Item {
            app: apps.resolve(id),
            windows: Vec::new(),
            processes: Vec::new(),
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
                processes: Vec::new(),
            }),
        }
    }

    for (app_id, processes) in running {
        let app = apps.resolve(app_id);
        match items.iter_mut().find(|item| item.app.id == app.id) {
            Some(item) => item.processes.extend(processes),
            // Services show windows too, like a file dialog or a password prompt, and don't get to
            // stay in the dock once those have closed.
            None if app.command.is_some() && !app.unlisted => items.push(Item {
                app,
                windows: Vec::new(),
                processes: processes.clone(),
            }),
            None => {}
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
    /// Turns opening at login on or off for the app with this id.
    ToggleLogin(String),
    /// Shows these windows, by their uuids, side by side.
    ShowAll(Vec<String>),
    /// Puts these windows away, without them showing up in the dock as minimized.
    Hide(Vec<u64>),
    /// Quits the app with this id: its windows close, and then whatever carries on is told to quit.
    Quit(String),
    /// Opens a place in the file manager.
    OpenPlace(Place),
    /// Asks before emptying the trash.
    ConfirmEmptyTrash,
    EmptyTrash,
}

/// The right-click menu for a place. `trash_count` is how many items are in the trash.
pub fn place_menu_entries(place: Place, trash_count: usize) -> Vec<menu::Entry> {
    let open = menu::Entry::item(
        &format!("Open {}", place.name()),
        Message::DockMenu(MenuAction::OpenPlace(place)),
    );
    match place {
        Place::Downloads => vec![open],
        Place::Trash if trash_count == 0 => vec![open],
        Place::Trash => vec![
            open,
            menu::Entry::Separator,
            menu::Entry::item(
                "Empty Trash",
                Message::DockMenu(MenuAction::ConfirmEmptyTrash),
            ),
        ],
    }
}

/// The right-click menu for an item: its windows, its desktop actions, then dock and app
/// controls.
pub fn menu_entries(item: &Item, pinned: bool) -> Vec<menu::Entry> {
    let mut entries = Vec::new();

    for window in &item.windows {
        let title = window.title.as_deref().unwrap_or(&item.app.name);
        let entry = menu::Entry::item(title, Message::DockMenu(MenuAction::FocusWindow(window.id)))
            .icon(menu::Icon::Glyph(Glyph::Window));
        entries.push(if window.is_minimized {
            entry.marked("◇")
        } else {
            entry.checked(window.is_focused)
        });
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

    let app_id = &item.app.id;
    entries.push(menu::Entry::item("Options", Message::Nothing).submenu(vec![
            menu::Entry::item(
                "Keep in Dock",
                Message::DockMenu(MenuAction::TogglePin(app_id.clone())),
            )
            .checked(pinned),
            menu::Entry::item(
                "Open at Login",
                Message::DockMenu(MenuAction::ToggleLogin(app_id.clone())),
            )
            .checked(autostart::is_enabled(app_id)),
        ]));
    entries.push(menu::Entry::Separator);

    if item.windows.is_empty() {
        if item.app.command.is_some() {
            entries.push(menu::Entry::item(
                "Open",
                Message::DockMenu(MenuAction::Open(app_id.clone())),
            ));
        }
        if item.is_running() {
            entries.push(menu::Entry::item(
                "Quit",
                Message::DockMenu(MenuAction::Quit(app_id.clone())),
            ));
        }
    } else {
        entries.push(menu::Entry::item(
            "Show All Windows",
            Message::DockMenu(MenuAction::ShowAll(
                item.windows
                    .iter()
                    .map(|window| window.uuid.clone())
                    .collect(),
            )),
        ));
        entries.push(menu::Entry::item(
            "Hide",
            Message::DockMenu(MenuAction::Hide(item.window_ids())),
        ));
        entries.push(menu::Entry::item(
            "Quit",
            Message::DockMenu(MenuAction::Quit(app_id.clone())),
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
        kwin::focus_window(window.id);
    } else if let Some(command) = &item.app.command {
        apps::spawn(command.clone());
    } else {
        log::warn!("don't know how to launch {}", item.app.id);
    }
}

/// `places` holds the current icon of each of [`Place::ALL`].
pub fn view<'a>(
    items: &'a [Item],
    places: &'a [Option<image::Handle>],
    minimized: &'a [Minimized],
    badges: &HashMap<String, u32>,
    layout: &'a Layout,
    material: Option<material::Kind>,
    fan_open: bool,
    downloads: &'a [image::Handle],
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

    let dot_y = layout.pill.y + PADDING_Y + ICON_SIZE + DOT_GAP;
    for (index, (item, rect)) in items.iter().zip(&layout.icons).enumerate() {
        if layout.dragged == Some(index) {
            continue;
        }
        let lift = layout.bounce.get(index).copied().unwrap_or(0.0);
        // A press becomes a click or a drag depending on what the pointer does next.
        let icon = mouse_area(icon(&item.app, rect.width))
            .on_press(Message::DockPressed(item.app.id.clone()))
            .on_right_press(Message::DockMenuRequested(item.app.id.clone()));
        layers.push(pin(icon).x(rect.x).y(rect.y - lift).into());

        if let Some(&count) = badges.get(&item.app.id.to_lowercase()) {
            layers.push(count_badge(count, rect, lift));
        }

        if item.is_running() {
            layers.push(
                pin(dot())
                    .x(rect.center_x() - DOT_SIZE / 2.0)
                    .y(dot_y)
                    .into(),
            );
        }
    }

    for (offset, &place) in Place::ALL.iter().enumerate() {
        let Some(rect) = layout.icons.get(place_slot(layout, offset)) else {
            continue;
        };
        let handle = places.get(offset).and_then(Option::as_ref);
        // Places can't be dragged, so they act right away. On press, not release: a popup's
        // pointer grab needs the press to still be held.
        let picture = if fan_open && place == Place::Downloads {
            fan_tile(rect.width)
        } else if place == Place::Downloads && !downloads.is_empty() {
            downloads_pile(downloads, rect.width)
        } else {
            icon_image(handle, place.name(), rect.width)
        };
        let icon = mouse_area(picture)
            .on_press(Message::PlaceClicked(place))
            .on_right_press(Message::PlaceMenuRequested(place));
        layers.push(pin(icon).x(rect.x).y(rect.y).into());
    }

    for (index, window) in minimized.iter().enumerate() {
        let Some(rect) = layout.icons.get(minimized_slot(layout, index)) else {
            continue;
        };
        let tile = mouse_area(minimized_tile(window, rect.width))
            .on_press(Message::DockThumbnailPressed(window.id))
            .on_right_press(Message::DockThumbnailMenu(window.id));
        layers.push(pin(tile).x(rect.x).y(rect.y).into());
    }

    let separator = container(Space::new())
        .width(1)
        .height(SEPARATOR_HEIGHT)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.3))),
            ..Default::default()
        });
    layers.push(
        pin(separator)
            .x(layout.separator - 0.5)
            .y(layout.pill.y + PADDING_Y + (ICON_SIZE - SEPARATOR_HEIGHT) / 2.0)
            .into(),
    );

    // The dragged icon floats above everything else.
    if let Some(index) = layout.dragged
        && let (Some(item), Some(rect)) = (items.get(index), layout.icons.get(index))
    {
        layers.push(pin(icon(&item.app, rect.width)).x(rect.x).y(rect.y).into());
    }

    if let Some(label) = &layout.label {
        let tint = material::panel_tint(material).min(0.6);
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

/// A minimized window's picture, with its app's icon on its corner, like Tahoe's.
fn minimized_tile(window: &Minimized, size: f32) -> Element<'_> {
    let picture: Element<'_> = match &window.image {
        Some(handle) => image(handle.clone())
            .width(size)
            .height(size)
            .content_fit(ContentFit::Contain)
            .into(),
        None => icon(&window.app, size),
    };
    let badge_size = size * TILE_BADGE;
    let badge = pin(icon(&window.app, badge_size))
        .x(size - badge_size)
        .y(size - badge_size);
    container(stack![picture, badge])
        .width(size)
        .height(size)
        .into()
}

/// A red count on an icon's corner, like macOS's.
fn count_badge<'a>(count: u32, icon: &Rectangle, lift: f32) -> Element<'a> {
    let scale = icon.width / ICON_SIZE;
    let size = BADGE_SIZE * scale;
    let label = if count > 99 {
        String::from("99+")
    } else {
        count.to_string()
    };
    let width = (menu::text_width(&label, BADGE_TEXT_SIZE * scale, fonts::BOLD)
        + BADGE_PADDING * scale)
        .max(size);
    let badge = container(
        text(label)
            .size(BADGE_TEXT_SIZE * scale)
            .font(fonts::BOLD)
            .color(Color::WHITE),
    )
    .center_x(width)
    .center_y(size)
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgb(0.95, 0.23, 0.19))),
        border: Border {
            radius: (size / 2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    });
    pin(badge)
        .x(icon.x + icon.width - width + BADGE_OVERHANG * scale)
        .y(icon.y - lift - BADGE_OVERHANG * scale)
        .into()
}

/// The Downloads tile: its newest files in a pile, the newest on top.
fn downloads_pile<'a>(files: &'a [image::Handle], size: f32) -> Element<'a> {
    const TILTS: [f32; 3] = [0.0, 7.0, -7.0];
    let mut layers: Vec<Element<'a>> = vec![Space::new().width(size).height(size).into()];
    for (index, file) in files.iter().enumerate().take(TILTS.len()).rev() {
        let picture = size * 0.86;
        let inset = (size - picture) / 2.0;
        layers.push(
            pin(image(file.clone())
                .width(picture)
                .height(picture)
                .content_fit(iced_widget::core::ContentFit::Contain)
                .border_radius(3.0)
                .rotation(iced_widget::core::Rotation::Floating(
                    iced_widget::core::Radians(TILTS[index].to_radians()),
                )))
            .x(inset)
            .y(inset)
            .into(),
        );
    }
    iced_widget::Stack::with_children(layers)
        .width(size)
        .height(size)
        .into()
}

/// The Downloads tile while its fan is open, which closes it.
fn fan_tile<'a>(size: f32) -> Element<'a> {
    container(crate::icons::icon(
        Glyph::ChevronDown,
        size * 0.45,
        Color::WHITE,
    ))
    .center_x(size)
    .center_y(size)
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.12, 0.1, 0.14, 0.72))),
        border: Border {
            radius: (size * 0.22).into(),
            ..Default::default()
        },
        ..Default::default()
    })
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
    icon_image(app.icon.as_ref(), &app.name, size)
}

fn icon_image<'a>(handle: Option<&image::Handle>, name: &str, size: f32) -> Element<'a> {
    match handle {
        Some(handle) => image(handle.clone()).width(size).height(size).into(),
        // No icon found: a rounded tile with the initial.
        None => container(text(name.chars().next().unwrap_or('?').to_string()).size(size * 0.45))
            .center(size)
            .style(move |_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.2))),
                border: Border {
                    radius: (size / 4.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
    }
}

#[cfg(test)]
mod room_tests {
    use super::*;

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_new_slot_grows_and_a_lost_one_leaves_a_gap_that_closes() {
        let start = Instant::now();
        let mut tracker = RoomTracker::default();
        tracker.update(keys(&["a", "b", "trash"]), false, start);
        assert!(!tracker.is_animating());

        tracker.update(keys(&["a", "new", "b", "trash"]), true, start);
        let room = tracker.room(start);
        assert_eq!(room.grown.get(&1), Some(&0.0));
        assert!(room.closing.is_empty());
        let room = tracker.room(start + ROOM_TIME / 2);
        assert!((room.grown[&1] - 0.5).abs() < 0.01);
        assert!(tracker.room(start + ROOM_TIME).grown.is_empty());
        assert!(!tracker.is_animating());

        tracker.update(keys(&["new", "b", "trash"]), true, start);
        // What was "a" is a gap before "new", all of a slot wide.
        assert_eq!(tracker.room(start).closing, vec![(0, 1.0)]);
        assert!(tracker.room(start + ROOM_TIME).closing.is_empty());
    }

    #[test]
    fn moving_slots_around_makes_no_room() {
        let start = Instant::now();
        let mut tracker = RoomTracker::default();
        tracker.update(keys(&["a", "b", "trash"]), false, start);
        tracker.update(keys(&["b", "a", "trash"]), true, start);
        assert!(!tracker.is_animating());
    }
}
