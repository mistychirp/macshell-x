//! Widgets on the desktop: a calendar, a clock, the weather and photos, opaque tiles with a thin rim
//! like Tahoe's, on a grid down the left of the screen. They can be dragged to another place on it
//! any time. Editing them adds a gallery to choose from, and a way to remove them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, io};

use chrono::{Datelike, Local, Timelike};
use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::text::LineHeight;
use iced_widget::core::{
    Background, Border, Color, ContentFit, Gradient, Length, Point, Radians, gradient,
};
use iced_widget::{
    Space, button, column, container, image, mouse_area, pin, row, stack, svg, text,
};
use serde::{Deserialize, Serialize};

use crate::app::{Element, Message};
use crate::icons::{self, Glyph};
use crate::{fonts, material, menu, menubar, places};

// Measured from Tahoe's.
const SMALL: f32 = 158.0;
const GAP: f32 = 16.0;
const STEP: f32 = SMALL + GAP;
const WIDE: f32 = 2.0 * SMALL + GAP;
const RADIUS: f32 = 22.0;
const MARGIN: f32 = 16.0;
const MINUS_SIZE: f32 = 20.0;
/// How wide a photo's picture is kept.
const PHOTO_PIXELS: u32 = 1200;
/// How many hours a medium weather widget shows.
const HOURS: usize = 6;
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
    Photo { path: Option<PathBuf>, wide: bool },
}

/// What the gallery and the widgets' menus can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Add(Kind),
    Remove(u64),
    /// Medium if true, and small otherwise.
    SetWide(u64, bool),
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
    /// Medium: two places wide.
    #[serde(default)]
    wide: bool,
}

impl Placed {
    fn is_wide(&self) -> bool {
        self.wide || matches!(self.kind, Kind::Photo { wide: true, .. })
    }

    fn columns(&self) -> i32 {
        if self.is_wide() { 2 } else { 1 }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    city: Option<String>,
    widgets: Vec<Placed>,
}

/// The weather somewhere.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Weather {
    pub city: String,
    pub temperature: f32,
    pub high: f32,
    pub low: f32,
    pub code: u32,
    pub is_day: bool,
    /// The next few hours.
    pub hours: Vec<Hour>,
}

