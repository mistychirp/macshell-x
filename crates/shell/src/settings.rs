//! The Settings app: a window of its own, in a process of its own (`macshell settings`), so that it
//! comes and goes like any other app. The glass settings go to our KWin plugin over D-Bus, which
//! applies them right away. The rest go to the config file, and the shell is told to read it again.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use iced_futures::Subscription;
use iced_layershell::actions::IcedXdgWindowSettings;
use iced_layershell::daemon;
use iced_layershell::reexport::{IcedId, Task, WithConnection};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;
use iced_widget::core::{
    Alignment, Background, Border, Color, ContentFit, Event, Length, Padding, Theme, theme, window,
};
use iced_widget::renderer::Renderer;
use iced_widget::scrollable::{Direction, Scrollbar};
use iced_widget::{
    Space, button, column, container, image, mouse_area, row, scrollable, slider, text, toggler,
};
use wayland_client::Connection;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::config::{self, Config};
use crate::icons::{self, Glyph};
use crate::material::{self, Material, MaterialSurface};
use crate::settings_file::SettingsFile;
use crate::{apps, dbus, fonts, menu, portal, wallpaper};

pub const APP_ID: &str = "macshell-settings";
const BUS_NAME: &str = "org.macshell.Settings";

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 560.0;
const RADIUS: f32 = 26.0;
/// The sidebar floats inside the window, like Tahoe's.
const SIDEBAR_INSET: f32 = 8.0;
const SIDEBAR_WIDTH: f32 = 196.0;
const SIDEBAR_RADIUS: f32 = 18.0;
const LIGHT_SIZE: f32 = 14.0;
const LIGHT_SPACING: f32 = 9.0;
const TEXT_SIZE: f32 = 13.0;
const CAPTION_SIZE: f32 = 11.0;
const TITLE_SIZE: f32 = 15.0;
const CARD_RADIUS: f32 = 14.0;
const DIM: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.55);

type Element<'a> = iced_widget::core::Element<'a, Message, Theme, Renderer>;

/// A setting of the glass: its key in our KWin plugin, its name here, and how far it goes.
struct GlassSlider {
    key: &'static str,
    label: &'static str,
    range: (f32, f32),
    step: f32,
}

const GLASS_SLIDERS: [GlassSlider; 7] = [
    GlassSlider {
        key: "GlassThickness",
        label: "Thickness",
        range: (5.0, 100.0),
        step: 0.5,
    },
    GlassSlider {
        key: "GlassBezel",
        label: "Edge Width",
        range: (4.0, 40.0),
        step: 0.5,
    },
    GlassSlider {
        key: "GlassProfile",
        label: "Edge Shape",
        range: (1.5, 5.0),
        step: 0.05,
    },
    GlassSlider {
        key: "GlassIor",
        label: "Refraction",
        range: (1.1, 2.2),
        step: 0.01,
    },
    GlassSlider {
        key: "GlassIorSpread",
        label: "Color Split",
        range: (0.0, 0.1),
        step: 0.002,
    },
    GlassSlider {
        key: "GlassGloss",
        label: "Highlights",
        range: (0.0, 1.0),
        step: 0.01,
    },
    GlassSlider {
        key: "Grain",
        label: "Grain",
        range: (0.0, 0.03),
        step: 0.001,
    },
];
const GREEN_FULL_SCREEN: &str = "GreenFullScreen";
/// Where an import keeps the settings it replaces.
const BACKUP_NAME: &str = "before-import.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Glass,
    Desktop,
    Windows,
    About,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::Glass => "Glass",
            Page::Desktop => "Desktop & Dock",
            Page::Windows => "Windows",
            Page::About => "About",
        }
    }
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Opened(IcedId),
    Closed,
    Reached(Option<WlSurface>),
    Dragged,
    Close,
    Minimize,
    LightsHovered(bool),
    ShowPage(Page),
    /// Opens one of KDE's settings modules, for what we don't do ourselves yet.
    OpenSystemPage(&'static str),
    /// The plugin's current settings, if it's there.
    GlassRead(Option<HashMap<&'static str, f32>>),
    GlassChanged(&'static str, f32),
    GlassReleased(&'static str),
    GlassReset,
    MagnificationToggled(bool),
    MagnifiedSizeChanged(f32),
    MagnifiedSizeReleased,
    AlwaysGlassyToggled(bool),
    ChooseWallpaper,
    WallpaperChosen(Option<PathBuf>),
    WallpaperPictureRead(Option<image::Handle>),
    GreenFullScreenChosen(bool),
    ExportSettings,
    ExportTo(Option<PathBuf>),
    ImportSettings,
    ImportFrom(Option<PathBuf>),
    /// How an export went, for the page to say.
    Exported(Result<String, String>),
    /// How an import went, for the page to say.
    Imported(Result<String, String>),
    Nothing,
}

#[zbus::proxy(
    interface = "org.macshell.Glass",
    default_service = "org.kde.KWin",
    default_path = "/org/macshell/Glass"
)]
trait Glass {
    #[zbus(name = "setting")]
    fn setting(&self, key: &str) -> zbus::Result<f64>;
    #[zbus(name = "setSetting")]
    fn set_setting(&self, key: &str, value: f64) -> zbus::Result<()>;
    #[zbus(name = "resetSettings")]
    fn reset_settings(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.macshell.Windows",
    default_service = "org.macshell.Shell",
    default_path = "/org/macshell/Windows"
)]
trait Shell {
    #[zbus(name = "ConfigChanged")]
    fn config_changed(&self) -> zbus::Result<()>;
}

async fn read_glass() -> Option<HashMap<&'static str, f32>> {
    let connection = dbus::session().await?;
    let proxy = GlassProxy::new(&connection).await.ok()?;
    let mut values = HashMap::new();
    for key in GLASS_SLIDERS
        .iter()
        .map(|slider| slider.key)
        .chain([GREEN_FULL_SCREEN])
    {
        match proxy.setting(key).await {
            Ok(value) => {
                values.insert(key, value as f32);
            }
            Err(err) => {
                log::warn!("can't read the glass settings: {err}");
                return None;
            }
        }
    }
    Some(values)
}

