//! Spotlight: a search field with four round buttons, which grows into a panel of applications, files,
//! actions or the clipboard's history. It's a surface as big as the screen, so that a click anywhere
//! else closes it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;
use std::{env, fs};

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings};
use iced_widget::core::text::{LineHeight, Wrapping};
use iced_widget::core::{Alignment, Background, Border, Color, ContentFit, Length, Padding};
use iced_widget::scrollable::{Direction, Scrollbar};
use iced_widget::{
    Id, Space, button, column, container, image, mouse_area, pin, row, scrollable, stack, text,
    text_input,
};

use crate::app::{Element, Message};
use crate::apps::App;
use crate::clipboard::Content;
use crate::icons::{self, Glyph};
use crate::menubar::SystemAction;
use crate::{fonts, material, places};

const WIDTH: f32 = 660.0;
const FIELD_HEIGHT: f32 = 52.0;
const FIELD_WIDTH: f32 = 440.0;
const CHIP_SIZE: f32 = 44.0;
const CHIP_GAP: f32 = 8.0;
const PANEL_HEIGHT: f32 = 500.0;
const PANEL_RADIUS: f32 = 26.0;
const PADDING: f32 = 14.0;
const ROW_HEIGHT: f32 = 44.0;
const CLIP_ROW_HEIGHT: f32 = 52.0;
const COLUMNS: usize = 7;
const CELL_HEIGHT: f32 = 92.0;
/// How long an app's name in the grid gets before it's cut short. In letters rather than measured,
/// since the grid is drawn often and has a lot of names.
const CELL_LABEL_LENGTH: usize = 14;
const ICON_SIZE: f32 = 26.0;
const GRID_ICON_SIZE: f32 = 52.0;
const TEXT_SIZE: f32 = 14.0;
const FIELD_TEXT_SIZE: f32 = 20.0;
const CAPTION_SIZE: f32 = 11.0;
/// How far down the screen it sits, as a share of its height.
const TOP: f32 = 0.16;
const MAX_CLIPS: usize = 50;
const DIM: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.55);

/// Which of its panels it's showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Just the field, or results for what's been typed in it.
    Search,
    Apps,
    Files,
    Actions,
    Clipboard,
}

/// The chips above the grid of applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Productivity,
    Social,
    Entertainment,
    Creativity,
    Development,
    Utilities,
}

impl Category {
    const ALL: [Self; 6] = [
        Self::Productivity,
        Self::Social,
        Self::Entertainment,
        Self::Creativity,
        Self::Development,
        Self::Utilities,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Productivity => "Productivity",
            Self::Social => "Social",
            Self::Entertainment => "Entertainment",
            Self::Creativity => "Creativity",
            Self::Development => "Development",
            Self::Utilities => "Utilities",
        }
    }

    /// Whether an app with these freedesktop categories belongs here.
    fn has(self, categories: &[String]) -> bool {
        let names: &[&str] = match self {
            Self::Productivity => &["Office", "Calendar", "Finance", "TextEditor"],
            Self::Social => &["Chat", "InstantMessaging", "Email", "Network", "WebBrowser"],
            Self::Entertainment => &["AudioVideo", "Audio", "Video", "Game", "Player"],
            Self::Creativity => &["Graphics", "Photography", "Art"],
            Self::Development => &["Development", "IDE"],
            Self::Utilities => &["Utility", "System", "Settings", "FileManager"],
        };
        categories
            .iter()
            .any(|category| names.contains(&category.as_str()))
    }
}

/// Something it can run that isn't an app of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Settings,
    Lock,
    Sleep,
    Restart,
    ShutDown,
    LogOut,
    EmptyTrash,
}

