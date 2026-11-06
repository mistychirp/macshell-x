//! Widgets on the desktop: a calendar, a clock, the weather and photos, on a grid down the left of
//! the screen, like Tahoe's. They can be dragged to another place on it any time. Editing them adds
//! a gallery to choose from, and a way to remove them. While a window is active, they turn to tinted
//! glass, and come back in color when the desktop is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, fs, io};

use chrono::{Datelike, Local, NaiveDate, Timelike};
use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::text::LineHeight;
use iced_widget::core::{
    Alignment, Background, Border, Color, ContentFit, Gradient, Length, Padding, Point, Radians,
    gradient,
};
use iced_widget::{
    Space, button, column, container, image, mouse_area, pin, row, stack, svg, text,
};
use serde::{Deserialize, Serialize};

use crate::anim;
use crate::app::{Element, Message};
use crate::group::group;
use crate::icons::{self, Glyph};
use crate::{fonts, material, menu, menubar, places};

// Measured from Tahoe's.
const SMALL: f32 = 158.0;
const GAP: f32 = 16.0;
const STEP: f32 = SMALL + GAP;
const RADIUS: f32 = 22.0;
/// How long widgets take to crossfade between color and glass, measured from Tahoe's.
const TINT_TIME: Duration = Duration::from_millis(170);
const MARGIN: f32 = 16.0;
const MINUS_SIZE: f32 = 18.0;
/// Where the middle of a widget's minus badge is, from its top left corner.
const MINUS_AT: f32 = 5.0;
/// How far the outline of where a dragged widget would go is outside that place.
const SLOT_OUTSET: f32 = 4.0;
const SLOT_LINE: f32 = 2.5;
/// How dark the parts of the screen widgets can't go are, while one's dragged.
const NO_GO_SHADE: f32 = 0.3;
/// How wide a photo's picture is kept.
const PHOTO_PIXELS: u32 = 1200;
/// How many hours a medium weather widget shows, and days a large one.
const HOURS: usize = 6;
const DAYS: usize = 6;
/// How often the weather's looked up.
pub const WEATHER_INTERVAL: Duration = Duration::from_secs(30 * 60);

const GALLERY_PADDING: f32 = 18.0;
const GALLERY_TITLE: f32 = 36.0;
const GALLERY_RADIUS: f32 = 26.0;
const GALLERY_WIDTH: f32 = 4.0 * SMALL + 3.0 * GAP + 2.0 * GALLERY_PADDING;
const GALLERY_HEIGHT: f32 = SMALL + GALLERY_TITLE + 2.0 * GALLERY_PADDING;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Calendar,
    Clock,
    Weather,
    Photo {
        path: Option<PathBuf>,
        /// From before there were sizes.
        #[serde(default)]
        wide: bool,
    },
}

impl Kind {
    fn sizes(&self) -> &'static [WidgetSize] {
        use WidgetSize::{ExtraLarge, Large, Medium, Small};
        match self {
            Kind::Calendar | Kind::Weather => &[Small, Medium, Large],
            Kind::Clock => &[Small, Medium],
            Kind::Photo { .. } => &[Small, Medium, Large, ExtraLarge],
        }
    }

    /// Whether it's glass whatever's active: the clock's dial sits on it.
    fn is_always_glass(&self) -> bool {
        matches!(self, Kind::Clock)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSize {
    Small,
    Medium,
    Large,
    ExtraLarge,
}

impl WidgetSize {
    fn cells(self) -> (i32, i32) {
        match self {
            Self::Small => (1, 1),
            Self::Medium => (2, 1),
            Self::Large => (2, 2),
            Self::ExtraLarge => (4, 2),
        }
    }

    fn pixels(self) -> (f32, f32) {
        let (columns, rows) = self.cells();
        (columns as f32 * STEP - GAP, rows as f32 * STEP - GAP)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
            Self::ExtraLarge => "Extra Large",
        }
    }
}

/// What the gallery and the widgets' menus can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Add(Kind),
    Remove(u64),
    SetSize(u64, WidgetSize),
    ChoosePhoto(u64),
    ChangeCity,
    Edit,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Placed {
    id: u64,
    kind: Kind,
    column: i32,
    row: i32,
    /// From before there were more sizes than this.
    #[serde(default)]
    wide: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<WidgetSize>,
}

impl Placed {
    fn size(&self) -> WidgetSize {
        self.size.unwrap_or(
            if self.wide || matches!(self.kind, Kind::Photo { wide: true, .. }) {
                WidgetSize::Medium
            } else {
                WidgetSize::Small
            },
        )
    }

    fn top_left(&self) -> Point {
        top_left(self.column, self.row)
    }
}

fn top_left(column: i32, row: i32) -> Point {
    Point::new(
        MARGIN + column as f32 * STEP,
        menubar::HEIGHT as f32 + MARGIN + row as f32 * STEP,
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    city: Option<String>,
    widgets: Vec<Placed>,
}

/// The weather somewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Weather {
    pub city: String,
    pub temperature: f32,
    pub high: f32,
    pub low: f32,
    pub code: u32,
    pub is_day: bool,
    /// The next few hours.
    pub hours: Vec<Hour>,
    /// The next few days.
    pub days: Vec<Day>,
}

/// The forecast for an hour.
#[derive(Debug, Clone, PartialEq)]
pub struct Hour {
    /// Like "14".
    pub hour: String,
    pub temperature: f32,
    pub code: u32,
}

/// The forecast for a day.
#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    /// Like "Mon".
    pub name: String,
    pub high: f32,
    pub low: f32,
    pub code: u32,
}

/// A photo's picture, in color and for when widgets are glass.
#[derive(Debug, Clone)]
pub struct Picture {
    color: image::Handle,
    tinted: image::Handle,
}

struct Drag {
    id: u64,
    /// Where in the widget it's held.
    grabbed: Point,
    start: Point,
    moved: bool,
}

pub struct Widgets {
    placed: Vec<Placed>,
    pub city: Option<String>,
    pub weather: Option<Weather>,
    photos: HashMap<PathBuf, Picture>,
    pub editing: bool,
    drag: Option<Drag>,
    pointer: Point,
    next_id: u64,
}