async fn write_glass(key: &'static str, value: f32) {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        GlassProxy::new(&connection)
            .await?
            // By way of text, or 0.02 would be saved as 0.019999999552965164.
            .set_setting(key, value.to_string().parse().unwrap_or(f64::from(value)))
            .await
    }
    .await;
    if let Err(err) = result {
        log::warn!("can't change the glass: {err}");
    }
}

async fn reset_glass() {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        GlassProxy::new(&connection).await?.reset_settings().await
    }
    .await;
    if let Err(err) = result {
        log::warn!("can't reset the glass: {err}");
    }
}

/// The settings as they are now, ready to be written to a file.
async fn current_settings() -> Result<SettingsFile, String> {
    let glass: BTreeMap<String, f32> = read_glass()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    let config = config::read_text().map_err(|err| format!("Can't read your settings: {err}"))?;
    SettingsFile::new(glass, &config)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

async fn export_settings(path: PathBuf) -> Result<String, String> {
    let text = current_settings().await?.to_text()?;
    fs::write(&path, text).map_err(|err| format!("Can't save {}: {err}", file_name(&path)))?;
    Ok(format!("Exported to {}.", file_name(&path)))
}

/// Applies a settings file, keeping what it replaces. Nothing changes unless the file's good and
/// what it replaces is safe.
async fn import_settings(path: PathBuf) -> Result<String, String> {
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("Can't read {}: {err}", file_name(&path)))?;
    let file = SettingsFile::parse(&text)?;
    let config = file.config_text()?;

    let backup = config::sibling(BACKUP_NAME)
        .ok_or_else(|| String::from("Can't find where to keep your current settings."))?;
    let current = current_settings().await?.to_text()?;
    if let Some(dir) = backup.parent() {
        fs::create_dir_all(dir)
            .map_err(|err| format!("Can't keep your current settings: {err}"))?;
    }
    fs::write(&backup, current)
        .map_err(|err| format!("Can't keep your current settings: {err}"))?;

    config::write_text(&config).map_err(|err| format!("Can't save the settings: {err}"))?;
    reset_glass().await;
    for setting in &GLASS_SLIDERS {
        if let Some(value) = file
            .glass
            .get(setting.key)
            .filter(|value| value.is_finite())
        {
            write_glass(setting.key, value.clamp(setting.range.0, setting.range.1)).await;
        }
    }
    let full_screen = file
        .glass
        .get(GREEN_FULL_SCREEN)
        .is_some_and(|value| *value != 0.0);
    write_glass(GREEN_FULL_SCREEN, if full_screen { 1.0 } else { 0.0 }).await;
    tell_shell().await;
    Ok(format!(
        "Imported {}. To undo it, import {BACKUP_NAME} from macshell's config folder.",
        file_name(&path)
    ))
}

