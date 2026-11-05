mod alert;
mod app;
mod appmenu;
mod apps;
mod audio;
mod autostart;
mod banners;
mod bluetooth;
mod brightness;
mod config;
mod control;
mod dbus;
mod dbusmenu;
mod dock;
mod fonts;
mod glass_protocol;
mod icons;
mod kwin;
mod launcher;
mod material;
mod media;
mod menu;
mod menubar;
mod network;
mod notification_center;
mod notifications;
mod places;
mod plasma_protocol;
mod shadow_protocol;
mod stack;
mod tray;
mod wallpaper;
mod window_shadow;

fn main() -> Result<(), iced_layershell::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,layershellev=error,macshell=info"),
    )
    .init();
    fonts::load();
    app::run()
}
