//! The daemon's change feed: what it says changed, so the app re-reads only that.

use std::hash::{Hash, Hasher};
use std::time::Duration;

use cosmic::iced::{Subscription, stream};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as Frame;

use crate::providers::daemon::{DaemonClient, DaemonEvent};

const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedEvent {
    Changed(DaemonEvent),
    /// The feed came back after a drop: whatever it said meanwhile was missed.
    Resumed,
}

struct Feed(DaemonClient);

impl Hash for Feed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.base_url().hash(state);
    }
}

pub fn daemon_events(client: DaemonClient) -> Subscription<FeedEvent> {
    Subscription::run_with(Feed(client), |feed| {
        let client = feed.0.clone();
        stream::channel(16, async move |mut output| {
            let mut retry = FIRST_RETRY;
            let mut connected_before = false;
            loop {
                match client.connect_events().await {
                    Ok(mut socket) => {
                        retry = FIRST_RETRY;
                        if connected_before && output.send(FeedEvent::Resumed).await.is_err() {
                            return;
                        }
                        connected_before = true;
                        while let Some(frame) = socket.next().await {
                            let text = match frame {
                                Ok(Frame::Text(text)) => text,
                                Ok(Frame::Close(_)) => break,
                                Ok(_) => continue,
                                Err(error) => {
                                    log::warn!("daemon event feed dropped: {error}");
                                    break;
                                }
                            };
                            match serde_json::from_str::<DaemonEvent>(&text) {
                                Ok(event) => {
                                    if output.send(FeedEvent::Changed(event)).await.is_err() {
                                        return;
                                    }
                                }
                                Err(error) => log::warn!("unreadable daemon event: {error}"),
                            }
                        }
                    }
                    Err(error) => log::warn!("daemon event feed unavailable: {error}"),
                }
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(LAST_RETRY);
            }
        })
    })
}