/// A small picture of the wallpaper, on a thread of its own. A wallpaper can be huge, and only needs
/// to fill a thumbnail here.
async fn read_wallpaper_picture(configured: Option<String>) -> Option<image::Handle> {
    let path = wallpaper::resolve(&configured?)?;
    tokio::task::spawn_blocking(move || {
        let picture = ::image::open(&path)
            .inspect_err(|err| log::warn!("can't read the wallpaper: {err}"))
            .ok()?
            .thumbnail(384, 216)
            .to_rgba8();
        let (width, height) = picture.dimensions();
        Some(image::Handle::from_rgba(width, height, picture.into_raw()))
    })
    .await
    .ok()
    .flatten()
}

/// Tells the shell to read the config file again. It's fine if it isn't running.
async fn tell_shell() {
    let result = async {
        let connection = dbus::session().await.ok_or(zbus::Error::Unsupported)?;
        ShellProxy::new(&connection).await?.config_changed().await
    }
    .await;
    if let Err(err) = result {
        log::debug!("can't tell the shell about a change: {err}");
    }
}

/// What the About page shows, which doesn't change while it's open.
struct About {
    host: String,
    system: String,
    kernel: String,
    processor: String,
    memory: String,
}

impl About {
    fn read() -> Self {
        let read_trimmed = |path: &str| {
            fs::read_to_string(path)
                .map(|contents| contents.trim().to_owned())
                .unwrap_or_default()
        };
        let os_release = read_trimmed("/etc/os-release");
        let system = os_release
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map_or_else(String::new, |name| name.trim_matches('"').to_owned());
        let cpu_info = read_trimmed("/proc/cpuinfo");
        let processor = cpu_info
            .lines()
            .find_map(|line| line.strip_prefix("model name"))
            .and_then(|line| line.split_once(':'))
            .map_or_else(String::new, |(_, name)| name.trim().to_owned());
        let mem_info = read_trimmed("/proc/meminfo");
        let memory = mem_info
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|kilobytes| kilobytes.parse::<f64>().ok())
            .map_or_else(String::new, |kilobytes| {
                format!("{:.0} GB", kilobytes / 1024.0 / 1024.0)
            });
        Self {
            host: read_trimmed("/proc/sys/kernel/hostname"),
            system,
            kernel: read_trimmed("/proc/sys/kernel/osrelease"),
            processor,
            memory,
        }
    }
}

struct SettingsApp {
    id: IcedId,
    material: Option<Rc<Material>>,
    surface: Option<MaterialSurface>,
    page: Page,
    lights_hovered: bool,
    glass: Option<HashMap<&'static str, f32>>,
    magnification: bool,
    magnified_size: f32,
    always_glassy: bool,
    wallpaper: Option<String>,
    wallpaper_picture: Option<image::Handle>,
    about: About,
    /// How the last export or import went.
    transfer_status: Option<String>,
}

pub fn run() -> Result<(), iced_layershell::Error> {
    // One at a time. A second one has nothing to do: the dock and the menu bar bring back the
    // window of the first, and it can't raise a window itself without a token from them.
    let Some(name) = claim_name() else {
        log::info!("the Settings app is already running");
        return Ok(());
    };
    // Held until the app quits.
    std::mem::forget(name);

    let connection = Connection::connect_to_env()
        .inspect_err(|err| log::error!("can't connect to Wayland: {err}"))
        .ok();
    let material = connection.as_ref().and_then(Material::new).map(Rc::new);

    daemon(
        move || SettingsApp::new(material.clone()),
        || String::from(APP_ID),
        SettingsApp::update,
        SettingsApp::view,
    )
    .style(|_: &SettingsApp, _: &Theme| theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    })
    .title(|_: &SettingsApp, _: IcedId| Some(String::from("System Settings")))
    .subscription(SettingsApp::subscription)
    .settings(Settings {
        layer_settings: LayerShellSettings {
            start_mode: StartMode::Background,
            ..Default::default()
        },
        default_font: fonts::REGULAR,
        default_text_size: TEXT_SIZE.into(),
        with_connection: connection.map(WithConnection::Value),
        ..Default::default()
    })
    .run()
}

/// Claims our name on the bus, which fails if it's taken.
fn claim_name() -> Option<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.replace_existing_names(false).name(BUS_NAME))
        .and_then(|builder| builder.build())
        .inspect_err(|err| log::debug!("can't claim {BUS_NAME}: {err}"))
        .ok()
}

