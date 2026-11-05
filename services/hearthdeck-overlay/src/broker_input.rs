use cosmic::iced::{Subscription, stream};
use futures_util::SinkExt;
use hearthdeck_protocol::input::{Delivery, Event, Hello, Role, VERSION, transport::Client};

pub(crate) fn subscription(capture: bool) -> Subscription<Event> {
    Subscription::run_with(capture, |&capture| {
        stream::channel(
            8,
            move |mut output: futures_channel::mpsc::Sender<Event>| async move {
                let mut delivery = if capture {
                    Delivery::Overlay
                } else {
                    Delivery::Observe
                };
                loop {
                    let result = async {
                        let mut client = Client::connect(Hello {
                            version: VERSION,
                            role: Role::Overlay,
                            delivery,
                        })
                        .await?;
                        loop {
                            let event = client.recv().await?;
                            if output.send(event).await.is_err() {
                                return Ok::<(), std::io::Error>(());
                            }
                        }
                    }
                    .await;
                    let Err(error) = result else { return };
                    delivery = Delivery::Observe;
                    if output
                        .send(Event::Failed(format!(
                            "Controller broker unavailable: {error}"
                        )))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            },
        )
    })
}
