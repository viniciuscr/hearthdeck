use cosmic::iced::{Subscription, stream};
use futures::SinkExt;
use hearthdeck_protocol::input::{
    Action, Delivery, Event, Hello, Role, VERSION, transport::Client,
};

use super::gamepad::GamepadEvent;

pub fn events(delivery: Delivery) -> Subscription<Event> {
    Subscription::run_with(delivery, |&delivery| {
        stream::channel(
            8,
            move |mut output: futures::channel::mpsc::Sender<Event>| async move {
                loop {
                    let result = async {
                        let mut client = Client::connect(Hello {
                            version: VERSION,
                            role: Role::Frontend,
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

pub fn navigation(action: Action) -> Option<GamepadEvent> {
    Some(match action {
        Action::MoveUp => GamepadEvent::MoveUp,
        Action::MoveDown => GamepadEvent::MoveDown,
        Action::MoveLeft => GamepadEvent::MoveLeft,
        Action::MoveRight => GamepadEvent::MoveRight,
        Action::Confirm => GamepadEvent::Confirm,
        Action::Back => GamepadEvent::Back,
        Action::Search => GamepadEvent::Search,
        Action::ContextMenu => GamepadEvent::ContextMenu,
        Action::PrevGroup => GamepadEvent::PrevGroup,
        Action::NextGroup => GamepadEvent::NextGroup,
        Action::PrevTab => GamepadEvent::PrevTab,
        Action::NextTab => GamepadEvent::NextTab,
        Action::FilterPanel => GamepadEvent::FilterPanel,
        Action::Keyboard => GamepadEvent::Keyboard,
        Action::ToggleOverlay => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_back_uses_the_existing_global_navigation_message() {
        assert_eq!(navigation(Action::Back), Some(GamepadEvent::Back));
        assert_eq!(navigation(Action::Keyboard), Some(GamepadEvent::Keyboard));
        assert_eq!(navigation(Action::ToggleOverlay), None);
    }
}