impl SettingsApp {
    fn new(material: Option<Rc<Material>>) -> (Self, Task<Message>) {
        let config = Config::load();
        let (id, open) = Message::base_window_open(IcedXdgWindowSettings {
            size: Some((WIDTH as u32, HEIGHT as u32)),
            client_side_decorations: true,
            app_id: Some(APP_ID),
            title: Some("System Settings"),
            fixed_size: true,
        });
        let read_picture = Task::perform(
            read_wallpaper_picture(config.desktop.wallpaper.clone()),
            Message::WallpaperPictureRead,
        );
        let settings = Self {
            id,
            material,
            surface: None,
            page: Page::Glass,
            lights_hovered: false,
            glass: None,
            magnification: config.dock.magnification,
            magnified_size: config.dock.magnified_size,
            always_glassy: config.desktop.always_glassy_widgets,
            wallpaper: config.desktop.wallpaper,
            wallpaper_picture: None,
            about: About::read(),
            transfer_status: None,
        };
        let read = Task::perform(read_glass(), Message::GlassRead);
        (settings, Task::batch([open, read, read_picture]))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // It's a small window, and most of what happens here changes it.
        iced_layershell::redraw::mark_changed(self.id);
        match message {
            Message::Opened(id) if id == self.id && self.surface.is_none() => {
                if let Some(material) = &self.material {
                    let connection = material.connection().clone();
                    return iced_runtime::window::run(id, move |window| {
                        material::surface_of(&connection, window)
                    })
                    .map(Message::Reached);
                }
            }
            Message::Reached(Some(surface)) => {
                if let Some(material) = &self.material {
                    let target = material.attach(surface);
                    material.set_appear(
                        &target,
                        material::Appear::Gentle((WIDTH / 2.0) as i32, (HEIGHT / 2.0) as i32),
                    );
                    material.set_window_shadow(&target, RADIUS, true);
                    material.set_shapes(&target, &material_shapes());
                    self.surface = Some(target);
                }
            }
            Message::Closed | Message::Close => return iced_runtime::exit(),
            Message::Dragged => return iced_runtime::window::drag(self.id),
            Message::Minimize => return iced_runtime::window::minimize(self.id, true),
            Message::LightsHovered(hovered) => self.lights_hovered = hovered,
            Message::ShowPage(page) => self.page = page,
            Message::OpenSystemPage(page) => {
                apps::spawn(vec![String::from("kcmshell6"), page.to_owned()]);
            }
            Message::GlassRead(values) => self.glass = values,
            Message::GlassChanged(key, value) => {
                if let Some(glass) = &mut self.glass {
                    glass.insert(key, value);
                }
                return Task::future(write_glass(key, value)).discard();
            }
            // Changes on their way could arrive in any order, so the last one is sent again.
            Message::GlassReleased(key) => {
                if let Some(value) = self.glass.as_ref().and_then(|glass| glass.get(key)) {
                    return Task::future(write_glass(key, *value)).discard();
                }
            }
            Message::GlassReset => {
                return Task::future(async {
                    reset_glass().await;
                    read_glass().await
                })
                .map(Message::GlassRead);
            }
            Message::MagnificationToggled(on) => {
                self.magnification = on;
                if let Err(err) = config::save_magnification(on) {
                    log::warn!("can't save the dock's magnification: {err}");
                }
                return Task::future(tell_shell()).discard();
            }
            Message::MagnifiedSizeChanged(size) => self.magnified_size = size,
            Message::MagnifiedSizeReleased => {
                if let Err(err) = config::save_magnified_size(self.magnified_size) {
                    log::warn!("can't save the dock's magnified size: {err}");
                }
                return Task::future(tell_shell()).discard();
            }
            Message::AlwaysGlassyToggled(on) => {
                self.always_glassy = on;
                if let Err(err) = config::save_always_glassy_widgets(on) {
                    log::warn!("can't save the widgets' look: {err}");
                }
                return Task::future(tell_shell()).discard();
            }
            Message::ChooseWallpaper => {
                return Task::perform(portal::choose_image(), Message::WallpaperChosen);
            }
            Message::WallpaperChosen(Some(path)) => {
                if let Err(err) = config::save_wallpaper(&path) {
                    log::warn!("can't save the wallpaper: {err}");
                }
                let configured = path.to_string_lossy().into_owned();
                self.wallpaper = Some(configured.clone());
                return Task::batch([
                    Task::future(tell_shell()).discard(),
                    Task::perform(
                        read_wallpaper_picture(Some(configured)),
                        Message::WallpaperPictureRead,
                    ),
                ]);
            }
            Message::WallpaperPictureRead(picture) => self.wallpaper_picture = picture,
            Message::GreenFullScreenChosen(full_screen) => {
                let value = if full_screen { 1.0 } else { 0.0 };
                if let Some(glass) = &mut self.glass {
                    glass.insert(GREEN_FULL_SCREEN, value);
                }
                return Task::future(write_glass(GREEN_FULL_SCREEN, value)).discard();
            }
            Message::ExportSettings => {
                return Task::perform(
                    portal::choose_settings_destination("macshell-settings.toml"),
                    Message::ExportTo,
                );
            }
            Message::ExportTo(Some(path)) => {
                return Task::perform(export_settings(path), Message::Exported);
            }
            Message::ImportSettings => {
                return Task::perform(portal::choose_settings_file(), Message::ImportFrom);
            }
            Message::ImportFrom(Some(path)) => {
                return Task::perform(import_settings(path), Message::Imported);
            }
            Message::Exported(result) => {
                self.transfer_status = Some(result.unwrap_or_else(|err| err));
            }
            Message::Imported(result) => {
                let imported = result.is_ok();
                self.transfer_status = Some(result.unwrap_or_else(|err| err));
                if imported {
                    let config = Config::load();
                    self.magnification = config.dock.magnification;
                    self.magnified_size = config.dock.magnified_size;
                    self.always_glassy = config.desktop.always_glassy_widgets;
                    self.wallpaper.clone_from(&config.desktop.wallpaper);
                    return Task::batch([
                        Task::perform(read_glass(), Message::GlassRead),
                        Task::perform(
                            read_wallpaper_picture(config.desktop.wallpaper),
                            Message::WallpaperPictureRead,
                        ),
                    ]);
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced_futures::event::listen_with(|event, _, id| match event {
            Event::Window(window::Event::Opened { .. }) => Some(Message::Opened(id)),
            Event::Window(window::Event::Closed | window::Event::CloseRequested) => {
                Some(Message::Closed)
            }
            _ => None,
        })
    }

    fn view(&self, _: IcedId) -> Element<'_> {
        let sidebar = container(
            column![
                // As far from the sidebar's left edge as from its top, and in line with its icons.
                container(lights(self.lights_hovered)).padding(Padding {
                    left: 6.0,
                    ..Default::default()
                }),
                Space::new().height(16),
                sidebar_item(Page::Glass, self.page, Glyph::Drop, rgb(58, 132, 246)),
                sidebar_item(Page::Desktop, self.page, Glyph::Photo, rgb(160, 92, 214)),
                sidebar_item(Page::Windows, self.page, Glyph::Window, rgb(112, 112, 122)),
                Space::new().height(12),
                text("System").size(CAPTION_SIZE).color(DIM),
                Space::new().height(4),
                system_item(
                    "Wi-Fi",
                    "kcm_networkmanagement",
                    Glyph::Wifi,
                    rgb(58, 132, 246)
                ),
                system_item(
                    "Bluetooth",
                    "kcm_bluetooth",
                    Glyph::Bluetooth,
                    rgb(58, 132, 246)
                ),
                system_item(
                    "Sound",
                    "kcm_pulseaudio",
                    Glyph::SpeakerHigh,
                    rgb(232, 74, 118)
                ),
                system_item("Displays", "kcm_kscreen", Glyph::SunHigh, rgb(58, 132, 246)),
                system_item(
                    "Keyboard",
                    "kcm_keyboard",
                    Glyph::Keyboard,
                    rgb(112, 112, 122)
                ),
                Space::new().height(Length::Fill),
                sidebar_item(Page::About, self.page, Glyph::Info, rgb(112, 112, 122)),
            ]
            .spacing(2),
        )
        .padding(Padding {
            top: 14.0,
            right: 8.0,
            bottom: 8.0,
            left: 8.0,
        })
        .width(SIDEBAR_WIDTH)
        .height(Length::Fill);

        let page: Element<'_> = match self.page {
            Page::Glass => self.glass_page(),
            Page::Desktop => self.desktop_page(),
            Page::Windows => self.windows_page(),
            Page::About => self.about_page(),
        };
        let content = column![
            container(text(self.page.title()).size(TITLE_SIZE).font(fonts::BOLD))
                .height(50)
                .padding(Padding {
                    left: 20.0,
                    ..Default::default()
                })
                .align_y(Alignment::Center),
            scrollable(container(page).padding(Padding {
                left: 20.0,
                right: 20.0,
                bottom: 20.0,
                top: 0.0,
            }))
            .direction(Direction::Vertical(
                Scrollbar::new().width(0).scroller_width(0),
            ))
            .height(Length::Fill)
            .width(Length::Fill),
        ]
        .width(Length::Fill);

        // Controls take their own presses, so this gets the rest.
        mouse_area(
            container(
                row![container(sidebar).padding(SIDEBAR_INSET), content]
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                text_color: Some(Color::WHITE),
                ..Default::default()
            }),
        )
        .on_press(Message::Dragged)
        .into()
    }

