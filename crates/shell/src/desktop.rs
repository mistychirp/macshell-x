//! The desktop: the Desktop folder's files as icons down the right of the screen, which can be
//! selected by clicking or with a rubber band, dragged around, opened and renamed, with a menu for
//! the desktop itself. It's its own surface above the wallpaper and below every window, so that the
//! wallpaper doesn't have to redraw along with a rubber band.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs, io};

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::{Alignment, Background, Border, Color, Length, Point, Rectangle, Size};
use iced_widget::{Space, column, container, image, mouse_area, pin, stack, text, text_input};
use serde::{Deserialize, Serialize};

use crate::app::{Element, Message};
use crate::icons::Glyph;
use crate::{dbus, fonts, menu, menubar, places};

// Measured from Tahoe's desktop.
const GRID_WIDTH: f32 = 92.0;
const GRID_HEIGHT: f32 = 108.0;
const MARGIN_RIGHT: f32 = 18.0;
const MARGIN_TOP: f32 = 10.0;
/// Icons stop short of the bottom of the screen, where the dock is.
const DOCK_CLEARANCE: f32 = 90.0;
const ICON_SIZE: f32 = 52.0;
const BOX_SIZE: f32 = 68.0;
const BOX_RADIUS: f32 = 8.0;
const LABEL_SIZE: f32 = 12.0;
const LABEL_LINE: f32 = 15.0;
const LABEL_GAP: f32 = 4.0;
const LABEL_WIDTH: f32 = GRID_WIDTH - 4.0;
const THUMBNAIL_SIZE: u32 = 128;
/// Files bigger than this keep their generic icon rather than being read for a thumbnail.
const MAX_THUMBNAIL_BYTES: u64 = 64 * 1024 * 1024;
/// How far a press has to move before it's a drag.
const DRAG_DISTANCE: f32 = 4.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Bottom,
        size: Some((0, 0)),
        exclusive_zone: Some(-1),
        // Clicking the desktop takes the keyboard from the active window, like on macOS.
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        namespace: Some(String::from("macshell-desktop")),
        ..Default::default()
    }
}

/// What the menus on the desktop can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewFolder,
    Open,
    Rename,
    MoveToTrash,
    GetInfo,
    ChangeWallpaper,
    EditWidgets,
    SortBy(Sort),
    CleanUp,
    CleanUpBy(Sort),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// Icons stay where they're put.
    #[default]
    None,
    Name,
    Kind,
    Modified,
    Size,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::None => "None",
            Sort::Name => "Name",
            Sort::Kind => "Kind",
            Sort::Modified => "Date Modified",
            Sort::Size => "Size",
        }
    }
}

/// A file in the Desktop folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
    pub len: u64,
}

struct Item {
    entry: Entry,
    /// The top left of its cell.
    position: Point,
    icon: Option<image::Handle>,
}

enum Gesture {
    /// A rubber band, and what was selected before it, which it adds to.
    Rubber { start: Point },
    /// Dragging the selected icons.
    Icons {
        start: Point,
        from: Vec<(PathBuf, Point)>,
        moved: bool,
    },
}

/// What's kept between sessions.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    sort: Sort,
    /// By file name, in distance from the right and the top, so that they survive a change of
    /// screen.
    positions: BTreeMap<String, (f32, f32)>,
}

pub struct Rename {
    pub path: PathBuf,
    pub value: String,
    /// Its field has yet to be given the keyboard. It can't have it until the icon's on screen.
    pub wants_focus: bool,
}

#[derive(Default)]
pub struct Desktop {
    folder: PathBuf,
    items: Vec<Item>,
    positions: BTreeMap<String, (f32, f32)>,
    sort: Sort,
    size: Size,
    pointer: Point,
    gesture: Option<Gesture>,
    selected: HashSet<PathBuf>,
    last_press: Option<(PathBuf, Instant)>,
    pub rename: Option<Rename>,
    /// Ctrl or Shift, which add to the selection.
    pub adding: bool,
}

