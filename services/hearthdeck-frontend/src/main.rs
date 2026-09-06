#[rustfmt::skip]
mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

mod app;
mod app_group;
mod icon_cache;
mod launch_state;
mod localize;
mod providers;
mod style;
mod subscriptions;
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

    app::run()
}