    fn glass_page(&self) -> Element<'_> {
        let Some(glass) = &self.glass else {
            return caption(
                "These settings need macshell's KWin plugin, which doesn't seem to be running.",
            );
        };
        let rows = GLASS_SLIDERS
            .iter()
            .map(|setting| {
                let value = glass.get(setting.key).copied().unwrap_or(setting.range.0);
                let key = setting.key;
                setting_row(
                    setting.label,
                    settings_slider(
                        setting.range,
                        value,
                        setting.step,
                        move |value| Message::GlassChanged(key, value),
                        Message::GlassReleased(key),
                    ),
                )
            })
            .collect();
        column![
            caption(
                "Everything made of glass is one material: a slab whose edge curves down, \
                 bending what's behind it. Changes show right away, on this window's sidebar too.",
            ),
            card(rows),
            row![
                Space::new().width(Length::Fill),
                capsule_button("Reset to Defaults", Message::GlassReset),
            ],
        ]
        .spacing(12)
        .into()
    }

    fn desktop_page(&self) -> Element<'_> {
        let picture: Element<'_> = match &self.wallpaper_picture {
            Some(handle) => image(handle.clone())
                .width(96)
                .height(54)
                .content_fit(ContentFit::Cover)
                .border_radius(8.0)
                .into(),
            None => container(icons::icon(Glyph::Photo, 22.0, DIM))
                .center_x(96)
                .center_y(54)
                .style(|_| container::Style {
                    background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08))),
                    border: Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
        };
        let file_name = self
            .wallpaper
            .as_deref()
            .and_then(|path| path.rsplit('/').next())
            .unwrap_or("None");

        let mut dock = vec![setting_row(
            "Magnification",
            settings_toggle(self.magnification, Message::MagnificationToggled),
        )];
        if self.magnification {
            dock.push(setting_row(
                "Magnified Size",
                settings_slider(
                    (crate::dock::ICON_SIZE, 128.0),
                    self.magnified_size,
                    1.0,
                    Message::MagnifiedSizeChanged,
                    Message::MagnifiedSizeReleased,
                ),
            ));
        }

        column![
            section(
                "Wallpaper",
                card(vec![
                    row![
                        picture,
                        container(
                            text(file_name)
                                .size(TEXT_SIZE)
                                .wrapping(iced_widget::core::text::Wrapping::None),
                        )
                        .width(Length::Fill)
                        .clip(true),
                        capsule_button("Choose…", Message::ChooseWallpaper),
                    ]
                    .spacing(12)
                    .padding(10)
                    .align_y(Alignment::Center)
                    .into(),
                ]),
            ),
            section("Dock", card(dock)),
            section(
                "Widgets",
                column![
                    card(vec![setting_row(
                        "Always Glassy",
                        settings_toggle(self.always_glassy, Message::AlwaysGlassyToggled),
                    )]),
                    caption(
                        "Without this, widgets turn to glass while a window is active, and come \
                         back in color when the desktop is.",
                    ),
                ]
                .spacing(6),
            ),
        ]
        .spacing(18)
        .into()
    }

    fn windows_page(&self) -> Element<'_> {
        let Some(glass) = &self.glass else {
            return caption(
                "These settings need macshell's KWin plugin, which doesn't seem to be running.",
            );
        };
        let full_screen = glass
            .get(GREEN_FULL_SCREEN)
            .is_some_and(|value| *value != 0.0);
        column![section(
            "Green Button",
            column![
                card(vec![setting_row(
                    "When Clicked",
                    row![
                        choice("Maximize", !full_screen, false),
                        choice("Full Screen", full_screen, true),
                    ]
                    .spacing(2)
                    .into(),
                )]),
                caption(
                    "A full-screen window has no title bar to leave it with yet: its window \
                         menu (Alt+F3) has the way out, under More. Resting on the button still \
                         shows its menu.",
                ),
            ]
            .spacing(6),
        ),]
        .into()
    }

    fn about_page(&self) -> Element<'_> {
        let about = &self.about;
        column![
            card(vec![
                info_row("Name", &about.host),
                info_row("System", &about.system),
                info_row("Kernel", &about.kernel),
                info_row("Processor", &about.processor),
                info_row("Memory", &about.memory),
            ]),
            card(vec![info_row("macshell", env!("CARGO_PKG_VERSION"))]),
            section(
                "Settings File",
                column![
                    card(vec![
                        row![
                            text("Dock, desktop, glass and windows")
                                .size(TEXT_SIZE)
                                .width(Length::Fill),
                            capsule_button("Import…", Message::ImportSettings),
                            capsule_button("Export…", Message::ExportSettings),
                        ]
                        .spacing(8)
                        .padding([0, 12])
                        .height(44)
                        .align_y(Alignment::Center)
                        .into(),
                    ]),
                    caption(self.transfer_status.as_deref().unwrap_or(
                        "Importing replaces your current settings, and keeps them in a file so \
                             you can go back. Where widgets and icons sit isn't included.",
                    ),),
                ]
                .spacing(6),
            ),
        ]
        .spacing(12)
        .into()
    }
}