impl Widgets {
    pub fn load() -> Self {
        let saved: Option<Saved> = fs::read_to_string(saved_path())
            .ok()
            .and_then(|text| toml::from_str(&text).ok());
        let (placed, city) = match saved {
            Some(saved) => (saved.widgets, saved.city),
            // A calendar to start with: it needs nothing from anywhere.
            None => (
                vec![Placed {
                    id: 0,
                    kind: Kind::Calendar,
                    column: 0,
                    row: 0,
                    wide: false,
                    size: None,
                }],
                None,
            ),
        };
        let next_id = placed.iter().map(|widget| widget.id + 1).max().unwrap_or(0);
        Self {
            placed,
            city,
            weather: None,
            photos: HashMap::new(),
            editing: false,
            drag: None,
            pointer: Point::ORIGIN,
            next_id,
        }
    }

    fn save(&self) {
        let saved = Saved {
            city: self.city.clone(),
            widgets: self.placed.clone(),
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
            log::warn!("can't save the widgets: {err}");
        }
    }

    pub fn has_weather(&self) -> bool {
        self.placed
            .iter()
            .any(|widget| widget.kind == Kind::Weather)
    }

    pub fn set_city(&mut self, city: String) {
        self.city = Some(city);
        self.weather = None;
        self.save();
    }