/// What a press on the desktop turned out to be about.
pub enum Pressed {
    Nothing,
    /// An icon was double-clicked.
    Open,
}

impl Desktop {
    pub fn load() -> Self {
        let saved: Saved = fs::read_to_string(saved_path())
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            folder: desktop_dir(),
            positions: saved.positions,
            sort: saved.sort,
            ..Default::default()
        }
    }

    /// Where the pointer is, in whole logical pixels.
    pub fn pointer(&self) -> (i32, i32) {
        (self.pointer.x as i32, self.pointer.y as i32)
    }

    pub fn folder(&self) -> PathBuf {
        self.folder.clone()
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
        self.arrange();
    }

    /// Whether the field for renaming should be given the keyboard now. Asks once, as soon as its
    /// icon is on screen.
    pub fn take_rename_focus(&mut self) -> bool {
        let items = &self.items;
        match &mut self.rename {
            Some(rename)
                if rename.wants_focus
                    && items.iter().any(|item| item.entry.path == rename.path) =>
            {
                rename.wants_focus = false;
                true
            }
            _ => false,
        }
    }

    /// Takes in what's in the folder now. `icon` finds a generic icon for what's new. Returns the
    /// new files worth reading a thumbnail from.
    pub fn merge(
        &mut self,
        entries: Vec<Entry>,
        mut icon: impl FnMut(&Entry) -> Option<image::Handle>,
    ) -> Vec<PathBuf> {
        let mut old: HashMap<PathBuf, Item> = self
            .items
            .drain(..)
            .map(|item| (item.entry.path.clone(), item))
            .collect();
        let mut wanted = Vec::new();
        self.items = entries
            .into_iter()
            .map(|entry| match old.remove(&entry.path) {
                Some(mut item) => {
                    item.entry = entry;
                    item
                }
                None => {
                    if entry.len <= MAX_THUMBNAIL_BYTES && has_thumbnail(&entry.path) {
                        wanted.push(entry.path.clone());
                    }
                    Item {
                        icon: icon(&entry),
                        entry,
                        position: Point::ORIGIN,
                    }
                }
            })
            .collect();
        // What's gone can't stay selected.
        let items = &self.items;
        self.selected
            .retain(|path| items.iter().any(|item| item.entry.path == *path));
        self.arrange();
        wanted
    }

    pub fn set_thumbnail(&mut self, path: &Path, thumbnail: image::Handle) {
        if let Some(item) = self.items.iter_mut().find(|item| item.entry.path == path) {
            item.icon = Some(thumbnail);
        }
    }

    /// Puts icons where they belong: all of them in order if they're being sorted, and otherwise
    /// where they were left, with any new ones in the first free cell.
    fn arrange(&mut self) {
        if self.size.width <= 0.0 {
            return;
        }
        if self.sort != Sort::None {
            self.sort_into_cells(self.sort);
            return;
        }
        let width = self.size.width;
        let mut unplaced = Vec::new();
        for (index, item) in self.items.iter_mut().enumerate() {
            match self.positions.get(&item.entry.name) {
                Some(&(from_right, y)) => {
                    item.position = Point::new(width - from_right, y);
                }
                None => unplaced.push(index),
            }
        }
        let any_new = !unplaced.is_empty();
        for index in unplaced {
            let position = self.first_free_cell();
            self.place(index, position);
        }
        if any_new {
            self.save();
        }
    }

    fn cell(&self, index: usize) -> Point {
        let top = menubar::HEIGHT as f32 + MARGIN_TOP;
        let rows = (((self.size.height - top - DOCK_CLEARANCE) / GRID_HEIGHT) as usize).max(1);
        Point::new(
            self.size.width - MARGIN_RIGHT - ((index / rows) + 1) as f32 * GRID_WIDTH,
            top + (index % rows) as f32 * GRID_HEIGHT,
        )
    }

    fn first_free_cell(&self) -> Point {
        let width = self.size.width;
        let taken = |cell: Point| {
            self.positions.values().any(|&(from_right, y)| {
                ((width - from_right) - cell.x).abs() < GRID_WIDTH / 2.0
                    && (y - cell.y).abs() < GRID_HEIGHT / 2.0
            })
        };
        (0..)
            .map(|index| self.cell(index))
            .find(|&cell| !taken(cell) || cell.x < 0.0)
            .unwrap_or(Point::ORIGIN)
    }

    fn place(&mut self, index: usize, position: Point) {
        let item = &mut self.items[index];
        item.position = position;
        self.positions.insert(
            item.entry.name.clone(),
            (self.size.width - position.x, position.y),
        );
    }

    fn sort_into_cells(&mut self, sort: Sort) {
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        let items = &self.items;
        let name_key = |index: &usize| items[*index].entry.name.to_lowercase();
        match sort {
            Sort::None | Sort::Name => order.sort_by_key(name_key),
            Sort::Kind => order.sort_by_key(|index| {
                let entry = &items[*index].entry;
                (
                    !entry.is_dir,
                    entry
                        .path
                        .extension()
                        .map(|extension| extension.to_ascii_lowercase()),
                    name_key(index),
                )
            }),
            // Newest and biggest first.
            Sort::Modified => {
                order.sort_by_key(|index| std::cmp::Reverse(items[*index].entry.modified));
            }
            Sort::Size => order.sort_by_key(|index| std::cmp::Reverse(items[*index].entry.len)),
        }
        for (cell, index) in order.into_iter().enumerate() {
            let position = self.cell(cell);
            self.place(index, position);
        }
    }

    /// Snaps every icon to the nearest free cell, keeping them about where they are.
    fn clean_up(&mut self) {
        let mut taken: Vec<Point> = Vec::new();
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        let items = &self.items;
        order.sort_by(|a, b| {
            let (a, b) = (items[*a].position, items[*b].position);
            (a.y, a.x)
                .partial_cmp(&(b.y, b.x))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let cells: Vec<Point> = (0..(self.items.len() * 2 + 64))
            .map(|index| self.cell(index))
            .filter(|cell| cell.x >= 0.0)
            .collect();
        for index in order {
            let at = self.items[index].position;
            let nearest = cells
                .iter()
                .filter(|cell| !taken.contains(cell))
                .min_by(|a, b| {
                    a.distance(at)
                        .partial_cmp(&b.distance(at))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();
            if let Some(cell) = nearest {
                taken.push(cell);
                self.place(index, cell);
            }
        }
    }

    pub fn clean_up_action(&mut self, sort: Option<Sort>) {
        match sort {
            Some(sort) => self.sort_into_cells(sort),
            None => self.clean_up(),
        }
        self.save();
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        self.arrange();
        self.save();
    }

    fn save(&self) {
        let saved = Saved {
            sort: self.sort,
            positions: self.positions.clone(),
        };
        let path = saved_path();
        let result = toml::to_string(&saved)
            .map_err(io::Error::other)
            .and_then(|text| {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, text)
            });
        if let Err(err) = result {
            log::warn!("can't save the desktop's layout: {err}");
        }
    }

    /// The icon's box and its label, in surface coordinates.
    fn hit_rects(item: &Item) -> [Rectangle; 2] {
        let center = item.position.x + GRID_WIDTH / 2.0;
        [
            Rectangle {
                x: center - BOX_SIZE / 2.0,
                y: item.position.y,
                width: BOX_SIZE,
                height: BOX_SIZE,
            },
            Rectangle {
                x: center - LABEL_WIDTH / 2.0,
                y: item.position.y + BOX_SIZE + LABEL_GAP,
                width: LABEL_WIDTH,
                height: 2.0 * LABEL_LINE,
            },
        ]
    }

    fn item_at(&self, point: Point) -> Option<usize> {
        // Topmost first, which is the last one drawn.
        self.items.iter().rposition(|item| {
            Self::hit_rects(item)
                .iter()
                .any(|rect| rect.contains(point))
        })
    }

    pub fn pointer_moved(&mut self, point: Point) {
        self.pointer = point;
        match &mut self.gesture {
            Some(Gesture::Rubber { start }) => {
                let band = rect_between(*start, point);
                let adding = self.adding;
                let mut selected = if adding {
                    self.selected.clone()
                } else {
                    HashSet::new()
                };
                for item in &self.items {
                    if Self::hit_rects(item)
                        .iter()
                        .any(|rect| rect.intersects(&band))
                    {
                        selected.insert(item.entry.path.clone());
                    }
                }
                self.selected = selected;
            }
            Some(Gesture::Icons { start, from, moved }) => {
                let delta = point - *start;
                if !*moved && delta.x.hypot(delta.y) < DRAG_DISTANCE {
                    return;
                }
                *moved = true;
                for (path, origin) in from.iter() {
                    if let Some(item) = self.items.iter_mut().find(|item| item.entry.path == *path)
                    {
                        item.position = *origin + delta;
                    }
                }
            }
            None => {}
        }
    }

    /// The left button went down at the pointer.
    pub fn pressed(&mut self) -> Pressed {
        let point = self.pointer;
        let Some(index) = self.item_at(point) else {
            if !self.adding {
                self.selected.clear();
            }
            self.gesture = Some(Gesture::Rubber { start: point });
            self.last_press = None;
            return Pressed::Nothing;
        };
        let path = self.items[index].entry.path.clone();
        let now = Instant::now();
        let double = self
            .last_press
            .as_ref()
            .is_some_and(|(last, at)| *last == path && now - *at < DOUBLE_CLICK);
        self.last_press = Some((path.clone(), now));

        if self.adding {
            if !self.selected.remove(&path) {
                self.selected.insert(path);
            }
        } else if !self.selected.contains(&path) {
            self.selected = HashSet::from([path]);
        }
        if double {
            return Pressed::Open;
        }
        let from = self
            .items
            .iter()
            .filter(|item| self.selected.contains(&item.entry.path))
            .map(|item| (item.entry.path.clone(), item.position))
            .collect();
        self.gesture = Some(Gesture::Icons {
            start: point,
            from,
            moved: false,
        });
        Pressed::Nothing
    }

    /// The left button came up.
    pub fn released(&mut self) {
        match self.gesture.take() {
            Some(Gesture::Icons {
                from, moved: true, ..
            }) => {
                let dragged: Vec<usize> = (0..self.items.len())
                    .filter(|index| {
                        from.iter()
                            .any(|(path, _)| *path == self.items[*index].entry.path)
                    })
                    .collect();
                for index in dragged {
                    self.place(index, self.items[index].position);
                }
                // Sorted icons go back to their order, like on macOS.
                self.arrange();
                self.save();
            }
            Some(Gesture::Icons { from, .. }) if from.len() > 1 && !self.adding => {
                // A plain click on one of several selected icons narrows it down to that one.
                if let Some(index) = self.item_at(self.pointer) {
                    self.selected = HashSet::from([self.items[index].entry.path.clone()]);
                }
            }
            _ => {}
        }
    }

    /// The right button went down: whatever's under the pointer becomes the selection. Returns the
    /// menu for that, and whether it's on an icon.
    pub fn context_menu(&mut self) -> Vec<menu::Entry> {
        match self.item_at(self.pointer) {
            Some(index) => {
                let path = self.items[index].entry.path.clone();
                if !self.selected.contains(&path) {
                    self.selected = HashSet::from([path]);
                }
                let mut entries = vec![
                    desktop_item("Open", Action::Open),
                    menu::Entry::Separator,
                    desktop_item("Move to Trash", Action::MoveToTrash)
                        .icon(menu::Icon::Glyph(Glyph::Trash)),
                    menu::Entry::Separator,
                    desktop_item("Get Info", Action::GetInfo).icon(menu::Icon::Glyph(Glyph::Info)),
                ];
                if self.selected.len() == 1 {
                    entries.push(desktop_item("Rename", Action::Rename));
                }
                entries
            }
            None => {
                self.selected.clear();
                let sorts = |current: Option<Sort>, action: fn(Sort) -> Action, none: bool| {
                    [
                        Sort::None,
                        Sort::Name,
                        Sort::Kind,
                        Sort::Modified,
                        Sort::Size,
                    ]
                    .into_iter()
                    .filter(|sort| none || *sort != Sort::None)
                    .map(|sort| {
                        desktop_item(sort.label(), action(sort)).checked(current == Some(sort))
                    })
                    .collect::<Vec<_>>()
                };
                vec![
                    desktop_item("New Folder", Action::NewFolder)
                        .icon(menu::Icon::Glyph(Glyph::NewFolder)),
                    menu::Entry::Separator,
                    desktop_item("Get Info", Action::GetInfo).icon(menu::Icon::Glyph(Glyph::Info)),
                    desktop_item("Change Wallpaper…", Action::ChangeWallpaper),
                    desktop_item("Edit Widgets…", Action::EditWidgets),
                    menu::Entry::Separator,
                    desktop_item("Sort By", Action::SortBy(self.sort))
                        .icon(menu::Icon::Glyph(Glyph::Sort))
                        .submenu(sorts(Some(self.sort), Action::SortBy, true)),
                    desktop_item("Clean Up", Action::CleanUp)
                        .icon(menu::Icon::Glyph(Glyph::Grid))
                        .enabled(self.sort == Sort::None),
                    desktop_item("Clean Up By", Action::CleanUp).submenu(sorts(
                        None,
                        Action::CleanUpBy,
                        false,
                    )),
                ]
            }
        }
    }

    /// The icons on show.
    pub fn icons(&self) -> impl Iterator<Item = &image::Handle> {
        self.items.iter().filter_map(|item| item.icon.as_ref())
    }

    pub fn selection(&self) -> Vec<PathBuf> {
        self.items
            .iter()
            .filter(|item| self.selected.contains(&item.entry.path))
            .map(|item| item.entry.path.clone())
            .collect()
    }

    /// Selects a file the moment it's known about, like a new folder.
    pub fn select_only(&mut self, path: PathBuf) {
        self.selected = HashSet::from([path]);
    }

    pub fn start_rename(&mut self, path: PathBuf) {
        let value = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.rename = Some(Rename {
            path,
            value,
            wants_focus: true,
        });
    }

    /// Finishes renaming. The file keeps its name if the new one can't be had.
    pub fn commit_rename(&mut self) {
        let Some(Rename { path, value, .. }) = self.rename.take() else {
            return;
        };
        let value = value.trim();
        let Some(parent) = path.parent() else {
            return;
        };
        let target = parent.join(value);
        if value.is_empty() || value.contains('/') || target == path || target.exists() {
            return;
        }
        if let Err(err) = fs::rename(&path, &target) {
            log::warn!("can't rename {}: {err}", path.display());
            return;
        }
        let old_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if let Some(position) = old_name.and_then(|name| self.positions.remove(&name)) {
            self.positions.insert(value.to_owned(), position);
        }
        if let Some(item) = self.items.iter_mut().find(|item| item.entry.path == path) {
            item.entry.path = target.clone();
            item.entry.name = value.to_owned();
        }
        if self.selected.remove(&path) {
            self.selected.insert(target);
        }
        self.save();
    }

    /// What decides whether the surface has to redraw.
    pub fn content_key(&self, active: bool) -> String {
        let items: Vec<_> = self
            .items
            .iter()
            .map(|item| (&item.entry.name, item.position, item.icon.is_some()))
            .collect();
        let mut selected: Vec<_> = self.selected.iter().collect();
        selected.sort();
        format!(
            "{items:?} {selected:?} {:?} {:?} {active}",
            self.rubber_band(),
            self.rename.as_ref().map(|rename| &rename.value),
        )
    }

    fn rubber_band(&self) -> Option<Rectangle> {
        match &self.gesture {
            Some(Gesture::Rubber { start }) if *start != self.pointer => {
                Some(rect_between(*start, self.pointer))
            }
            _ => None,
        }
    }
}

fn desktop_item(label: &str, action: Action) -> menu::Entry {
    menu::Entry::item(label, Message::Desktop(action))
}

fn rect_between(a: Point, b: Point) -> Rectangle {
    Rectangle {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        width: (a.x - b.x).abs(),
        height: (a.y - b.y).abs(),
    }
}

/// The widget that takes the keyboard while renaming.
pub fn rename_field() -> iced_widget::Id {
    iced_widget::Id::new("desktop-rename")
}

/// `active`: the desktop has the keyboard, rather than some window. Its selection shows in the
/// accent color then, and in grey otherwise.
pub fn view<'a>(desktop: &'a Desktop, active: bool, below: Vec<Element<'a>>) -> Element<'a> {
    let mut layers = below;
    for item in &desktop.items {
        let selected = desktop.selected.contains(&item.entry.path);
        let renaming = desktop
            .rename
            .as_ref()
            .filter(|rename| rename.path == item.entry.path);
        layers.push(
            pin(item_view(item, selected, active, renaming))
                .x(item.position.x)
                .y(item.position.y)
                .into(),
        );
    }
    if let Some(band) = desktop.rubber_band() {
        layers.push(
            pin(container(Space::new())
                .width(band.width)
                .height(band.height)
                .style(|_| container::Style {
                    // Measured from Tahoe's.
                    background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.15))),
                    border: Border {
                        color: Color::from_rgba(1.0, 1.0, 1.0, 0.55),
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                }))
            .x(band.x)
            .y(band.y)
            .into(),
        );
    }
    mouse_area(
        container(stack(layers))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::DesktopPressed)
    .on_right_press(Message::DesktopRightPressed)
    .into()
}