fn material_shapes() -> Vec<material::Shape> {
    vec![
        material::Shape {
            rect: (0, 0, WIDTH as i32, HEIGHT as i32),
            radius: RADIUS as i32,
            style: material::Style::Window { active: true },
        },
        material::Shape {
            rect: (
                SIDEBAR_INSET as i32,
                SIDEBAR_INSET as i32,
                SIDEBAR_WIDTH as i32,
                (HEIGHT - 2.0 * SIDEBAR_INSET) as i32,
            ),
            radius: SIDEBAR_RADIUS as i32,
            style: material::Style::Control,
        },
    ]
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::from_rgb8(red, green, blue)
}

/// The traffic lights. There's no zooming a window of a fixed size, so the green one is off.
fn lights<'a>(hovered: bool) -> Element<'a> {
    let dot = |color: Color, mark: Option<Element<'a>>, on_press: Option<Message>| {
        button(
            container(mark.unwrap_or_else(|| Space::new().into()))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(LIGHT_SIZE)
        .height(LIGHT_SIZE)
        .padding(0)
        .on_press_maybe(on_press)
        .style(move |_, _| button::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: (LIGHT_SIZE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
    };
    let ink = Color::from_rgba(0.0, 0.0, 0.0, 0.6);
    let close_mark = hovered.then(|| icons::icon(Glyph::Close, 9.0, ink));
    let minimize_mark = hovered.then(|| {
        container(Space::new())
            .width(8)
            .height(1.5)
            .style(move |_| container::Style {
                background: Some(Background::Color(ink)),
                ..Default::default()
            })
            .into()
    });
    mouse_area(
        row![
            dot(rgb(255, 92, 93), close_mark, Some(Message::Close)),
            dot(rgb(255, 195, 11), minimize_mark, Some(Message::Minimize)),
            dot(Color::from_rgba(1.0, 1.0, 1.0, 0.17), None, None),
        ]
        .spacing(LIGHT_SPACING),
    )
    .on_enter(Message::LightsHovered(true))
    .on_exit(Message::LightsHovered(false))
    .into()
}

fn sidebar_tile<'a>(glyph: Glyph, color: Color) -> Element<'a> {
    container(icons::icon(glyph, 15.0, Color::WHITE))
        .center_x(22)
        .center_y(22)
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn sidebar_row<'a>(
    label: &'a str,
    tile: Element<'a>,
    selected: bool,
    press: Message,
) -> Element<'a> {
    button(
        row![tile, text(label).size(TEXT_SIZE)]
            .spacing(8)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([4, 6])
    .on_press(press)
    .style(move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let background = if selected {
            Some(menu::ACCENT)
        } else if hovered {
            Some(Color::from_rgba(1.0, 1.0, 1.0, 0.1))
        } else {
            None
        };
        button::Style {
            background: background.map(Background::Color),
            text_color: Color::WHITE,
            border: Border {
                radius: 10.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
}

fn sidebar_item<'a>(page: Page, current: Page, glyph: Glyph, color: Color) -> Element<'a> {
    sidebar_row(
        page.title(),
        sidebar_tile(glyph, color),
        page == current,
        Message::ShowPage(page),
    )
}

fn system_item<'a>(
    label: &'a str,
    module: &'static str,
    glyph: Glyph,
    color: Color,
) -> Element<'a> {
    sidebar_row(
        label,
        sidebar_tile(glyph, color),
        false,
        Message::OpenSystemPage(module),
    )
}

fn caption<'a>(content: &'a str) -> Element<'a> {
    text(content)
        .size(CAPTION_SIZE)
        .line_height(iced_widget::core::text::LineHeight::Absolute(15.0.into()))
        .color(DIM)
        .into()
}

/// A group of rows on one rounded background, with lines between them.
fn card<'a>(rows: Vec<Element<'a>>) -> Element<'a> {
    let count = rows.len();
    let mut items = Vec::with_capacity(count * 2);
    for (index, item) in rows.into_iter().enumerate() {
        items.push(item);
        if index + 1 < count {
            items.push(
                container(
                    container(Space::new())
                        .width(Length::Fill)
                        .height(1)
                        .style(|_| container::Style {
                            background: Some(Background::Color(Color::from_rgba(
                                1.0, 1.0, 1.0, 0.08,
                            ))),
                            ..Default::default()
                        }),
                )
                .padding([0, 12])
                .into(),
            );
        }
    }
    container(column(items))
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.07))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.07),
                width: 1.0,
                radius: CARD_RADIUS.into(),
            },
            ..Default::default()
        })
        .into()
}