    /// Photos on the grid that haven't been read yet.
    pub fn photos_to_load(&self) -> Vec<PathBuf> {
        self.placed
            .iter()
            .filter_map(|widget| match &widget.kind {
                Kind::Photo {
                    path: Some(path), ..
                } if !self.photos.contains_key(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn set_photo_picture(&mut self, path: PathBuf, picture: Picture) {
        self.photos.insert(path, picture);
    }

    pub fn photo_handles(&self) -> impl Iterator<Item = &image::Handle> {
        self.photos
            .values()
            .flat_map(|picture| [&picture.color, &picture.tinted])
    }

    pub fn set_photo(&mut self, id: u64, chosen: PathBuf) {
        if let Some(widget) = self.placed.iter_mut().find(|widget| widget.id == id)
            && let Kind::Photo { path, .. } = &mut widget.kind
        {
            *path = Some(chosen);
            self.save();
        }
    }

    /// Adds a widget in the first free place. Returns its id.
    pub fn add(&mut self, kind: Kind) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let (column, row) = self.first_free(None, WidgetSize::Small);
        self.placed.push(Placed {
            id,
            kind,
            column,
            row,
            wide: false,
            size: Some(WidgetSize::Small),
        });
        self.save();
        id
    }

    pub fn remove(&mut self, id: u64) {
        self.placed.retain(|widget| widget.id != id);
        self.save();
    }

    fn first_free(&self, except: Option<u64>, size: WidgetSize) -> (i32, i32) {
        (0..64)
            .flat_map(|row| (0..4).map(move |column| (column, row)))
            .find(|&(column, row)| self.is_free(except, column, row, size))
            .unwrap_or((0, 0))
    }

    /// Resizes a widget. If it doesn't fit where it is at that size, it goes to the first place it
    /// does.
    pub fn set_size(&mut self, id: u64, size: WidgetSize) {
        let Some(index) = self.placed.iter().position(|widget| widget.id == id) else {
            return;
        };
        let (column, row) = (self.placed[index].column, self.placed[index].row);
        let place = if self.is_free(Some(id), column, row, size) {
            (column, row)
        } else {
            self.first_free(Some(id), size)
        };
        let widget = &mut self.placed[index];
        widget.size = Some(size);
        (widget.column, widget.row) = place;
        self.save();
    }

    fn is_free(&self, except: Option<u64>, column: i32, row: i32, size: WidgetSize) -> bool {
        let (columns, rows) = size.cells();
        self.placed
            .iter()
            .filter(|widget| Some(widget.id) != except)
            .all(|widget| {
                let (other_columns, other_rows) = widget.size().cells();
                widget.column + other_columns <= column
                    || column + columns <= widget.column
                    || widget.row + other_rows <= row
                    || row + rows <= widget.row
            })
    }

    /// Where it's drawn, which follows the pointer while it's dragged.
    fn drawn_at(&self, widget: &Placed) -> Point {
        match &self.drag {
            Some(drag) if drag.id == widget.id && drag.moved => Point::new(
                self.pointer.x - drag.grabbed.x,
                self.pointer.y - drag.grabbed.y,
            ),
            _ => widget.top_left(),
        }
    }

    fn at(&self, point: Point) -> Option<&Placed> {
        self.placed.iter().rev().find(|widget| {
            let at = widget.top_left();
            let (width, height) = widget.size().pixels();
            point.x >= at.x && point.x < at.x + width && point.y >= at.y && point.y < at.y + height
        })
    }

    pub fn pointer_moved(&mut self, point: Point) {
        self.pointer = point;
        if let Some(drag) = &mut self.drag {
            let distance = (point.x - drag.start.x).hypot(point.y - drag.start.y);
            drag.moved |= distance >= 4.0;
        }
    }

    /// The left button went down. Returns an action if that's what it's on, and whether it was on a
    /// widget at all.
    pub fn pressed(&mut self) -> (Option<Action>, bool) {
        let point = self.pointer;
        if self.editing {
            let minus = self.placed.iter().rev().find(|widget| {
                let at = widget.top_left();
                let (dx, dy) = (point.x - at.x - MINUS_AT, point.y - at.y - MINUS_AT);
                dx.hypot(dy) <= MINUS_SIZE / 2.0 + 2.0
            });
            if let Some(widget) = minus {
                return (Some(Action::Remove(widget.id)), true);
            }
        }
        let Some(widget) = self.at(point) else {
            return (None, false);
        };
        let at = widget.top_left();
        let id = widget.id;
        let action = match &widget.kind {
            Kind::Photo { path: None, .. } => Some(Action::ChoosePhoto(id)),
            _ => None,
        };
        self.drag = Some(Drag {
            id,
            grabbed: Point::new(point.x - at.x, point.y - at.y),
            start: point,
            moved: false,
        });
        (action, true)
    }

    /// The free place nearest to where a widget's being dragged.
    fn nearest_free(&self, drag: &Drag) -> Option<(i32, i32)> {
        let size = self
            .placed
            .iter()
            .find(|widget| widget.id == drag.id)?
            .size();
        let x = self.pointer.x - drag.grabbed.x - MARGIN;
        let y = self.pointer.y - drag.grabbed.y - menubar::HEIGHT as f32 - MARGIN;
        let wanted = (x / STEP, y / STEP);
        (0..16)
            .flat_map(|row| (0..16).map(move |column| (column, row)))
            .filter(|&(column, row)| self.is_free(Some(drag.id), column, row, size))
            .min_by(|a, b| {
                let distance = |&(column, row): &(i32, i32)| {
                    (column as f32 - wanted.0).hypot(row as f32 - wanted.1)
                };
                distance(a)
                    .partial_cmp(&distance(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// The left button came up: a dragged widget settles in the nearest free place.
    pub fn released(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if !drag.moved {
            return;
        }
        if let Some((column, row)) = self.nearest_free(&drag)
            && let Some(widget) = self.placed.iter_mut().find(|widget| widget.id == drag.id)
        {
            widget.column = column;
            widget.row = row;
        }
        self.save();
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The menu for the widget under the pointer, if there is one.
    pub fn context_menu(&self) -> Option<Vec<menu::Entry>> {
        let widget = self.at(self.pointer)?;
        let mut entries = Vec::new();
        match &widget.kind {
            Kind::Photo { .. } => {
                entries.push(widget_item("Edit “Photo”…", Action::ChoosePhoto(widget.id)));
                entries.push(menu::Entry::Separator);
            }
            Kind::Weather => {
                entries.push(widget_item("Edit “Forecast”…", Action::ChangeCity));
                entries.push(menu::Entry::Separator);
            }
            _ => {}
        }
        entries.push(menu::Entry::item("Size", Message::Nothing).enabled(false));
        let current = widget.size();
        for &size in widget.kind.sizes() {
            entries.push(
                widget_item(size.label(), Action::SetSize(widget.id, size))
                    .checked(size == current),
            );
        }
        entries.push(menu::Entry::Separator);
        entries.push(widget_item("Remove Widget", Action::Remove(widget.id)));
        entries.push(menu::Entry::Separator);
        entries.push(widget_item("Edit Widgets…", Action::Edit));
        Some(entries)
    }

    /// The glass behind widgets: all of them when they're tinted, and otherwise the ones that always
    /// are.
    pub fn material_shapes(&self, tinted: bool) -> Vec<material::Shape> {
        self.placed
            .iter()
            .filter(|widget| tinted || widget.kind.is_always_glass())
            .map(|widget| {
                let at = self.drawn_at(widget);
                let (width, height) = widget.size().pixels();
                material::Shape {
                    rect: (at.x as i32, at.y as i32, width as i32, height as i32),
                    radius: RADIUS as i32,
                    style: material::Style::Card,
                }
            })
            .collect()
    }

    /// What decides whether the surface has to redraw.
    pub fn content_key(&self, tint: f32) -> String {
        let now = Local::now();
        let dragged = self
            .drag
            .as_ref()
            .filter(|drag| drag.moved)
            .map(|_| self.pointer);
        // The clock's second hand needs a redraw every second; everything else every minute.
        let has_clock = self.placed.iter().any(|widget| widget.kind == Kind::Clock);
        let time = if has_clock {
            now.format("%F %T")
        } else {
            now.format("%F %H:%M")
        };
        format!(
            "{:?} {:?} {} {dragged:?} {time} {:?} {tint}",
            self.placed,
            self.weather,
            self.editing,
            self.photos.keys().collect::<Vec<_>>(),
        )
    }

    /// The widgets, each as a layer of the desktop. `tint`: how far along they are in turning to
    /// glass, which they are for a window being active.
    pub fn layers(&self, tint: f32, screen_height: f32) -> Vec<Element<'_>> {
        let mut layers = Vec::new();
        let dragging = self.drag.as_ref().filter(|drag| drag.moved);
        if let Some(drag) = dragging {
            layers.extend(no_go_shade(screen_height));
            if let Some((column, row)) = self.nearest_free(drag)
                && let Some(widget) = self.placed.iter().find(|widget| widget.id == drag.id)
            {
                layers.push(slot_outline(column, row, widget.size()));
            }
        }
        // What's being dragged is on top of the rest.
        let mut order: Vec<&Placed> = self.placed.iter().collect();
        order.sort_by_key(|widget| dragging.is_some_and(|drag| drag.id == widget.id));
        for widget in order {
            let at = self.drawn_at(widget);
            layers.push(pin(self.crossfaded(widget, tint)).x(at.x).y(at.y).into());
            if self.editing {
                layers.push(
                    pin(minus_badge())
                        .x(at.x + MINUS_AT - MINUS_SIZE / 2.0)
                        .y(at.y + MINUS_AT - MINUS_SIZE / 2.0)
                        .into(),
                );
            }
        }
        layers
    }

    /// A widget part of the way between its two looks. Color goes over the glass one at full strength
    /// where it's opaque, so that no wallpaper shows through in the middle.
    fn crossfaded(&self, widget: &Placed, tint: f32) -> Element<'_> {
        if tint <= 0.0 {
            return self.widget_view(widget, false);
        }
        if tint >= 1.0 {
            return self.widget_view(widget, true);
        }
        let under = if widget.kind.is_always_glass() {
            tint
        } else {
            1.0
        };
        stack![
            group(self.widget_view(widget, true)).opacity(under),
            group(self.widget_view(widget, false)).opacity(1.0 - tint),
        ]
        .into()
    }

    fn widget_view(&self, widget: &Placed, tinted: bool) -> Element<'_> {
        let size = widget.size();
        match &widget.kind {
            Kind::Calendar => calendar(size, tinted),
            Kind::Clock => clock(size, tinted),
            Kind::Weather => {
                weather_view(self.weather.as_ref(), self.city.as_deref(), size, tinted)
            }
            Kind::Photo { path, .. } => {
                let picture = path.as_ref().and_then(|path| self.photos.get(path));
                photo(picture, size, tinted)
            }
        }
    }
}

fn widget_item(label: &str, action: Action) -> menu::Entry {
    menu::Entry::item(label, Message::Widget(action))
}

/// Darkens the parts of the screen widgets can't go: under the bar, and the margin on the left.
fn no_go_shade<'a>(screen_height: f32) -> [Element<'a>; 2] {
    let shade = |width: Length, height: f32| -> Element<'a> {
        container(Space::new())
            .width(width)
            .height(height)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(
                    0.0,
                    0.0,
                    0.0,
                    NO_GO_SHADE,
                ))),
                ..Default::default()
            })
            .into()
    };
    let bar = menubar::HEIGHT as f32;
    [
        // Pinned, like everything else: a stack takes its size from its first layer.
        pin(shade(Length::Fill, bar)).into(),
        pin(shade(MARGIN.into(), screen_height - bar - MARGIN))
            .y(bar + MARGIN)
            .into(),
    ]
}