impl Action {
    const ALL: [Self; 7] = [
        Self::Settings,
        Self::Lock,
        Self::Sleep,
        Self::EmptyTrash,
        Self::LogOut,
        Self::Restart,
        Self::ShutDown,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Settings => "System Settings",
            Self::Lock => "Lock Screen",
            Self::Sleep => "Sleep",
            Self::Restart => "Restart",
            Self::ShutDown => "Shut Down",
            Self::LogOut => "Log Out",
            Self::EmptyTrash => "Empty Trash",
        }
    }

    fn glyph(self) -> Glyph {
        match self {
            Self::Settings => Glyph::Gear,
            Self::Lock => Glyph::Lock,
            Self::Sleep => Glyph::Sleep,
            Self::Restart => Glyph::Restart,
            Self::ShutDown => Glyph::Power,
            Self::LogOut => Glyph::Person,
            Self::EmptyTrash => Glyph::Trash,
        }
    }

    /// What it means to the shell.
    pub fn message(self) -> Message {
        match self {
            Self::Settings => Message::OpenSettings,
            Self::Lock => Message::System(SystemAction::Lock),
            Self::Sleep => Message::System(SystemAction::Sleep),
            Self::Restart => Message::System(SystemAction::Restart),
            Self::ShutDown => Message::System(SystemAction::ShutDown),
            Self::LogOut => Message::System(SystemAction::LogOut),
            Self::EmptyTrash => Message::DockMenu(crate::dock::MenuAction::ConfirmEmptyTrash),
        }
    }
}

/// A file it knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

impl FileHit {
    fn of(path: PathBuf) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let is_dir = path.is_dir();
        Some(Self { path, name, is_dir })
    }
}

/// Something that was copied, and when.
#[derive(Debug, Clone)]
pub struct Clip {
    pub content: Content,
    pub copied: chrono::DateTime<chrono::Local>,
    /// What was focused when it was copied, which most likely copied it.
    pub source: Option<Rc<App>>,
}

impl Clip {
    fn title(&self) -> String {
        match &self.content {
            Content::Text(text) => text.trim().lines().next().unwrap_or_default().to_owned(),
            Content::Files(files) => match files.as_slice() {
                [file] => file
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                files => format!("{} files", files.len()),
            },
            Content::Image { mime, .. } => match mime.as_str() {
                "image/jpeg" => String::from("JPEG image"),
                _ => String::from("PNG image"),
            },
        }
    }

    fn detail(&self) -> String {
        let kind = match &self.content {
            Content::Text(_) => "Text",
            Content::Files(_) => "File",
            Content::Image { .. } => "Image",
        };
        format!("{kind} · Copied {}", self.copied.format("%H:%M"))
    }
}

/// One thing that can be chosen.
#[derive(Debug, Clone)]
pub enum Hit {
    App(Rc<App>),
    /// A desktop action of an app: its command, its name, and the app.
    AppAction(Rc<App>, usize),
    File(FileHit),
    Action(Action),
    Clip(usize),
}

/// How well `name` matches what was typed, higher being better, if it does: the start of it, then
/// the start of a word, then anywhere, then its letters in order.
fn score(name: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(1);
    }
    let name = name.to_lowercase();
    let query = query.to_lowercase();
    if name.starts_with(&query) {
        return Some(400);
    }
    if let Some(at) = name.find(&query) {
        let word_start = !name[..at].ends_with(|c: char| c.is_alphanumeric());
        return Some(if word_start { 300 } else { 200 });
    }
    let mut letters = name.chars();
    query
        .chars()
        .all(|wanted| letters.any(|letter| letter == wanted))
        .then_some(100)
}

/// What's in the shell's list of apps, sorted by how well they match, then by name.
fn matching_apps(apps: &[Rc<App>], query: &str) -> Vec<Rc<App>> {
    let mut found: Vec<(u32, &Rc<App>)> = apps
        .iter()
        .filter_map(|app| Some((score(&app.name, query)?, app)))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    found.into_iter().map(|(_, app)| app.clone()).collect()
}