/// The forecast for an hour.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hour {
    /// Like "14".
    pub hour: String,
    pub temperature: f32,
    pub code: u32,
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
    photos: HashMap<PathBuf, image::Handle>,
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

    pub fn set_photo_picture(&mut self, path: PathBuf, picture: image::Handle) {
        self.photos.insert(path, picture);
    }

    pub fn photo_handles(&self) -> impl Iterator<Item = &image::Handle> {
        self.photos.values()
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
        let columns = if matches!(kind, Kind::Photo { wide: true, .. }) {
            2
        } else {
            1
        };
        let (column, row) = self.first_free(None, columns);
        self.placed.push(Placed {
            id,
            kind,
            column,
            row,
            wide: false,
        });
        self.save();
        id
    }

    fn first_free(&self, except: Option<u64>, columns: i32) -> (i32, i32) {
        (0..64)
            .flat_map(|row| (0..4).map(move |column| (column, row)))
            .find(|&(column, row)| self.is_free(except, column, row, columns))
            .unwrap_or((0, 0))
    }

    /// Makes a widget medium or small. If there's no room where it is, it goes to the first place
    /// there is.
    pub fn set_wide(&mut self, id: u64, wide: bool) {
        let Some(index) = self.placed.iter().position(|widget| widget.id == id) else {
            return;
        };
        let columns = if wide { 2 } else { 1 };
        let (column, row) = (self.placed[index].column, self.placed[index].row);
        let place = if self.is_free(Some(id), column, row, columns) {
            (column, row)
        } else {
            self.first_free(Some(id), columns)
        };
        let widget = &mut self.placed[index];
        widget.wide = wide;
        if let Kind::Photo {
            wide: photo_wide, ..
        } = &mut widget.kind
        {
            *photo_wide = wide;
        }
        (widget.column, widget.row) = place;
        self.save();
    }

    pub fn remove(&mut self, id: u64) {
        self.placed.retain(|widget| widget.id != id);
        self.save();
    }

    fn is_free(&self, except: Option<u64>, column: i32, row: i32, columns: i32) -> bool {
        self.placed
            .iter()
            .filter(|widget| Some(widget.id) != except)
            .all(|widget| {
                widget.row != row
                    || widget.column + widget.columns() <= column
                    || column + columns <= widget.column
            })
    }

    fn top_left(widget: &Placed) -> Point {
        Point::new(
            MARGIN + widget.column as f32 * STEP,
            menubar::HEIGHT as f32 + MARGIN + widget.row as f32 * STEP,
        )
    }

    /// Where it's drawn, which follows the pointer while it's dragged.
    fn drawn_at(&self, widget: &Placed) -> Point {
        match &self.drag {
            Some(drag) if drag.id == widget.id && drag.moved => Point::new(
                self.pointer.x - drag.grabbed.x,
                self.pointer.y - drag.grabbed.y,
            ),
            _ => Self::top_left(widget),
        }
    }

    fn at(&self, point: Point) -> Option<&Placed> {
        self.placed.iter().rev().find(|widget| {
            let at = Self::top_left(widget);
            let width = widget.columns() as f32 * STEP - GAP;
            point.x >= at.x && point.x < at.x + width && point.y >= at.y && point.y < at.y + SMALL
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
                let at = Self::top_left(widget);
                let (dx, dy) = (point.x - at.x, point.y - at.y);
                dx.hypot(dy) <= MINUS_SIZE / 2.0 + 2.0
            });
            if let Some(widget) = minus {
                return (Some(Action::Remove(widget.id)), true);
            }
        }
        let Some(widget) = self.at(point) else {
            return (None, false);
        };
        let at = Self::top_left(widget);
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

    /// The left button came up: a dragged widget settles in the nearest free place.
    pub fn released(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if !drag.moved {
            return;
        }
        let Some(columns) = self
            .placed
            .iter()
            .find(|widget| widget.id == drag.id)
            .map(Placed::columns)
        else {
            return;
        };
        let x = self.pointer.x - drag.grabbed.x - MARGIN;
        let y = self.pointer.y - drag.grabbed.y - menubar::HEIGHT as f32 - MARGIN;
        let wanted = (x / STEP, y / STEP);
        let nearest = (0..16)
            .flat_map(|row| (0..16).map(move |column| (column, row)))
            .filter(|&(column, row)| self.is_free(Some(drag.id), column, row, columns))
            .min_by(|a, b| {
                let distance = |&(column, row): &(i32, i32)| {
                    (column as f32 - wanted.0).hypot(row as f32 - wanted.1)
                };
                distance(a)
                    .partial_cmp(&distance(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some((column, row)) = nearest
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
                entries.push(widget_item("Choose Photo…", Action::ChoosePhoto(widget.id)))
            }
            Kind::Weather => entries.push(widget_item("Change City…", Action::ChangeCity)),
            _ => {}
        }
        let wide = widget.is_wide();
        entries.push(menu::Entry::item("Size", Message::Nothing).submenu(vec![
            widget_item("Small", Action::SetWide(widget.id, false)).checked(!wide),
            widget_item("Medium", Action::SetWide(widget.id, true)).checked(wide),
        ]));
        entries.push(widget_item("Remove Widget", Action::Remove(widget.id)));
        entries.push(menu::Entry::Separator);
        entries.push(widget_item("Edit Widgets…", Action::Edit));
        Some(entries)
    }

    /// What decides whether the surface has to redraw.
    pub fn content_key(&self) -> String {
        let now = Local::now();
        let dragged = self
            .drag
            .as_ref()
            .filter(|drag| drag.moved)
            .map(|_| self.pointer);
        format!(
            "{:?} {:?} {} {dragged:?} {} {:?}",
            self.placed,
            self.weather,
            self.editing,
            now.format("%F %H:%M"),
            self.photos.keys().collect::<Vec<_>>(),
        )
    }

    /// The widgets, each as a layer of the desktop.
    pub fn layers(&self) -> Vec<Element<'_>> {
        let mut layers = Vec::new();
        for widget in &self.placed {
            let at = self.drawn_at(widget);
            layers.push(pin(self.widget_view(widget)).x(at.x).y(at.y).into());
            if self.editing {
                layers.push(
                    pin(minus_badge())
                        .x(at.x - MINUS_SIZE / 2.0)
                        .y(at.y - MINUS_SIZE / 2.0)
                        .into(),
                );
            }
        }
        layers
    }

    fn widget_view(&self, widget: &Placed) -> Element<'_> {
        let wide = widget.is_wide();
        match &widget.kind {
            Kind::Calendar if wide => calendar_medium(),
            Kind::Calendar => calendar(),
            Kind::Clock if wide => clock_medium(),
            Kind::Clock => clock(),
            Kind::Weather if wide => weather_medium(self.weather.as_ref(), self.city.as_deref()),
            Kind::Weather => weather_view(self.weather.as_ref(), self.city.as_deref()),
            Kind::Photo { path, .. } => {
                let picture = path.as_ref().and_then(|path| self.photos.get(path));
                photo(picture, wide)
            }
        }
    }
}