/// The outline of where a dragged widget would land.
fn slot_outline<'a>(column: i32, row: i32, size: WidgetSize) -> Element<'a> {
    let at = top_left(column, row);
    let (width, height) = size.pixels();
    pin(container(Space::new())
        .width(width + 2.0 * SLOT_OUTSET)
        .height(height + 2.0 * SLOT_OUTSET)
        .style(|_| container::Style {
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.3),
                width: SLOT_LINE,
                radius: (RADIUS + SLOT_OUTSET).into(),
            },
            ..Default::default()
        }))
    .x(at.x - SLOT_OUTSET)
    .y(at.y - SLOT_OUTSET)
    .into()
}

/// How far along widgets are in turning to glass, as a crossfade that can change its mind.
#[derive(Debug)]
pub struct Tint {
    from: f32,
    to: bool,
    started: Instant,
}

impl Default for Tint {
    fn default() -> Self {
        Self {
            from: 0.0,
            to: false,
            started: Instant::now() - TINT_TIME,
        }
    }
}

impl Tint {
    pub fn aim(&mut self, tinted: bool) {
        if tinted != self.to {
            self.from = self.value();
            self.to = tinted;
            self.started = anim::now();
        }
    }

    pub fn value(&self) -> f32 {
        let t = (anim::seconds_since(self.started) / TINT_TIME.as_secs_f32()).min(1.0);
        let to = if self.to { 1.0 } else { 0.0 };
        self.from + (to - self.from) * t * t * (3.0 - 2.0 * t)
    }

    pub fn is_animating(&self) -> bool {
        anim::seconds_since(self.started) < TINT_TIME.as_secs_f32()
    }
}

/// A widget's frame, with its background going from one color to another downward, or none at all
/// when `tinted`, where the glass behind it shows.
fn tile<'a>(
    content: impl Into<Element<'a>>,
    size: WidgetSize,
    colors: (Color, Color),
    tinted: bool,
    padding: f32,
) -> Element<'a> {
    let (width, height) = size.pixels();
    let (top, bottom) = colors;
    container(content)
        .padding(padding)
        .width(width)
        .height(height)
        .clip(true)
        .style(move |_| container::Style {
            background: (!tinted).then(|| {
                Background::Gradient(Gradient::Linear(
                    gradient::Linear::new(Radians(std::f32::consts::PI))
                        .add_stop(0.0, top)
                        .add_stop(1.0, bottom),
                ))
            }),
            border: rim(),
            text_color: Some(Color::WHITE),
            ..Default::default()
        })
        .into()
}

fn rim() -> Border {
    Border {
        color: Color::from_rgba(1.0, 1.0, 1.0, 0.22),
        width: 1.0,
        radius: RADIUS.into(),
    }
}