fn matching_actions(apps: &[Rc<App>], query: &str, limit: usize) -> Vec<Hit> {
    let mut found: Vec<(u32, Hit)> = Action::ALL
        .iter()
        .filter_map(|action| Some((word_score(action.name(), query)?, Hit::Action(*action))))
        .collect();
    for app in apps {
        for (index, action) in app.actions.iter().enumerate() {
            let text = format!("{} {}", action.name, app.name);
            if let Some(found_score) = word_score(&text, query) {
                found.push((found_score, Hit::AppAction(app.clone(), index)));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().take(limit).map(|(_, hit)| hit).collect()
}

pub struct Spotlight {
    pub id: iced_layershell::reexport::IcedId,
    pub material: Option<material::MaterialSurface>,
    pub mode: Mode,
    pub query: String,
    pub category: Option<Category>,
    pub selected: usize,
    /// What the last finished search of files found, and what it was for.
    pub files: Vec<FileHit>,
    pub files_query: String,
    pub recent_files: Vec<FileHit>,
    /// The last shapes sent, to tell when they change.
    pub shapes: Vec<material::Shape>,
}

impl Spotlight {
    pub fn new(id: iced_layershell::reexport::IcedId) -> Self {
        Self {
            id,
            material: None,
            mode: Mode::Search,
            query: String::new(),
            category: None,
            selected: 0,
            files: Vec::new(),
            files_query: String::new(),
            recent_files: Vec::new(),
            shapes: Vec::new(),
        }
    }

    pub fn field_id() -> Id {
        Id::new("spotlight-field")
    }

    pub fn results_id() -> Id {
        Id::new("spotlight-results")
    }

    /// Puts the keyboard back in the field. Going between just the field and the whole panel makes
    /// a new one, which doesn't have it.
    pub fn refocus<T: Send + 'static>() -> iced_layershell::reexport::Task<T> {
        iced_layershell::reexport::Task::batch([
            iced_runtime::widget::operation::focus(Self::field_id()),
            iced_runtime::widget::operation::move_cursor_to_end(Self::field_id()),
        ])
    }

    /// Whether it's a whole panel rather than just the field and its buttons.
    pub fn expanded(&self) -> bool {
        self.mode != Mode::Search || !self.query.trim().is_empty()
    }

    /// What can be chosen now, in order.
    pub fn hits(&self, apps: &[Rc<App>], clips: &VecDeque<Clip>) -> Vec<Hit> {
        let query = self.query.trim();
        match self.mode {
            Mode::Search if query.is_empty() => Vec::new(),
            Mode::Search => {
                // What really matches comes first, and apps that only have the letters in order last.
                let (strong, loose): (Vec<Rc<App>>, Vec<Rc<App>>) = matching_apps(apps, query)
                    .into_iter()
                    .partition(|app| word_score(&app.name, query).is_some());
                let mut hits: Vec<Hit> = strong.into_iter().take(6).map(Hit::App).collect();
                hits.extend(matching_actions(apps, query, 4));
                hits.extend(self.file_hits().into_iter().take(8).map(Hit::File));
                hits.extend(loose.into_iter().take(3).map(Hit::App));
                hits
            }
            Mode::Apps => {
                let category = self.category;
                matching_apps(apps, query)
                    .into_iter()
                    .filter(|app| category.is_none_or(|category| category.has(&app.categories)))
                    .map(Hit::App)
                    .collect()
            }
            Mode::Files => self.file_hits().into_iter().map(Hit::File).collect(),
            Mode::Actions => matching_actions(apps, query, 200),
            Mode::Clipboard => clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| score(&clip.title(), query).is_some())
                .map(|(index, _)| Hit::Clip(index))
                .collect(),
        }
    }

    fn file_hits(&self) -> Vec<FileHit> {
        if self.query.trim().is_empty() {
            self.recent_files.clone()
        } else {
            self.files.clone()
        }
    }

    /// Whether what's typed now needs a search of the files.
    pub fn wants_file_search(&self) -> Option<String> {
        let query = self.query.trim();
        let searching = matches!(self.mode, Mode::Search | Mode::Files);
        (searching && !query.is_empty() && query != self.files_query).then(|| query.to_owned())
    }

    /// Which way the arrow keys move in what's showing.
    fn columns(&self) -> usize {
        if self.mode == Mode::Apps { COLUMNS } else { 1 }
    }

    /// Moves the selection, by `rows` down and `across` to the right.
    pub fn select_by(&mut self, rows: i32, across: i32, count: usize) {
        if count == 0 {
            self.selected = 0;
            return;
        }
        let columns = self.columns() as i32;
        let moved = self.selected as i32 + rows * columns + across;
        self.selected = moved.clamp(0, count as i32 - 1) as usize;
    }

    /// How far down its list the selection is, from 0 to 1, for scrolling to it.
    pub fn scroll_share(&self, count: usize) -> f32 {
        let columns = self.columns();
        let rows = count.div_ceil(columns);
        if rows <= 1 {
            0.0
        } else {
            (self.selected / columns) as f32 / (rows - 1) as f32
        }
    }

    /// Where the glass goes on a screen of this size.
    pub fn material_shapes(&self, width: f32, height: f32) -> Vec<material::Shape> {
        let x = ((width - WIDTH) / 2.0).round() as i32;
        let y = (height * TOP).round() as i32;
        let card = |rect, radius| material::Shape {
            rect,
            radius,
            style: material::Style::Card,
        };
        if self.expanded() {
            return vec![card(
                (x, y, WIDTH as i32, PANEL_HEIGHT as i32),
                PANEL_RADIUS as i32,
            )];
        }
        let total = FIELD_WIDTH + 4.0 * (CHIP_GAP + CHIP_SIZE);
        let left = ((width - total) / 2.0).round() as i32;
        let mut shapes = vec![card(
            (left, y, FIELD_WIDTH as i32, FIELD_HEIGHT as i32),
            (FIELD_HEIGHT / 2.0) as i32,
        )];
        let chip_y = y + ((FIELD_HEIGHT - CHIP_SIZE) / 2.0) as i32;
        for index in 0..4 {
            let chip_x =
                left + (FIELD_WIDTH + CHIP_GAP + index as f32 * (CHIP_SIZE + CHIP_GAP)) as i32;
            shapes.push(card(
                (chip_x, chip_y, CHIP_SIZE as i32, CHIP_SIZE as i32),
                (CHIP_SIZE / 2.0) as i32,
            ));
        }
        shapes
    }

    pub fn view<'a>(
        &'a self,
        apps: &'a [Rc<App>],
        clips: &'a VecDeque<Clip>,
        file_icons: &'a FileIcons,
        width: f32,
        height: f32,
    ) -> Element<'a> {
        let top = (height * TOP).round();
        let content: Element<'a> = if self.expanded() {
            let left = ((width - WIDTH) / 2.0).round();
            let hits = self.hits(apps, clips);
            let panel = container(
                column![self.head(), self.body(hits, clips, file_icons)]
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(WIDTH)
            .height(PANEL_HEIGHT);
            pin(mouse_area(panel).on_press(Message::Nothing))
                .x(left)
                .y(top)
                .into()
        } else {
            let total = FIELD_WIDTH + 4.0 * (CHIP_GAP + CHIP_SIZE);
            let left = ((width - total) / 2.0).round();
            let bar = row![
                container(self.field(Glyph::Search, "Spotlight Search"))
                    .width(FIELD_WIDTH)
                    .height(FIELD_HEIGHT)
                    .padding([0.0, 16.0])
                    .center_y(FIELD_HEIGHT),
                Space::new().width(CHIP_GAP),
                chip(Mode::Apps, Glyph::Grid),
                Space::new().width(CHIP_GAP),
                chip(Mode::Files, Glyph::Folder),
                Space::new().width(CHIP_GAP),
                chip(Mode::Actions, Glyph::Stacks),
                Space::new().width(CHIP_GAP),
                chip(Mode::Clipboard, Glyph::Clipboard),
            ]
            .align_y(Alignment::Center);
            pin(mouse_area(bar).on_press(Message::Nothing))
                .x(left)
                .y(top)
                .into()
        };
        mouse_area(
            stack![container(Space::new()).width(width).height(height), content,]
                .width(width)
                .height(height),
        )
        .on_press(Message::SpotlightClose)
        .into()
    }

    fn field<'a>(&'a self, glyph: Glyph, placeholder: &'a str) -> Element<'a> {
        row![
            icons::icon(glyph, 20.0, DIM),
            text_input(placeholder, &self.query)
                .id(Self::field_id())
                .on_input(Message::SpotlightTyped)
                .size(FIELD_TEXT_SIZE)
                .padding(0)
                .style(|_, _| text_input::Style {
                    background: Background::Color(Color::TRANSPARENT),
                    border: Border::default(),
                    icon: Color::WHITE,
                    placeholder: DIM,
                    value: Color::WHITE,
                    selection: crate::menu::ACCENT,
                }),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    }

    fn head(&self) -> Element<'_> {
        let (glyph, placeholder) = match self.mode {
            Mode::Search => (Glyph::Search, "Spotlight Search"),
            Mode::Apps => (Glyph::Grid, "Applications"),
            Mode::Files => (Glyph::Folder, "Files"),
            Mode::Actions => (Glyph::Stacks, "Actions"),
            Mode::Clipboard => (Glyph::Clipboard, "Clipboard"),
        };
        let line = container(Space::new())
            .width(Length::Fill)
            .height(1)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1))),
                ..Default::default()
            });
        column![
            container(self.field(glyph, placeholder))
                .width(Length::Fill)
                .height(FIELD_HEIGHT)
                .padding([0.0, 18.0])
                .center_y(FIELD_HEIGHT),
            line,
        ]
        .into()
    }

    fn body<'a>(
        &'a self,
        hits: Vec<Hit>,
        clips: &'a VecDeque<Clip>,
        file_icons: &'a FileIcons,
    ) -> Element<'a> {
        let list: Element<'a> =
            if hits.is_empty() {
                let message = match self.mode {
                    Mode::Clipboard if self.query.trim().is_empty() => "Nothing copied yet",
                    Mode::Files if self.query.trim().is_empty() => "No recent files",
                    _ if self.query.trim().is_empty() => "",
                    _ => "No results",
                };
                container(text(message).size(TEXT_SIZE).color(DIM))
                    .center_x(Length::Fill)
                    .center_y(160)
                    .into()
            } else if self.mode == Mode::Apps {
                self.grid(&hits)
            } else {
                column(hits.iter().enumerate().map(|(index, hit)| {
                    hit_row(hit, index, index == self.selected, clips, file_icons)
                }))
                .spacing(2)
                .into()
            };
        let scroll = scrollable(container(list).padding(Padding {
            top: 8.0,
            right: PADDING - 6.0,
            bottom: PADDING,
            left: PADDING - 6.0,
        }))
        .id(Self::results_id())
        .direction(Direction::Vertical(
            Scrollbar::new().width(0).scroller_width(0),
        ))
        .width(Length::Fill)
        .height(Length::Fill);
        if self.mode == Mode::Apps {
            column![self.category_chips(), scroll].into()
        } else {
            scroll.into()
        }
    }

    fn category_chips(&self) -> Element<'_> {
        let chips = Category::ALL.iter().map(|category| {
            let selected = self.category == Some(*category);
            let category = *category;
            button(text(category.name()).size(12.0))
                .padding([3, 10])
                .on_press(Message::SpotlightCategory((!selected).then_some(category)))
                .style(move |_, status| {
                    let hovered =
                        matches!(status, button::Status::Hovered | button::Status::Pressed);
                    let background = if selected {
                        crate::menu::ACCENT
                    } else {
                        Color::from_rgba(1.0, 1.0, 1.0, if hovered { 0.18 } else { 0.08 })
                    };
                    button::Style {
                        background: Some(Background::Color(background)),
                        text_color: Color::WHITE,
                        border: Border {
                            radius: 11.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                })
                .into()
        });
        container(iced_widget::Row::with_children(chips).spacing(6))
            .padding(Padding {
                top: 10.0,
                right: PADDING,
                bottom: 0.0,
                left: PADDING,
            })
            .into()
    }

    fn grid<'a>(&self, hits: &[Hit]) -> Element<'a> {
        let selected = self.selected;
        let rows = hits.chunks(COLUMNS).enumerate().map(|(row_index, chunk)| {
            let cells = chunk.iter().enumerate().map(|(column_index, hit)| {
                let index = row_index * COLUMNS + column_index;
                let Hit::App(app) = hit else {
                    return Space::new().into();
                };
                grid_cell(app.clone(), index, index == selected)
            });
            iced_widget::Row::with_children(cells).into()
        });
        iced_widget::Column::with_children(rows).into()
    }
}