fn item_view<'a>(
    item: &'a Item,
    selected: bool,
    active: bool,
    renaming: Option<&'a Rename>,
) -> Element<'a> {
    let icon: Element<'a> = match &item.icon {
        Some(handle) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        None => Space::new().width(ICON_SIZE).height(ICON_SIZE).into(),
    };
    let tile = container(icon)
        .center_x(BOX_SIZE)
        .center_y(BOX_SIZE)
        .style(move |_| {
            if selected {
                container::Style {
                    background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.4))),
                    border: Border {
                        color: Color::from_rgba(1.0, 1.0, 1.0, 0.35),
                        width: 2.0,
                        radius: BOX_RADIUS.into(),
                    },
                    ..Default::default()
                }
            } else {
                container::Style::default()
            }
        });

    let label: Element<'a> = match renaming {
        Some(rename) => text_input("", &rename.value)
            .id(rename_field())
            .on_input(Message::DesktopRenameChanged)
            .size(LABEL_SIZE)
            .padding([1, 4])
            .style(|_, _| text_input::Style {
                background: Background::Color(Color::WHITE),
                border: Border {
                    color: menu::ACCENT,
                    width: 2.0,
                    radius: 4.0.into(),
                },
                icon: Color::BLACK,
                placeholder: Color::from_rgb(0.5, 0.5, 0.5),
                value: Color::BLACK,
                selection: Color::from_rgba(0.7, 0.18, 0.69, 0.35),
            })
            .into(),
        None => label_view(&item.entry.name, selected, active),
    };

    container(
        column![tile, Space::new().height(LABEL_GAP), label]
            .align_x(Alignment::Center)
            .width(GRID_WIDTH),
    )
    .into()
}