fn minus_badge<'a>() -> Element<'a> {
    // Measured from Tahoe's: pale and see-through, with a darker line.
    container(
        container(Space::new())
            .width(8.0)
            .height(1.5)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgb8(128, 130, 145))),
                ..Default::default()
            }),
    )
    .center_x(MINUS_SIZE)
    .center_y(MINUS_SIZE)
    .style(|_| container::Style {
        background: Some(Background::Color(Color::from_rgba8(205, 205, 222, 0.85))),
        border: Border {
            radius: (MINUS_SIZE / 2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

const CALENDAR_COLORS: (Color, Color) =
    (Color::from_rgb8(62, 62, 62), Color::from_rgb8(40, 40, 40));
const RED: Color = Color::from_rgb(1.0, 0.27, 0.27);

fn calendar<'a>(size: WidgetSize, tinted: bool) -> Element<'a> {
    let accent = if tinted { Color::WHITE } else { RED };
    let content: Element<'a> = match size {
        WidgetSize::Small => month(MonthScale::SMALL, tinted),
        WidgetSize::Medium => {
            // Today in big, beside the month.
            let today = Local::now().date_naive();
            let (width, _) = size.pixels();
            row![
                container(
                    column![
                        text(today.format("%A").to_string().to_uppercase())
                            .size(11.0)
                            .font(fonts::BOLD)
                            .color(accent),
                        text(today.day().to_string())
                            .size(58.0)
                            .line_height(LineHeight::Absolute(64.0.into())),
                        text(today.format("%B %Y").to_string())
                            .size(11.0)
                            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
                    ]
                    .spacing(2),
                )
                .padding(Padding {
                    top: 15.0,
                    left: 16.0,
                    ..Default::default()
                })
                .width(width - SMALL),
                month(MonthScale::SMALL, tinted),
            ]
            .into()
        }
        _ => month(MonthScale::LARGE, tinted),
    };
    tile(content, size, CALENDAR_COLORS, tinted, 0.0)
}

/// How big a month is drawn.
#[derive(Clone, Copy)]
struct MonthScale {
    column: f32,
    row: f32,
    text: f32,
    left: f32,
    top: f32,
}

impl MonthScale {
    // Measured from Tahoe's.
    const SMALL: Self = Self {
        column: 19.0,
        row: 16.0,
        text: 10.5,
        left: 11.5,
        top: 15.0,
    };
    const LARGE: Self = Self {
        column: 42.0,
        row: 38.0,
        text: 16.0,
        left: 19.0,
        top: 18.0,
    };
}

/// The number of days in the month of a day.
fn days_in_month(day: NaiveDate) -> u32 {
    let first = day.with_day(1).unwrap_or(day);
    let next = if first.month() == 12 {
        first
            .with_year(first.year() + 1)
            .and_then(|first| first.with_month(1))
    } else {
        first.with_month(first.month() + 1)
    };
    next.map_or(30, |next| (next - first).num_days() as u32)
}

/// The month, with today picked out.
fn month<'a>(scale: MonthScale, tinted: bool) -> Element<'a> {
    let today = Local::now().date_naive();
    let first = today.with_day(1).unwrap_or(today);
    let offset = first.weekday().num_days_from_monday();
    let circle = scale.row.min(scale.column);

    let cell = move |label: String, color: Color, is_today: bool| -> Element<'a> {
        let number = container(
            text(label)
                .size(scale.text)
                .font(fonts::BOLD)
                .color(color)
                .line_height(LineHeight::Absolute(circle.into())),
        )
        .center_x(circle)
        .center_y(circle)
        .style(move |_| {
            if is_today {
                container::Style {
                    background: Some(Background::Color(if tinted {
                        Color::from_rgb8(215, 215, 225)
                    } else {
                        Color::from_rgb(0.94, 0.24, 0.24)
                    })),
                    border: Border {
                        radius: (circle / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            } else {
                container::Style::default()
            }
        });
        container(number)
            .center_x(scale.column)
            .center_y(scale.row)
            .into()
    };
    let weekday_color = |column: u32| {
        if column >= 5 {
            Color::from_rgba(1.0, 1.0, 1.0, 0.55)
        } else {
            Color::WHITE
        }
    };

    let mut lines: Vec<Element<'a>> = vec![
        container(
            text(first.format("%B").to_string().to_uppercase())
                .size(scale.text)
                .font(fonts::BOLD)
                .color(if tinted {
                    Color::from_rgba(1.0, 1.0, 1.0, 0.75)
                } else {
                    RED
                }),
        )
        .padding(Padding {
            left: 5.0,
            ..Default::default()
        })
        .into(),
        iced_widget::Row::with_children((0..7).map(|column| {
            cell(
                ["M", "T", "W", "T", "F", "S", "S"][column as usize].to_owned(),
                weekday_color(column),
                false,
            )
        }))
        .into(),
    ];
    // Blanks up to the first day, then the days, in rows of seven.
    let days: Vec<Option<u32>> = (0..offset)
        .map(|_| None)
        .chain((1..=days_in_month(today)).map(Some))
        .collect();
    for week in days.chunks(7) {
        lines.push(
            iced_widget::Row::with_children(week.iter().enumerate().map(|(column, day)| {
                let Some(day) = *day else {
                    return Space::new().width(scale.column).height(scale.row).into();
                };
                let is_today = day == today.day();
                let color = match (is_today, tinted) {
                    (true, true) => Color::from_rgb8(40, 40, 60),
                    (true, false) => Color::WHITE,
                    _ => weekday_color(column as u32),
                };
                cell(day.to_string(), color, is_today)
            }))
            .into(),
        );
    }

    container(iced_widget::Column::with_children(lines))
        .padding(Padding {
            top: scale.top,
            left: scale.left,
            ..Default::default()
        })
        .into()
}

/// A dial on glass, measured from Tahoe's, with the time in numbers beside it at medium.
fn clock<'a>(size: WidgetSize, tinted: bool) -> Element<'a> {
    let colors = (Color::TRANSPARENT, Color::TRANSPARENT);
    if size == WidgetSize::Small {
        return tile(dial(tinted), size, colors, true, 0.0);
    }
    let now = Local::now();
    tile(
        row![
            dial(tinted),
            container(
                column![
                    text(now.format("%H:%M").to_string())
                        .size(38.0)
                        .font(fonts::BOLD),
                    text(now.format("%A").to_string()).size(13.0),
                    text(now.format("%B %-d").to_string())
                        .size(13.0)
                        .color(Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
                ]
                .spacing(2),
            )
            .center_y(Length::Fill),
        ],
        size,
        colors,
        true,
        0.0,
    )
}

/// An analog clock, as big as a small widget.
fn dial<'a>(tinted: bool) -> Element<'a> {
    // Out of a hundred, in the drawing's own units.
    const NUMBER_RADIUS: f32 = 49.0;
    const NUMBER_BOX: f32 = 24.0;
    let center = SMALL / 2.0;
    let now = Local::now();
    let seconds = now.second() as f32;
    let minutes = now.minute() as f32 + seconds / 60.0;
    let hours = (now.hour() % 12) as f32 + minutes / 60.0;
    let (face, ink) = if tinted {
        ("#d8d8e4", Color::from_rgb8(40, 40, 60))
    } else {
        ("#ffffff", Color::from_rgb8(20, 20, 22))
    };
    let ink_css = if tinted { "#28283c" } else { "#141416" };

    let line = |turn: f32, from: f32, to: f32, width: f32, color: &str| {
        let radians = turn * std::f32::consts::TAU;
        format!(
            r##"<line x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}" stroke="{color}" stroke-width="{width}" stroke-linecap="round"/>"##,
            50.0 + from * radians.sin(),
            50.0 - from * radians.cos(),
            50.0 + to * radians.sin(),
            50.0 - to * radians.cos(),
        )
    };
    let mut ticks = String::new();
    for tick in 0..60 {
        let turn = tick as f32 / 60.0;
        if tick % 5 == 0 {
            ticks.push_str(&line(turn, 39.0, 42.5, 0.9, ink_css));
        } else {
            ticks.push_str(&line(turn, 40.5, 42.5, 0.5, "#a0a0a8"));
        }
    }
    let drawing = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="44" fill="{face}"/>{ticks}{}{}{}<circle cx="50" cy="50" r="1.8" fill="#ffffff" stroke="#f09a1a" stroke-width="1"/></svg>"##,
        line(hours / 12.0, 0.0, 21.0, 3.0, ink_css),
        line(minutes / 60.0, 0.0, 33.0, 2.2, ink_css),
        line(seconds / 60.0, -8.0, 36.0, 0.6, "#f09a1a"),
    );

    // Numbers are text: the renderer of SVGs doesn't know the shell's font.
    let mut layers: Vec<Element<'a>> = vec![
        svg(svg::Handle::from_memory(drawing.into_bytes()))
            .width(SMALL)
            .height(SMALL)
            .into(),
    ];
    for number in 1..=12 {
        let radians = number as f32 / 12.0 * std::f32::consts::TAU;
        layers.push(
            pin(container(
                text(number.to_string())
                    .size(14.0)
                    .font(fonts::BOLD)
                    .color(ink),
            )
            .center_x(NUMBER_BOX)
            .center_y(NUMBER_BOX))
            .x(center + NUMBER_RADIUS * radians.sin() - NUMBER_BOX / 2.0)
            .y(center - NUMBER_RADIUS * radians.cos() - NUMBER_BOX / 2.0)
            .into(),
        );
    }
    iced_widget::Stack::with_children(layers)
        .width(SMALL)
        .height(SMALL)
        .into()
}

fn weather_view<'a>(
    weather: Option<&Weather>,
    city: Option<&str>,
    size: WidgetSize,
    tinted: bool,
) -> Element<'a> {
    let Some(weather) = weather else {
        return tile(
            column![
                text(city.unwrap_or("Weather").to_owned())
                    .size(14.0)
                    .font(fonts::BOLD),
                text("--°")
                    .size(42.0)
                    .line_height(LineHeight::Absolute(50.0.into())),
                Space::new().height(Length::Fill),
                icons::icon(Glyph::Cloud, 18.0, Color::WHITE),
                text(if city.is_some() {
                    "Looking it up…"
                } else {
                    "Choose a city"
                })
                .size(12.5)
                .font(fonts::BOLD),
            ],
            size,
            sky(3, true),
            tinted,
            14.0,
        );
    };
    let colors = sky(weather.code, weather.is_day);
    let glyph = condition_glyph(weather.code, weather.is_day);
    let bold = |words: String, size: f32| {
        text(words)
            .size(size)
            .font(fonts::BOLD)
            .line_height(LineHeight::Absolute((size + 3.0).into()))
    };
    let condition_name = condition(weather.code).to_owned();
    let range = format!("H:{:.0}° L:{:.0}°", weather.high, weather.low);
    let temperature = format!("{:.0}°", weather.temperature);
    if size == WidgetSize::Small {
        return tile(
            column![
                bold(weather.city.clone(), 14.0),
                text(temperature)
                    .size(42.0)
                    .line_height(LineHeight::Absolute(50.0.into())),
                Space::new().height(Length::Fill),
                icons::icon(glyph, 18.0, Color::WHITE),
                bold(condition_name, 12.5),
                bold(range, 12.5),
            ],
            size,
            colors,
            tinted,
            14.0,
        );
    }
    let now = row![
        column![
            bold(weather.city.clone(), 14.0),
            text(temperature)
                .size(38.0)
                .line_height(LineHeight::Absolute(44.0.into())),
        ],
        Space::new().width(Length::Fill),
        column![
            icons::icon(glyph, 18.0, Color::WHITE),
            bold(condition_name, 12.5),
            bold(range, 12.5),
        ]
        .spacing(2)
        .align_x(Alignment::End),
    ];
    let hours = iced_widget::Row::with_children(weather.hours.iter().map(|hour| {
        container(
            column![
                text(hour.hour.clone())
                    .size(11.0)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, 0.75)),
                icons::icon(condition_glyph(hour.code, true), 16.0, Color::WHITE),
                bold(format!("{:.0}°", hour.temperature), 12.0),
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        )
        .center_x(Length::FillPortion(1))
        .into()
    }));
    if size == WidgetSize::Medium {
        return tile(
            column![now, Space::new().height(Length::Fill), hours],
            size,
            colors,
            tinted,
            14.0,
        );
    }
    // The large one adds the next few days.
    let days = iced_widget::Column::with_children(weather.days.iter().map(|day| {
        row![
            text(day.name.clone()).size(12.5).width(44.0),
            icons::icon(condition_glyph(day.code, true), 16.0, Color::WHITE),
            Space::new().width(Length::Fill),
            text(format!("{:.0}°", day.low))
                .size(12.5)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.65)),
            Space::new().width(12.0),
            text(format!("{:.0}°", day.high)).size(12.5),
        ]
        .align_y(Alignment::Center)
        .height(22.0)
        .into()
    }));
    tile(
        column![
            now,
            Space::new().height(12.0),
            hours,
            Space::new().height(Length::Fill),
            days,
        ],
        size,
        colors,
        tinted,
        14.0,
    )
}