fn widget_item(label: &str, action: Action) -> menu::Entry {
    menu::Entry::item(label, Message::Widget(action))
}

/// A tile's frame, with its background going from one color to another downward.
fn tile<'a>(
    content: impl Into<Element<'a>>,
    width: f32,
    top: Color,
    bottom: Color,
    padding: f32,
) -> Element<'a> {
    container(content)
        .padding(padding)
        .width(width)
        .height(SMALL)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                gradient::Linear::new(Radians(std::f32::consts::PI))
                    .add_stop(0.0, top)
                    .add_stop(1.0, bottom),
            ))),
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
    container(
        container(Space::new())
            .width(9.0)
            .height(2.0)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::WHITE)),
                ..Default::default()
            }),
    )
    .center_x(MINUS_SIZE)
    .center_y(MINUS_SIZE)
    .style(|_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.1, 0.1, 0.12, 0.92))),
        border: Border {
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.3),
            width: 1.0,
            radius: (MINUS_SIZE / 2.0).into(),
        },
        ..Default::default()
    })
    .into()
}

const CALENDAR_TOP: Color = Color::from_rgb8(62, 62, 62);
const CALENDAR_BOTTOM: Color = Color::from_rgb8(40, 40, 40);
const RED: Color = Color::from_rgb(1.0, 0.27, 0.27);

fn calendar<'a>() -> Element<'a> {
    tile(month(), SMALL, CALENDAR_TOP, CALENDAR_BOTTOM, 0.0)
}

/// Today in big, beside the month.
fn calendar_medium<'a>() -> Element<'a> {
    let today = Local::now().date_naive();
    let big = column![
        text(today.format("%A").to_string().to_uppercase())
            .size(11.0)
            .font(fonts::BOLD)
            .color(RED),
        text(today.day().to_string())
            .size(58.0)
            .line_height(LineHeight::Absolute(64.0.into())),
        text(today.format("%B %Y").to_string())
            .size(11.0)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
    ]
    .spacing(2);
    tile(
        row![
            container(big)
                .padding(iced_widget::core::Padding {
                    top: 15.0,
                    left: 16.0,
                    ..Default::default()
                })
                .width(WIDE - SMALL),
            month(),
        ],
        WIDE,
        CALENDAR_TOP,
        CALENDAR_BOTTOM,
        0.0,
    )
}