/// A round button that opens a panel.
fn chip<'a>(mode: Mode, glyph: Glyph) -> Element<'a> {
    button(
        container(icons::icon(glyph, 20.0, Color::WHITE))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(CHIP_SIZE)
    .height(CHIP_SIZE)
    .padding(0)
    .on_press(Message::SpotlightMode(mode))
    .style(|_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then_some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.14))),
            text_color: Color::WHITE,
            border: Border {
                radius: (CHIP_SIZE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
}

fn app_picture<'a>(app: &App, size: f32) -> Element<'a> {
    match &app.icon {
        Some(handle) => image(handle.clone())
            .width(size)
            .height(size)
            .content_fit(ContentFit::Contain)
            .into(),
        None => container(
            text(
                app.name
                    .chars()
                    .next()
                    .map_or_else(String::new, |c| c.to_uppercase().to_string()),
            )
            .size(size * 0.5),
        )
        .center_x(size)
        .center_y(size)
        .style(move |_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.16))),
            border: Border {
                radius: (size * 0.22).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into(),
    }
}

fn cell_label(name: &str) -> String {
    if name.chars().count() <= CELL_LABEL_LENGTH {
        return name.to_owned();
    }
    let mut short: String = name.chars().take(CELL_LABEL_LENGTH - 1).collect();
    short.push('…');
    short
}