/// What the weather's called, for a WMO weather code.
fn condition(code: u32) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly Clear",
        2 => "Partly Cloudy",
        3 => "Cloudy",
        45 | 48 => "Foggy",
        51..=57 => "Drizzle",
        61..=67 => "Rain",
        71..=77 => "Snow",
        80..=82 => "Showers",
        85 | 86 => "Snow Showers",
        95..=99 => "Thunderstorm",
        _ => "",
    }
}

fn condition_glyph(code: u32, is_day: bool) -> Glyph {
    match code {
        0 | 1 if is_day => Glyph::SunHigh,
        0 | 1 => Glyph::Moon,
        51..=67 | 80..=82 => Glyph::Rain,
        71..=77 | 85 | 86 => Glyph::Snow,
        95..=99 => Glyph::Bolt,
        _ => Glyph::Cloud,
    }
}

/// A widget's colors for the weather, top and bottom. Tahoe's cloudy sky is measured.
fn sky(code: u32, is_day: bool) -> (Color, Color) {
    let rgb = Color::from_rgb8;
    match (code, is_day) {
        (_, false) => (rgb(24, 32, 62), rgb(44, 52, 88)),
        (0..=2, true) => (rgb(42, 112, 196), rgb(104, 168, 224)),
        (51..=67 | 80..=82 | 95..=99, true) => (rgb(84, 98, 116), rgb(58, 70, 88)),
        _ => (rgb(150, 165, 180), rgb(95, 115, 135)),
    }
}

