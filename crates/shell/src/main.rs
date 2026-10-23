mod menubar;
mod niri;

fn main() -> Result<(), iced_layershell::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,layershellev=error,macshell=info"),
    )
    .init();
    menubar::run()
}
