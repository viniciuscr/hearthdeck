//! Versioned, user-local controller delivery. Capture belongs to a connection.
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 1;
pub const MAX_FRAME: usize = 1024;
pub const QUEUE: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum Action {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Confirm,
    Back,
    Search,
    ContextMenu,
    PrevGroup,
    NextGroup,
    PrevTab,
    NextTab,
    FilterPanel,
    Keyboard,
    ToggleOverlay,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum Role {
    Frontend,
    Overlay,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum Delivery {
    Observe,
    Handoff,
    Frontend,
    TextEntry,
    Overlay,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct Hello {
    pub version: u16,
    pub role: Role,
    pub delivery: Delivery,
}

impl Hello {
    #[must_use]
    pub fn valid(self) -> bool {
        self.version == VERSION
            && matches!(
                (self.role, self.delivery),
                (
                    Role::Frontend,
                    Delivery::Observe
                        | Delivery::Handoff
                        | Delivery::Frontend
                        | Delivery::TextEntry
                ) | (Role::Overlay, Delivery::Observe | Delivery::Overlay)
            )
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionContext {
    pub mode: crate::InputMode,
    pub app_ids: Vec<String>,
}

impl SessionContext {
    #[must_use]
    pub fn matches_focus(&self, focused: Option<&str>) -> bool {
        focused.is_some_and(|focused| self.app_ids.iter().any(|id| id == focused))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Event {
    Ready,
    Action(Action),
    Failed(String),
}

#[cfg(unix)]
pub mod transport {
    use std::{io, path::PathBuf, time::Duration};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use tokio::net::UnixStream;

    use super::{Event, Hello, MAX_FRAME};

    #[must_use]
    pub fn socket_path() -> Option<PathBuf> {
        crate::paths::input_socket_path()
            .map(|path| path.with_file_name("hearthdeck-input-v1.sock"))
    }

    pub async fn read<T: serde::de::DeserializeOwned>(
        reader: &mut (impl AsyncRead + Unpin),
    ) -> io::Result<T> {
        let mut bytes = Vec::new();
        loop {
            let byte = reader.read_u8().await?;
            if byte == b'\n' {
                return serde_json::from_slice(&bytes).map_err(io::Error::other);
            }
            bytes.push(byte);
            if bytes.len() > MAX_FRAME {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "input frame too large",
                ));
            }
        }
    }

    pub async fn write<T: serde::Serialize>(
        writer: &mut (impl AsyncWrite + Unpin),
        value: &T,
    ) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        if bytes.len() > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "input frame too large",
            ));
        }
        bytes.push(b'\n');
        tokio::time::timeout(Duration::from_secs(2), writer.write_all(&bytes))
            .await
            .map_err(io::Error::other)?
    }

    pub struct Client(UnixStream);

    impl Client {
        pub async fn connect(hello: Hello) -> io::Result<Self> {
            let path =
                socket_path().ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR is not set"))?;
            let mut socket = UnixStream::connect(path).await?;
            write(&mut socket, &hello).await?;
            Ok(Self(socket))
        }

        pub async fn recv(&mut self) -> io::Result<Event> {
            read(&mut self.0).await
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn session_and_services_share_keyboard_adapter_configuration() {
            for unit in [
                include_str!("../../../deploy/systemd/hearthdeck-input.service"),
                include_str!("../../../deploy/systemd/hearthdeck-bridge.service"),
                include_str!("../../../packaging/arch/hearthdeck-input.service"),
                include_str!("../../../packaging/arch/hearthdeck-bridge.service"),
                include_str!("../../../packaging/arch/hearthdeck-overlay.service"),
            ] {
                assert!(unit.contains("EnvironmentFile=-%h/.config/hearthdeck/input.env"));
                assert!(!unit.contains("HEARTHDECK_INPUT_MODE"));
            }
            assert!(
                include_str!("../../../packaging/arch/hearthdeck-session")
                    .contains("HEARTHDECK_GAMEPAD_OSK_ADAPTER")
            );
            assert!(
                include_str!("../../../packaging/arch/input.env.example")
                    .contains("HEARTHDECK_GAMEPAD_OSK_ADAPTER")
            );
            for config in [
                include_str!("../../../packaging/arch/hearthdeck-session"),
                include_str!("../../../packaging/arch/input.env.example"),
            ] {
                assert!(!config.contains("HEARTHDECK_INPUT_MODE"));
            }
        }

        #[tokio::test]
        async fn frames_keep_boundaries_and_reject_oversized_input() {
            let (mut tx, mut rx) = tokio::io::duplex(MAX_FRAME * 3);
            write(&mut tx, &Event::Ready).await.unwrap();
            write(&mut tx, &Event::Action(super::super::Action::Back))
                .await
                .unwrap();
            assert_eq!(read::<Event>(&mut rx).await.unwrap(), Event::Ready);
            assert_eq!(
                read::<Event>(&mut rx).await.unwrap(),
                Event::Action(super::super::Action::Back)
            );
            tx.write_all(&vec![b'x'; MAX_FRAME + 1]).await.unwrap();
            assert_eq!(
                read::<Event>(&mut rx).await.unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_cannot_request_another_surfaces_delivery() {
        assert!(
            !Hello {
                version: VERSION,
                role: Role::Frontend,
                delivery: Delivery::Overlay
            }
            .valid()
        );
        assert!(
            !Hello {
                version: 0,
                role: Role::Overlay,
                delivery: Delivery::Observe
            }
            .valid()
        );
        let hello = Hello {
            version: VERSION,
            role: Role::Overlay,
            delivery: Delivery::Overlay,
        };
        assert!(hello.valid());
        assert_eq!(
            serde_json::from_slice::<Hello>(&serde_json::to_vec(&hello).unwrap()).unwrap(),
            hello
        );
    }

    #[test]
    fn compatibility_requires_an_exact_known_session_focus() {
        let session = SessionContext {
            mode: crate::InputMode {
                profile: crate::InputProfile::Desktop,
                retro_osk: false,
            },
            app_ids: vec!["org.mozilla.firefox".into()],
        };
        assert!(session.matches_focus(Some("org.mozilla.firefox")));
        assert!(!session.matches_focus(None));
        assert!(!session.matches_focus(Some("unrelated")));
        assert!(!session.matches_focus(Some("org.mozilla.firefox.popup")));
    }
}