fn photo<'a>(picture: Option<&Picture>, size: WidgetSize, tinted: bool) -> Element<'a> {
    let (width, height) = size.pixels();
    let Some(picture) = picture else {
        return tile(
            container(
                column![
                    icons::icon(Glyph::Photo, 30.0, Color::WHITE),
                    text("Choose a Photo…").size(12.0),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
            size,
            (Color::from_rgb8(72, 72, 78), Color::from_rgb8(48, 48, 54)),
            tinted,
            0.0,
        );
    };
    let handle = if tinted {
        &picture.tinted
    } else {
        &picture.color
    };
    stack![
        image(handle.clone())
            .width(width)
            .height(height)
            .content_fit(ContentFit::Cover)
            .opacity(if tinted { 0.88_f32 } else { 1.0_f32 })
            .border_radius(RADIUS),
        container(Space::new())
            .width(width)
            .height(height)
            .style(|_| container::Style {
                border: rim(),
                ..Default::default()
            }),
    ]
    .into()
}

pub fn gallery_settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Bottom,
        layer: Layer::Top,
        size: Some((GALLERY_WIDTH as u32, GALLERY_HEIGHT as u32)),
        exclusive_zone: Some(-1),
        margin: Some((0, 0, crate::dock::reserved_height() as i32 + 12, 0)),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("dock")),
        ..Default::default()
    }
}

pub fn gallery_shapes() -> Vec<material::Shape> {
    vec![material::Shape {
        rect: (0, 0, GALLERY_WIDTH as i32, GALLERY_HEIGHT as i32),
        radius: GALLERY_RADIUS as i32,
        style: material::Style::Frosted,
    }]
}

/// Choosing a widget to add: each one's the real thing, at its real size.
pub fn gallery<'a>(widgets: &'a Widgets, material: Option<material::Kind>) -> Element<'a> {
    let choice = |preview: Element<'a>, kind: Kind| -> Element<'a> {
        mouse_area(preview)
            .on_press(Message::Widget(Action::Add(kind)))
            .into()
    };
    let done = button(
        container(text("Done").size(13.0).font(fonts::BOLD))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .on_press(Message::Widget(Action::Done))
    .width(72)
    .height(26)
    .padding(0)
    .style(|_, status| button::Style {
        background: Some(Background::Color(Color {
            a: if status == button::Status::Hovered {
                1.0
            } else {
                0.9
            },
            ..Color::from_rgb(0.63, 0.31, 0.66)
        })),
        text_color: Color::WHITE,
        border: Border {
            radius: 13.0.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    let tint = material::panel_tint(material);
    let small = WidgetSize::Small;
    container(column![
        row![
            text("Widgets").size(15.0).font(fonts::BOLD),
            Space::new().width(Length::Fill),
            done,
        ]
        .align_y(Alignment::Center)
        .height(GALLERY_TITLE - 8.0),
        Space::new().height(8.0),
        row![
            choice(calendar(small, false), Kind::Calendar),
            choice(clock(small, false), Kind::Clock),
            choice(
                weather_view(
                    widgets.weather.as_ref(),
                    widgets.city.as_deref(),
                    small,
                    false
                ),
                Kind::Weather
            ),
            choice(
                photo(None, small, false),
                Kind::Photo {
                    path: None,
                    wide: false
                }
            ),
        ]
        .spacing(GAP),
    ])
    .padding(GALLERY_PADDING)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.1, 0.1, 0.12, tint))),
        border: material::panel_border(material, GALLERY_RADIUS),
        text_color: Some(Color::WHITE),
        ..Default::default()
    })
    .into()
}

/// Reads a photo into a picture that's big enough, on a thread of its own.
pub async fn load_photo(path: PathBuf) -> Option<Picture> {
    tokio::task::spawn_blocking(move || read_photo(&path))
        .await
        .ok()
        .flatten()
}

fn read_photo(path: &Path) -> Option<Picture> {
    let picture = ::image::open(path)
        .inspect_err(|err| log::warn!("can't read a photo: {err}"))
        .ok()?;
    let small = picture.thumbnail(PHOTO_PIXELS, PHOTO_PIXELS).to_rgba8();
    let (width, height) = small.dimensions();
    let tinted = tint_pixels(small.as_raw());
    Some(Picture {
        color: image::Handle::from_rgba(width, height, small.into_raw()),
        tinted: image::Handle::from_rgba(width, height, tinted),
    })
}

/// A picture the way it looks when widgets are glass, measured from Tahoe's: only its brightness is
/// left, from a dark navy to a pale lavender.
fn tint_pixels(pixels: &[u8]) -> Vec<u8> {
    const DARK: [f32; 3] = [30.0, 26.0, 66.0];
    const LIGHT: [f32; 3] = [228.0, 222.0, 234.0];
    let mut tinted = Vec::with_capacity(pixels.len());
    for pixel in pixels.chunks_exact(4) {
        let brightness = (0.2126 * f32::from(pixel[0])
            + 0.7152 * f32::from(pixel[1])
            + 0.0722 * f32::from(pixel[2]))
            / 255.0;
        for channel in 0..3 {
            tinted.push((DARK[channel] + (LIGHT[channel] - DARK[channel]) * brightness) as u8);
        }
        tinted.push(pixel[3]);
    }
    tinted
}

#[derive(Deserialize)]
struct GeocodingAnswer {
    results: Option<Vec<GeocodingResult>>,
}

#[derive(Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
}

#[derive(Deserialize)]
struct ForecastAnswer {
    current: ForecastCurrent,
    daily: ForecastDaily,
    hourly: ForecastHourly,
}

#[derive(Deserialize)]
struct ForecastCurrent {
    temperature_2m: f32,
    weather_code: u32,
    is_day: u8,
}

#[derive(Deserialize)]
struct ForecastDaily {
    /// Like "2026-11-06".
    time: Vec<String>,
    temperature_2m_max: Vec<f32>,
    temperature_2m_min: Vec<f32>,
    weather_code: Vec<u32>,
}

#[derive(Deserialize)]
struct ForecastHourly {
    /// Like "2026-11-06T14:00".
    time: Vec<String>,
    temperature_2m: Vec<f32>,
    weather_code: Vec<u32>,
}

/// Looks up the weather in a city, from Open-Meteo, which needs no account.
pub async fn fetch_weather(city: String) -> Option<Weather> {
    tokio::task::spawn_blocking(move || read_weather(&city))
        .await
        .ok()
        .flatten()
}