fn grid_cell<'a>(app: Rc<App>, index: usize, selected: bool) -> Element<'a> {
    let name = text(cell_label(&app.name))
        .size(CAPTION_SIZE)
        .wrapping(Wrapping::None)
        .center();
    let cell = container(
        column![
            app_picture(&app, GRID_ICON_SIZE),
            container(name)
                .width(Length::Fill)
                .center_x(Length::Fill)
                .clip(true),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .width(Length::FillPortion(1))
    .height(CELL_HEIGHT)
    .padding([8, 4])
    .style(move |_| selection_style(selected, 14.0));
    mouse_area(cell)
        .on_enter(Message::SpotlightHovered(index))
        .on_press(Message::SpotlightChosen(index))
        .into()
}

fn selection_style(selected: bool, radius: f32) -> container::Style {
    container::Style {
        background: selected.then_some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.14))),
        text_color: Some(Color::WHITE),
        border: Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn glyph_picture<'a>(glyph: Glyph) -> Element<'a> {
    container(icons::icon(glyph, 16.0, Color::WHITE))
        .center_x(ICON_SIZE)
        .center_y(ICON_SIZE)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.14))),
            border: Border {
                radius: 7.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn hit_row<'a>(
    hit: &Hit,
    index: usize,
    selected: bool,
    clips: &'a VecDeque<Clip>,
    file_icons: &'a FileIcons,
) -> Element<'a> {
    let (picture, title, detail, hint): (Element<'a>, String, Option<String>, &str) = match hit {
        Hit::App(app) => (app_picture(app, ICON_SIZE), app.name.clone(), None, "Open"),
        Hit::AppAction(app, action) => (
            app_picture(app, ICON_SIZE),
            app.actions
                .get(*action)
                .map_or_else(String::new, |action| action.name.clone()),
            Some(app.name.clone()),
            "Run",
        ),
        Hit::File(file) => (
            file_icons.picture(file),
            file.name.clone(),
            Some(short_path(&file.path)),
            "Open",
        ),
        Hit::Action(action) => (
            glyph_picture(action.glyph()),
            action.name().to_owned(),
            None,
            "Run",
        ),
        Hit::Clip(clip) => {
            let Some(clip) = clips.get(*clip) else {
                return Space::new().into();
            };
            let picture: Element<'a> = match &clip.content {
                Content::Image { picture, .. } => image(picture.clone())
                    .width(ICON_SIZE + 6.0)
                    .height(ICON_SIZE + 6.0)
                    .content_fit(ContentFit::Cover)
                    .border_radius(6.0)
                    .into(),
                Content::Files(_) => glyph_picture(Glyph::Folder),
                Content::Text(_) => match &clip.source {
                    Some(app) if app.icon.is_some() => app_picture(app, ICON_SIZE),
                    _ => glyph_picture(Glyph::Clipboard),
                },
            };
            (picture, clip.title(), Some(clip.detail()), "Copy")
        }
    };
    let words: Element<'a> = match detail {
        Some(detail) => column![
            line_of(title, TEXT_SIZE, Color::WHITE),
            line_of(detail, CAPTION_SIZE, DIM),
        ]
        .into(),
        None => line_of(title, TEXT_SIZE, Color::WHITE),
    };
    let mut content = row![
        container(picture)
            .center_x(ICON_SIZE + 6.0)
            .center_y(ICON_SIZE + 6.0),
        container(words).width(Length::Fill).clip(true),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    if selected {
        content = content.push(text(hint).size(CAPTION_SIZE).color(DIM));
    }
    let height = if matches!(hit, Hit::Clip(_)) {
        CLIP_ROW_HEIGHT
    } else {
        ROW_HEIGHT
    };
    mouse_area(
        container(content)
            .width(Length::Fill)
            .height(height)
            .padding([0, 10])
            .center_y(height)
            .style(move |_| selection_style(selected, 12.0)),
    )
    .on_enter(Message::SpotlightHovered(index))
    .on_press(Message::SpotlightChosen(index))
    .into()
}