/// A name broken over at most two lines, and shortened on the second if it's still too long.
fn two_lines(name: &str) -> String {
    let width = LABEL_WIDTH - 12.0;
    let fits = |words: &str| menu::text_width(words, LABEL_SIZE, fonts::BOLD) <= width;
    if fits(name) {
        return name.to_owned();
    }
    // The most that fits on the first line, between words if it can.
    let mut end = 0;
    let mut last_space = None;
    for (index, character) in name.char_indices() {
        if !fits(&name[..index + character.len_utf8()]) {
            break;
        }
        end = index + character.len_utf8();
        if character == ' ' {
            last_space = Some(index);
        }
    }
    let (first, rest) = match last_space {
        Some(space) => (&name[..space], name[space + 1..].trim_start()),
        None => name.split_at(end),
    };
    format!(
        "{first}\n{}",
        menu::fit(rest, LABEL_SIZE, fonts::BOLD, width)
    )
}

fn label_view(name: &str, selected: bool, active: bool) -> Element<'_> {
    let shown = two_lines(name);
    let words = |color: Color| {
        text(shown.clone())
            .size(LABEL_SIZE)
            .line_height(iced_widget::core::text::LineHeight::Absolute(
                LABEL_LINE.into(),
            ))
            .font(fonts::BOLD)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .color(color)
    };
    if selected {
        // Measured from Tahoe: its accent when the desktop's active, and light grey when not.
        let (background, ink) = if active {
            (Color::from_rgb8(128, 54, 130), Color::WHITE)
        } else {
            (
                Color::from_rgb8(200, 200, 205),
                Color::from_rgb8(70, 70, 70),
            )
        };
        return container(words(ink))
            .padding([1, 6])
            .max_width(LABEL_WIDTH)
            .style(move |_| container::Style {
                background: Some(Background::Color(background)),
                border: Border {
                    radius: 5.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into();
    }
    // A shadow, so that names stay legible on a bright wallpaper.
    container(stack![
        pin(words(Color::from_rgba(0.0, 0.0, 0.0, 0.6)).width(LABEL_WIDTH)).y(1.0),
        words(Color::WHITE).width(LABEL_WIDTH),
    ])
    .width(LABEL_WIDTH)
    .into()
}

fn has_thumbnail(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg"
            )
        })
}

