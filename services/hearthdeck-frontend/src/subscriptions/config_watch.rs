//! One cosmic-config source, watched: the keys an app writes, as it writes
//! them.
//!
//! This is the live half of the shell's settings wiring. Settings stored
//! through cosmic-config are read once on the way in and then re-read whenever
//! their owner saves them again, so a change made in COSMIC Settings reaches
//! the shell while it runs instead of on the next start. Each source is one
//! subscription: it reports which keys a transaction touched, and the consumer
//! decides what that changes about itself - see `wallpaper`, the first of them.
//!
//! The stream ends if the source cannot be opened or watched at all; there is
//! nothing to re-read then, and the app keeps the value it loaded with.

use cosmic::cosmic_config::Config;
use cosmic::iced::{Subscription, stream};
use futures::{SinkExt, StreamExt};

/// Watches the `version` config of `name`, emitting the keys changed in each
/// transaction as its owner saves them. The same `name` and `version` share one
/// watcher, so several consumers of one source cost one inotify watch.
pub fn config_watch(name: &'static str, version: u64) -> Subscription<Vec<String>> {
    Subscription::run_with((name, version), |&(name, version)| {
        stream::channel(16, async move |mut output| {
            let config = match Config::new(name, version) {
                Ok(config) => config,
                Err(error) => {
                    log::warn!("config source {name} unavailable: {error:?}");
                    return;
                }
            };
            // The watcher reports through its own channel: its callback is a
            // plain `Fn`, so the sends go through the sink it holds.
            let (sender, mut changes) = futures::channel::mpsc::channel(16);
            let _watcher = match config.watch(move |_config, keys| {
                let mut sender = sender.clone();
                let _ = sender.try_send(keys.to_vec());
            }) {
                // Held for as long as this stream lives, which is what keeps
                // the underlying watch open.
                Ok(watcher) => watcher,
                Err(error) => {
                    log::warn!("config source {name} could not be watched: {error:?}");
                    return;
                }
            };
            while let Some(keys) = changes.next().await {
                if output.send(keys).await.is_err() {
                    return;
                }
            }
        })
    })
}