fn line_of<'a>(content: String, size: f32, color: Color) -> Element<'a> {
    text(content)
        .size(size)
        .font(fonts::REGULAR)
        .color(color)
        .line_height(LineHeight::Absolute((size * 1.3).into()))
        .wrapping(Wrapping::None)
        .into()
}

/// Theme icons for files, by what kind of file they are.
#[derive(Debug, Default)]
pub struct FileIcons {
    pub folder: Option<image::Handle>,
    pub image: Option<image::Handle>,
    pub video: Option<image::Handle>,
    pub audio: Option<image::Handle>,
    pub pdf: Option<image::Handle>,
    pub text: Option<image::Handle>,
}

impl FileIcons {
    fn picture<'a>(&self, file: &FileHit) -> Element<'a> {
        let extension = Path::new(&file.name)
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let handle = if file.is_dir {
            &self.folder
        } else {
            match extension.as_str() {
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "bmp" => &self.image,
                "mp4" | "mkv" | "webm" | "mov" | "avi" => &self.video,
                "mp3" | "flac" | "ogg" | "wav" | "m4a" | "opus" => &self.audio,
                "pdf" => &self.pdf,
                _ => &self.text,
            }
        };
        match handle {
            Some(handle) => image(handle.clone())
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .content_fit(ContentFit::Contain)
                .into(),
            None => glyph_picture(if file.is_dir {
                Glyph::Folder
            } else {
                Glyph::Info
            }),
        }
    }
}

