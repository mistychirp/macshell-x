mod alert;
mod app;
mod apps;
mod audio;
mod bluetooth;
mod config;
mod control;
mod dbus;
mod dock;
mod glass_protocol;
mod icons;
mod material;
mod media;
mod menu;
mod menubar;
mod network;
mod niri;
mod places;
mod stack;

fn main() -> Result<(), iced_layershell::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,layershellev=error,macshell=info"),
    )
    .init();
    app::run()
}