fn section<'a>(title: &'a str, content: impl Into<Element<'a>>) -> Element<'a> {
    column![
        container(text(title).size(TEXT_SIZE).font(fonts::BOLD)).padding(Padding {
            left: 4.0,
            ..Default::default()
        }),
        content.into(),
    ]
    .spacing(6)
    .into()
}

/// A setting's name, with what changes it on the right.
fn setting_row<'a>(label: &'a str, control: Element<'a>) -> Element<'a> {
    row![
        text(label).size(TEXT_SIZE).width(112),
        container(control)
            .width(Length::Fill)
            .align_x(Alignment::End),
    ]
    .padding([0, 12])
    .height(40)
    .align_y(Alignment::Center)
    .into()
}

fn info_row<'a>(label: &'a str, value: &'a str) -> Element<'a> {
    row![
        text(label).size(TEXT_SIZE).width(Length::Fill),
        text(value).size(TEXT_SIZE).color(DIM),
    ]
    .padding([0, 12])
    .height(36)
    .align_y(Alignment::Center)
    .into()
}

fn settings_slider<'a>(
    range: (f32, f32),
    value: f32,
    step: f32,
    on_change: impl Fn(f32) -> Message + 'a,
    on_release: Message,
) -> Element<'a> {
    slider(range.0..=range.1, value.clamp(range.0, range.1), on_change)
        .on_release(on_release)
        .step(step)
        .width(Length::Fill)
        .height(18.0)
        .style(|_, _| slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    Background::Color(Color::WHITE),
                    Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.2)),
                ),
                width: 4.0,
                border: Border {
                    radius: 2.0.into(),
                    ..Default::default()
                },
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 8.0 },
                background: Background::Color(Color::WHITE),
                border_width: 0.5,
                border_color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
            },
        })
        .into()
}