/// A path, with the home folder as `~`.
fn short_path(path: &Path) -> String {
    let parent = path.parent().unwrap_or(path);
    match parent.strip_prefix(places::home()) {
        Ok(rest) if rest.as_os_str().is_empty() => String::from("~"),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => parent.display().to_string(),
    }
}

/// Adds what was just copied to the top, moving it there if it's already in the history.
pub fn remember_clip(clips: &mut VecDeque<Clip>, clip: Clip) {
    clips.retain(|known| known.content != clip.content);
    clips.push_front(clip);
    clips.truncate(MAX_CLIPS);
}

pub fn settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Overlay,
        size: Some((0, 0)),
        exclusive_zone: Some(-1),
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        namespace: Some(String::from("macshell-spotlight")),
        ..Default::default()
    }
}

/// How deep a search goes, and how much it looks at before giving up.
const SEARCH_DEPTH: usize = 7;
const SEARCH_ENTRIES: usize = 60_000;
const SEARCH_RESULTS: usize = 60;
/// Folders that are full of files nobody's looking for by name.
const SKIPPED_FOLDERS: [&str; 6] = [
    "node_modules",
    "target",
    "__pycache__",
    "venv",
    "build",
    "dist",
];

/// Files under the home folder with names like `query`, best first. For another thread.
pub fn search_files(query: &str) -> Vec<FileHit> {
    let home = places::home();
    let mut found: Vec<(u32, PathBuf)> = Vec::new();
    let mut folders = VecDeque::from([(home, 0usize)]);
    let mut seen = 0;
    while let Some((folder, depth)) = folders.pop_front() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SEARCH_ENTRIES {
                folders.clear();
                break;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            // Not following links, which can go around in circles.
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if let Some(found_score) = word_score(&name, query) {
                found.push((found_score, path.clone()));
            }
            if is_dir && depth < SEARCH_DEPTH && !SKIPPED_FOLDERS.contains(&name.as_ref()) {
                folders.push_back((path, depth + 1));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found
        .into_iter()
        .take(SEARCH_RESULTS)
        .filter_map(|(_, path)| FileHit::of(path))
        .collect()
}

/// Like [`score`], without the looser letters-in-order match, which finds too much in long names.
fn word_score(name: &str, query: &str) -> Option<u32> {
    score(name, query).filter(|found| *found > 100)
}

/// What Qt, GTK and others list as recently used, newest first.
pub fn recent_files() -> Vec<FileHit> {
    let path = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| places::home().join(".local/share"))
        .join("recently-used.xbel");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_recent(&text)
}

fn parse_recent(text: &str) -> Vec<FileHit> {
    let mut entries: Vec<(SystemTime, PathBuf)> = text
        .match_indices("<bookmark ")
        .filter_map(|(at, _)| {
            let tag = &text[at..at + text[at..].find('>')?];
            let href = tag.split("href=\"").nth(1)?.split('"').next()?;
            let path = crate::portal::path_of(href)?;
            let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok()?;
            Some((modified, path))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries
        .into_iter()
        .take(30)
        .filter_map(|(_, path)| FileHit::of(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_the_start_first() {
        assert_eq!(score("Discord", "dis"), Some(400));
        assert_eq!(score("Visual Studio Code", "stu"), Some(300));
        assert_eq!(score("Discord", "cor"), Some(200));
        assert_eq!(score("Discord", "dcd"), Some(100));
        assert_eq!(score("Discord", "xyz"), None);
        assert_eq!(score("Discord", ""), Some(1));
    }

    #[test]
    fn cuts_long_names_for_the_grid() {
        assert_eq!(cell_label("Alacritty"), "Alacritty");
        let short = cell_label("KDE System Settings Module");
        assert_eq!(short.chars().count(), CELL_LABEL_LENGTH);
        assert!(short.ends_with('…'));
    }

    #[test]
    fn loose_matches_come_after_real_ones() {
        let mut spotlight = Spotlight::new(iced_layershell::reexport::IcedId::unique());
        spotlight.query = String::from("notes");
        spotlight.files = vec![FileHit {
            path: PathBuf::from("/home/me/notes"),
            name: String::from("notes"),
            is_dir: true,
        }];
        spotlight.files_query = String::from("notes");
        let app = |name: &str| {
            Rc::new(App {
                id: name.to_owned(),
                name: name.to_owned(),
                icon: None,
                command: Some(vec![String::from("x")]),
                unlisted: false,
                categories: Vec::new(),
                actions: Vec::new(),
            })
        };
        let apps = vec![app("No Terminal Editors"), app("Notes")];
        let hits = spotlight.hits(&apps, &VecDeque::new());
        assert!(matches!(&hits[0], Hit::App(found) if found.name == "Notes"));
        assert!(matches!(&hits[1], Hit::File(_)));
        assert!(matches!(&hits[2], Hit::App(found) if found.name == "No Terminal Editors"));
    }

    #[test]
    fn files_need_a_real_match() {
        assert_eq!(word_score("notes.txt", "notes"), Some(400));
        assert_eq!(word_score("notes.txt", "nts"), None);
    }

    #[test]
    fn moves_around_a_grid_and_stops_at_its_ends() {
        let mut spotlight = Spotlight::new(iced_layershell::reexport::IcedId::unique());
        spotlight.mode = Mode::Apps;
        spotlight.select_by(1, 0, 20);
        assert_eq!(spotlight.selected, COLUMNS);
        spotlight.select_by(0, 1, 20);
        assert_eq!(spotlight.selected, COLUMNS + 1);
        spotlight.select_by(9, 0, 20);
        assert_eq!(spotlight.selected, 19);
        spotlight.select_by(-9, 0, 20);
        assert_eq!(spotlight.selected, 0);
    }

    #[test]
    fn a_copied_thing_moves_to_the_top_instead_of_repeating() {
        let clip = |text: &str| Clip {
            content: Content::Text(text.to_owned()),
            copied: chrono::Local::now(),
            source: None,
        };
        let mut clips = VecDeque::new();
        remember_clip(&mut clips, clip("a"));
        remember_clip(&mut clips, clip("b"));
        remember_clip(&mut clips, clip("a"));
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].content, Content::Text(String::from("a")));
    }

    #[test]
    fn reads_recent_files_that_still_exist() {
        let file = env::current_exe().unwrap();
        let text = format!(
            "<xbel><bookmark href=\"file://{}\" added=\"x\"><info/></bookmark>\
             <bookmark href=\"file:///no/such/file\"></bookmark></xbel>",
            file.display()
        );
        let found = parse_recent(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, file);
    }
}