/// The month, with today picked out.
fn month<'a>() -> Element<'a> {
    // Measured from Tahoe's.
    const COLUMN: f32 = 19.0;
    const ROW: f32 = 16.0;
    const SIZE: f32 = 10.5;
    const LEFT: f32 = 11.5;
    let today = Local::now().date_naive();
    let first = today.with_day(1).unwrap_or(today);
    let days_in_month = first
        .with_month(first.month() % 12 + 1)
        .and_then(|next| {
            if first.month() == 12 {
                next.with_year(first.year() + 1)
            } else {
                Some(next)
            }
        })
        .map_or(30, |next| (next - first).num_days() as u32);
    let offset = first.weekday().num_days_from_monday();

    let cell = |label: String, color: Color, circle: bool| -> Element<'a> {
        let number = container(
            text(label)
                .size(SIZE)
                .font(fonts::BOLD)
                .color(color)
                .line_height(LineHeight::Absolute(ROW.into())),
        )
        .center_x(ROW)
        .center_y(ROW)
        .style(move |_| {
            if circle {
                container::Style {
                    background: Some(Background::Color(Color::from_rgb(0.94, 0.24, 0.24))),
                    border: Border {
                        radius: (ROW / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            } else {
                container::Style::default()
            }
        });
        container(number).center_x(COLUMN).center_y(ROW).into()
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
                .size(SIZE)
                .font(fonts::BOLD)
                .color(RED),
        )
        .padding(iced_widget::core::Padding {
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
        .chain((1..=days_in_month).map(Some))
        .collect();
    for week in days.chunks(7) {
        lines.push(
            iced_widget::Row::with_children(week.iter().enumerate().map(|(column, day)| {
                let Some(day) = *day else {
                    return Space::new().width(COLUMN).height(ROW).into();
                };
                let is_today = day == today.day();
                let color = if is_today {
                    Color::WHITE
                } else {
                    weekday_color(column as u32)
                };
                cell(day.to_string(), color, is_today)
            }))
            .into(),
        );
    }

    container(iced_widget::Column::with_children(lines))
        .padding(iced_widget::core::Padding {
            top: 15.0,
            left: LEFT,
            ..Default::default()
        })
        .into()
}

const CLOCK_TOP: Color = Color::from_rgb8(50, 50, 52);
const CLOCK_BOTTOM: Color = Color::from_rgb8(26, 26, 28);

fn clock<'a>() -> Element<'a> {
    tile(clock_face(), SMALL, CLOCK_TOP, CLOCK_BOTTOM, 0.0)
}

/// The time in hands and in numbers, with the date.
fn clock_medium<'a>() -> Element<'a> {
    let now = Local::now();
    tile(
        row![
            clock_face(),
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
        WIDE,
        CLOCK_TOP,
        CLOCK_BOTTOM,
        0.0,
    )
}

/// An analog clock, as big as a small widget.
fn clock_face<'a>() -> Element<'a> {
    const NUMBER_RADIUS: f32 = 60.0;
    const NUMBER_BOX: f32 = 24.0;
    let center = SMALL / 2.0;
    let now = Local::now();
    let minutes = now.minute() as f32;
    let hours = (now.hour() % 12) as f32 + minutes / 60.0;

    let hand = |turn: f32, length: f32, width: f32| {
        let radians = turn * std::f32::consts::TAU;
        format!(
            r##"<line x1="50" y1="50" x2="{:.2}" y2="{:.2}" stroke="#fff" stroke-width="{width}" stroke-linecap="round"/>"##,
            50.0 + length * radians.sin(),
            50.0 - length * radians.cos(),
        )
    };
    let hands = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">{}{}<circle cx="50" cy="50" r="2.6" fill="#fff"/></svg>"##,
        hand(hours / 12.0, 23.0, 3.4),
        hand(minutes / 60.0, 33.0, 2.4),
    );

    // Drawn as text: the renderer of SVGs doesn't know the shell's font.
    let mut layers: Vec<Element<'a>> = vec![
        svg(svg::Handle::from_memory(hands.into_bytes()))
            .width(SMALL)
            .height(SMALL)
            .into(),
    ];
    for number in 1..=12 {
        let radians = number as f32 / 12.0 * std::f32::consts::TAU;
        layers.push(
            pin(
                container(text(number.to_string()).size(13.0).font(fonts::BOLD))
                    .center_x(NUMBER_BOX)
                    .center_y(NUMBER_BOX),
            )
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

fn weather_view<'a>(weather: Option<&Weather>, city: Option<&str>) -> Element<'a> {
    let (name, temperature, condition, range, glyph, (top, bottom)) = match weather {
        Some(weather) => (
            weather.city.clone(),
            format!("{:.0}°", weather.temperature),
            condition(weather.code).to_owned(),
            format!("H:{:.0}° L:{:.0}°", weather.high, weather.low),
            condition_glyph(weather.code, weather.is_day),
            sky(weather.code, weather.is_day),
        ),
        None => (
            city.unwrap_or("Weather").to_owned(),
            String::from("--°"),
            String::from(if city.is_some() {
                "Looking it up…"
            } else {
                "Choose a city"
            }),
            String::new(),
            Glyph::Cloud,
            sky(3, true),
        ),
    };
    let line = |words: String, size: f32| {
        text(words)
            .size(size)
            .font(fonts::BOLD)
            .line_height(LineHeight::Absolute((size + 3.0).into()))
    };
    tile(
        column![
            line(name, 14.0),
            text(temperature)
                .size(42.0)
                .line_height(LineHeight::Absolute(50.0.into())),
            Space::new().height(Length::Fill),
            icons::icon(glyph, 18.0, Color::WHITE),
            line(condition, 12.5),
            line(range, 12.5),
        ],
        SMALL,
        top,
        bottom,
        14.0,
    )
}

/// The weather now along the top, and the next few hours under it.
fn weather_medium<'a>(weather: Option<&Weather>, city: Option<&str>) -> Element<'a> {
    let Some(weather) = weather else {
        let (top, bottom) = sky(3, true);
        let words = city.map_or("Choose a city", |_| "Looking it up…");
        return tile(
            column![
                text(city.unwrap_or("Weather").to_owned())
                    .size(14.0)
                    .font(fonts::BOLD),
                text(words).size(12.5),
            ]
            .spacing(4),
            WIDE,
            top,
            bottom,
            14.0,
        );
    };
    let (top, bottom) = sky(weather.code, weather.is_day);
    let now = row![
        column![
            text(weather.city.clone()).size(14.0).font(fonts::BOLD),
            text(format!("{:.0}°", weather.temperature))
                .size(38.0)
                .line_height(LineHeight::Absolute(44.0.into())),
        ],
        Space::new().width(Length::Fill),
        column![
            icons::icon(
                condition_glyph(weather.code, weather.is_day),
                18.0,
                Color::WHITE
            ),
            text(condition(weather.code).to_owned())
                .size(12.5)
                .font(fonts::BOLD),
            text(format!("H:{:.0}° L:{:.0}°", weather.high, weather.low))
                .size(12.5)
                .font(fonts::BOLD),
        ]
        .spacing(2)
        .align_x(iced_widget::core::Alignment::End),
    ];
    let hours = iced_widget::Row::with_children(weather.hours.iter().map(|hour| {
        container(
            column![
                text(hour.hour.clone())
                    .size(11.0)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, 0.75)),
                icons::icon(condition_glyph(hour.code, true), 16.0, Color::WHITE),
                text(format!("{:.0}°", hour.temperature))
                    .size(12.0)
                    .font(fonts::BOLD),
            ]
            .spacing(4)
            .align_x(iced_widget::core::Alignment::Center),
        )
        .center_x(Length::FillPortion(1))
        .into()
    }));
    tile(
        column![now, Space::new().height(Length::Fill), hours],
        WIDE,
        top,
        bottom,
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

fn photo<'a>(picture: Option<&image::Handle>, wide: bool) -> Element<'a> {
    let width = if wide { WIDE } else { SMALL };
    let Some(picture) = picture else {
        return tile(
            container(
                column![
                    icons::icon(Glyph::Photo, 30.0, Color::WHITE),
                    text("Choose a Photo…").size(12.0),
                ]
                .spacing(8)
                .align_x(iced_widget::core::Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
            width,
            Color::from_rgb8(72, 72, 78),
            Color::from_rgb8(48, 48, 54),
            0.0,
        );
    };
    stack![
        image(picture.clone())
            .width(width)
            .height(SMALL)
            .content_fit(ContentFit::Cover)
            .border_radius(RADIUS),
        container(Space::new())
            .width(width)
            .height(SMALL)
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
        margin: Some((0, 0, crate::dock::RESERVED_HEIGHT as i32 + 12, 0)),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from("macshell-widget-gallery")),
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
    container(column![
        row![
            text("Widgets").size(15.0).font(fonts::BOLD),
            Space::new().width(Length::Fill),
            done,
        ]
        .align_y(iced_widget::core::Alignment::Center)
        .height(GALLERY_TITLE - 8.0),
        Space::new().height(8.0),
        row![
            choice(calendar(), Kind::Calendar),
            choice(clock(), Kind::Clock),
            choice(
                weather_view(widgets.weather.as_ref(), widgets.city.as_deref()),
                Kind::Weather
            ),
            choice(
                photo(None, false),
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
pub async fn load_photo(path: PathBuf) -> Option<image::Handle> {
    tokio::task::spawn_blocking(move || read_photo(&path))
        .await
        .ok()
        .flatten()
}

fn read_photo(path: &Path) -> Option<image::Handle> {
    let picture = ::image::open(path)
        .inspect_err(|err| log::warn!("can't read a photo: {err}"))
        .ok()?;
    let small = picture.thumbnail(PHOTO_PIXELS, PHOTO_PIXELS).to_rgba8();
    let (width, height) = small.dimensions();
    Some(image::Handle::from_rgba(width, height, small.into_raw()))
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
struct ForecastHourly {
    /// Like "2026-11-06T14:00".
    time: Vec<String>,
    temperature_2m: Vec<f32>,
    weather_code: Vec<u32>,
}

#[derive(Deserialize)]
struct ForecastCurrent {
    temperature_2m: f32,
    weather_code: u32,
    is_day: u8,
}

#[derive(Deserialize)]
struct ForecastDaily {
    temperature_2m_max: Vec<f32>,
    temperature_2m_min: Vec<f32>,
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
            .query("daily", "temperature_2m_max,temperature_2m_min")
            .query("hourly", "temperature_2m,weather_code")
            .query("forecast_hours", HOURS.to_string())
            .query("timezone", "auto")
            .query("forecast_days", "1")
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
    use super::*;

    #[test]
    fn saved_widgets_from_before_sizes_still_load() {
        let saved: Saved = toml::from_str(
            "city = \"london\"\n\
             [[widgets]]\nid = 0\nkind = \"calendar\"\ncolumn = 0\nrow = 0\n\
             [[widgets]]\nid = 3\ncolumn = 0\nrow = 2\n\
             [widgets.kind.photo]\npath = \"/a.jpg\"\nwide = true\n",
        )
        .expect("old layouts have to keep working");
        assert_eq!(saved.widgets.len(), 2);
        assert!(!saved.widgets[0].is_wide());
        assert!(saved.widgets[1].is_wide());
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
}