fn settings_toggle<'a>(value: bool, on_toggle: impl Fn(bool) -> Message + 'a) -> Element<'a> {
    toggler(value)
        .on_toggle(on_toggle)
        .size(20.0)
        .style(move |_, _| toggler::Style {
            background: Background::Color(if value {
                menu::ACCENT
            } else {
                Color::from_rgba(1.0, 1.0, 1.0, 0.18)
            }),
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: Background::Color(Color::WHITE),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: None,
            border_radius: None,
            padding_ratio: 0.1,
        })
        .into()
}

fn capsule_button<'a>(label: &'a str, press: Message) -> Element<'a> {
    button(text(label).size(TEXT_SIZE))
        .padding([4, 12])
        .on_press(press)
        .style(|_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(Color::from_rgba(
                    1.0,
                    1.0,
                    1.0,
                    if hovered { 0.22 } else { 0.12 },
                ))),
                text_color: Color::WHITE,
                border: Border {
                    radius: 13.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}

/// One side of a choice between two things.
fn choice<'a>(label: &'a str, selected: bool, full_screen: bool) -> Element<'a> {
    button(text(label).size(TEXT_SIZE))
        .padding([4, 12])
        .on_press(Message::GreenFullScreenChosen(full_screen))
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let background = if selected {
                menu::ACCENT
            } else {
                Color::from_rgba(1.0, 1.0, 1.0, if hovered { 0.22 } else { 0.1 })
            };
            button::Style {
                background: Some(Background::Color(background)),
                text_color: Color::WHITE,
                border: Border {
                    radius: 13.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}