/// What's in the Desktop folder, without what's hidden.
pub async fn list(folder: PathBuf) -> Vec<Entry> {
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some(Entry {
                path: entry.path(),
                name,
                is_dir: metadata.is_dir(),
                modified: metadata.modified().ok(),
                len: metadata.len(),
            })
        })
        .collect()
}

/// Reads an image into a small picture of itself, on a thread of its own.
pub async fn thumbnail(path: PathBuf) -> Option<image::Handle> {
    tokio::task::spawn_blocking(move || {
        let picture = ::image::open(&path).ok()?;
        let small = picture.thumbnail(THUMBNAIL_SIZE, THUMBNAIL_SIZE).to_rgba8();
        let (width, height) = small.dimensions();
        Some(image::Handle::from_rgba(width, height, small.into_raw()))
    })
    .await
    .ok()
    .flatten()
}

/// Makes a folder called "untitled folder", with a number if that's taken.
pub fn new_folder(folder: &Path) -> io::Result<PathBuf> {
    for number in 1.. {
        let name = if number == 1 {
            String::from("untitled folder")
        } else {
            format!("untitled folder {number}")
        };
        let path = folder.join(name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    unreachable!("there's always another number")
}

/// Shows the file manager's properties window for these.
pub async fn get_info(paths: Vec<PathBuf>) {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        let uris: Vec<String> = paths.iter().map(|path| places::file_uri(path)).collect();
        connection
            .call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItemProperties",
                &(uris, ""),
            )
            .await?;
        zbus::Result::Ok(())
    }
    .await;
    if let Err(err) = result {
        log::warn!("can't show a file's properties: {err}");
    }
}

/// The Desktop folder from the XDG user dirs, or `~/Desktop`.
fn desktop_dir() -> PathBuf {
    places::user_dir("XDG_DESKTOP_DIR").unwrap_or_else(|| places::home().join("Desktop"))
}

fn saved_path() -> PathBuf {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| places::home().join(".local/state"))
        .join("macshell/desktop.toml")
}