fn hours_of(hourly: &ForecastHourly) -> Vec<Hour> {
    hourly
        .time
        .iter()
        .zip(&hourly.temperature_2m)
        .zip(&hourly.weather_code)
        .map(|((time, temperature), code)| Hour {
            hour: time
                .split_once('T')
                .map_or(time.as_str(), |(_, hour)| {
                    hour.split_once(':').map_or(hour, |(hour, _)| hour)
                })
                .to_owned(),
            temperature: *temperature,
            code: *code,
        })
        .collect()
}

fn days_of(daily: &ForecastDaily) -> Vec<Day> {
    daily
        .time
        .iter()
        .zip(&daily.temperature_2m_max)
        .zip(&daily.temperature_2m_min)
        .zip(&daily.weather_code)
        .map(|(((time, high), low), code)| Day {
            name: NaiveDate::parse_from_str(time, "%Y-%m-%d")
                .map_or_else(|_| time.clone(), |day| day.format("%a").to_string()),
            high: *high,
            low: *low,
            code: *code,
        })
        .collect()
}

fn read_weather(city: &str) -> Option<Weather> {
    let result = (|| -> Result<Option<Weather>, ureq::Error> {
        let places: GeocodingAnswer = ureq::get("https://geocoding-api.open-meteo.com/v1/search")
            .query("name", city)
            .query("count", "1")
            .call()?
            .body_mut()
            .read_json()?;
        let Some(place) = places.results.and_then(|mut results| results.pop()) else {
            log::warn!("no weather for a city of that name");
            return Ok(None);
        };
        let forecast: ForecastAnswer = ureq::get("https://api.open-meteo.com/v1/forecast")
            .query("latitude", place.latitude.to_string())
            .query("longitude", place.longitude.to_string())
            .query("current", "temperature_2m,weather_code,is_day")
            .query(
                "daily",
                "temperature_2m_max,temperature_2m_min,weather_code",
            )
            .query("hourly", "temperature_2m,weather_code")
            .query("forecast_hours", HOURS.to_string())
            .query("forecast_days", DAYS.to_string())
            .query("timezone", "auto")
            .call()?
            .body_mut()
            .read_json()?;
        Ok(Some(Weather {
            city: place.name,
            temperature: forecast.current.temperature_2m,
            high: *forecast.daily.temperature_2m_max.first().unwrap_or(&0.0),
            low: *forecast.daily.temperature_2m_min.first().unwrap_or(&0.0),
            code: forecast.current.weather_code,
            is_day: forecast.current.is_day != 0,
            hours: hours_of(&forecast.hourly),
            days: days_of(&forecast.daily),
        }))
    })();
    result
        .inspect_err(|err| log::warn!("can't look up the weather: {err}"))
        .ok()
        .flatten()
}

fn saved_path() -> PathBuf {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| places::home().join(".local/state"))
        .join("macshell/widgets.toml")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_tint_goes_there_and_back_from_wherever_it_got_to() {
        let mut tint = super::Tint::default();
        assert_eq!(tint.value(), 0.0);
        tint.aim(true);
        assert!(tint.is_animating());
        tint.started -= super::TINT_TIME;
        assert_eq!(tint.value(), 1.0);
        assert!(!tint.is_animating());
        tint.aim(false);
        assert!(tint.value() > 0.9);
        tint.started -= super::TINT_TIME;
        assert_eq!(tint.value(), 0.0);
    }

    use super::*;

    #[test]
    fn saved_widgets_from_before_sizes_still_load() {
        let saved: Saved = toml::from_str(
            "city = \"london\"\n\
             [[widgets]]\nid = 0\nkind = \"calendar\"\ncolumn = 0\nrow = 0\n\
             [[widgets]]\nid = 1\nkind = \"clock\"\ncolumn = 0\nrow = 1\nwide = true\n\
             [[widgets]]\nid = 3\ncolumn = 0\nrow = 2\n\
             [widgets.kind.photo]\npath = \"/a.jpg\"\nwide = true\n",
        )
        .expect("old layouts have to keep working");
        assert_eq!(saved.widgets.len(), 3);
        assert_eq!(saved.widgets[0].size(), WidgetSize::Small);
        assert_eq!(saved.widgets[1].size(), WidgetSize::Medium);
        assert_eq!(saved.widgets[2].size(), WidgetSize::Medium);
    }

    #[test]
    fn saved_widgets_survive_a_round_trip() {
        let saved = Saved {
            city: None,
            widgets: vec![Placed {
                id: 7,
                kind: Kind::Photo {
                    path: Some(PathBuf::from("/a.jpg")),
                    wide: false,
                },
                column: 1,
                row: 2,
                wide: false,
                size: Some(WidgetSize::ExtraLarge),
            }],
        };
        let text = toml::to_string(&saved).expect("it has to save");
        let loaded: Saved = toml::from_str(&text).expect("what's saved has to load");
        assert_eq!(loaded.widgets, saved.widgets);
    }

    #[test]
    fn hours_read_as_just_the_hour() {
        let hours = hours_of(&ForecastHourly {
            time: vec![String::from("2026-11-06T14:00")],
            temperature_2m: vec![9.5],
            weather_code: vec![3],
        });
        assert_eq!(hours[0].hour, "14");
    }

    #[test]
    fn a_big_widget_needs_all_of_its_places_free() {
        let mut widgets = Widgets {
            placed: Vec::new(),
            city: None,
            weather: None,
            photos: HashMap::new(),
            editing: false,
            drag: None,
            pointer: Point::ORIGIN,
            next_id: 0,
        };
        widgets.placed.push(Placed {
            id: 0,
            kind: Kind::Calendar,
            column: 1,
            row: 1,
            wide: false,
            size: Some(WidgetSize::Small),
        });
        assert!(widgets.is_free(None, 0, 0, WidgetSize::Medium));
        assert!(!widgets.is_free(None, 0, 0, WidgetSize::Large));
        assert!(!widgets.is_free(None, 0, 1, WidgetSize::Medium));
        assert_eq!(widgets.first_free(None, WidgetSize::Large), (2, 0));
    }
}
