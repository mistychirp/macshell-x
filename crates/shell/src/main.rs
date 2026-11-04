mod alert;
mod app;
mod apps;
mod audio;
mod banners;
mod bluetooth;
mod brightness;
mod config;
mod control;
mod dbus;
mod dock;
mod glass_protocol;
mod icons;
mod kwin;
mod material;
mod media;
mod menu;
mod menubar;
mod network;
mod notification_center;
mod notifications;
mod places;
mod plasma_protocol;
mod stack;
mod wallpaper;

fn main() -> Result<(), iced_layershell::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,layershellev=error,macshell=info"),
    )
    .init();
    app::run()
}
