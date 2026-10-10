#[rustfmt::skip]
mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

mod app;
mod app_group;
mod app_legacy;
mod icon_cache;
mod input_ownership;
mod launch_state;
mod localize;
mod providers;
mod screens;
mod settings;
mod style;
mod subscriptions;
mod system_status;
mod toplevel;
mod ui;
mod widgets;

use config::{APP_ID, VERSION};
use log::info;

use localize::localize;

// TODO watch the desktop dirs for changes and update the list of apps on change

fn main() -> cosmic::iced::Result {
    // Initialize logger
    pretty_env_logger::try_init().ok();
    info!("HearthDeck ({})", APP_ID);
    info!("Version: {}", VERSION);
    // Prepare i18n
    localize();

    // The rewritten frontend (`app`) is the default. The inherited one
    // (`app_legacy`) stays runnable behind `HEARTHDECK_LEGACY=1` until the
    // rewrite covers every screen.
    if std::env::var_os("HEARTHDECK_LEGACY").is_some() {
        app_legacy::run()
    } else {
        app::run()
    }
}
