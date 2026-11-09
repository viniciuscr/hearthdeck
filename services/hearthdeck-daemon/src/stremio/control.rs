use serde::Serialize;
use tracing::info;

use super::{LinkOffer, LinkPoll, LinkState, SOURCE_ID, StremioConnection, StremioRepository};
use crate::catalog::CatalogStore;

pub(crate) struct Control<'a> {
    pub session: &'a StremioRepository,
    pub link: &'a LinkState,
    pub catalog: &'a CatalogStore,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlError {
    #[error("{0}")]
    Link(#[source] anyhow::Error),
    #[error("could not read or clear Stremio connection")]
    Storage(#[source] anyhow::Error),
    #[error("could not clear Stremio catalog")]
    Catalog(#[from] sqlx::Error),
}

/// The pending link's credential-free view.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum LinkStatus {
    Idle,
    Expired,
    Waiting {
        code: String,
        link: String,
        qr: String,
        seconds_left: u64,
    },
    Linked {
        connection: StremioConnection,
    },
}

impl Control<'_> {
    pub(crate) async fn settings(&self) -> Result<Option<StremioConnection>, ControlError> {
        self.session
            .connection()
            .await
            .map_err(ControlError::Storage)
    }

    pub(crate) async fn start_link(&self) -> Result<LinkOffer, ControlError> {
        let offer = super::request_link().await.map_err(ControlError::Link)?;
        self.link.start(offer.clone()).await;
        info!("stremio link started");
        Ok(offer)
    }

    pub(crate) async fn poll_link(&self) -> Result<LinkStatus, ControlError> {
        let poll = self.link.poll().await.map_err(ControlError::Link)?;
        self.complete_poll(poll).await
    }

    async fn complete_poll(&self, poll: LinkPoll) -> Result<LinkStatus, ControlError> {
        let status = match poll {
            LinkPoll::Idle => LinkStatus::Idle,
            LinkPoll::Expired => LinkStatus::Expired,
            LinkPoll::Waiting(offer, seconds_left) => LinkStatus::Waiting {
                code: offer.code,
                link: offer.link,
                qr: offer.qr,
                seconds_left,
            },
            LinkPoll::Linked(auth_key) => {
                // Link completion retains its existing gateway-error response on save failure.
                let connection = self
                    .session
                    .save_session(&auth_key)
                    .await
                    .map_err(ControlError::Link)?;
                info!("stremio account linked");
                LinkStatus::Linked { connection }
            }
        };
        Ok(status)
    }

    pub(crate) async fn clear(&self) -> Result<(), ControlError> {
        self.link.cancel().await;
        self.session.clear().await.map_err(ControlError::Storage)?;
        self.catalog.replace_source(SOURCE_ID, Vec::new()).await?;
        // The arrivals were confirmed against this account's addons, so they are
        // as account-owned as the library rows and must not outlive the session.
        self.catalog
            .replace_source(crate::discovery::providers::arrivals::SOURCE_ID, Vec::new())
            .await?;
        info!("stremio account unlinked");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use serde_json::json;
    use tempfile::{TempDir, tempdir};

    use super::*;
    use crate::{database::Database, stremio::PendingLink};

    async fn storage() -> (TempDir, Database, StremioRepository, CatalogStore) {
        let directory = tempdir().unwrap();
        let database = Database::connect(&directory.path().join("hearthdeck.db"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let session = StremioRepository::new(database.pool().clone());
        let catalog = CatalogStore::new(database.pool().clone());
        (directory, database, session, catalog)
    }

    fn offer() -> LinkOffer {
        LinkOffer {
            code: "ABCD".to_owned(),
            link: "https://link.stremio.com/ABCD".to_owned(),
            qr: "https://link.stremio.com/qr/ABCD".to_owned(),
            expires_in_seconds: 300,
        }
    }

    async fn seed_sources(database: &Database) {
        for source in [SOURCE_ID, "stremio-arrivals", "desktop-apps"] {
            sqlx::query(
                "INSERT INTO library_items \
                 (id, source_id, title, kind, metadata_json, updated_at) \
                 VALUES (?, ?, ?, 'application', '{}', '2026-01-01')",
            )
            .bind(source)
            .bind(source)
            .bind(source)
            .execute(database.pool())
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn idle_expired_and_waiting_keep_their_wire_shapes() {
        let (_directory, _database, session, catalog) = storage().await;
        let link = LinkState::new();
        let control = Control {
            session: &session,
            link: &link,
            catalog: &catalog,
        };
        assert!(control.settings().await.unwrap().is_none());
        assert_eq!(
            serde_json::to_value(control.poll_link().await.unwrap()).unwrap(),
            json!({"status": "idle"})
        );

        *link.pending.lock().await = Some(PendingLink {
            code: "ABCD".to_owned(),
            offer: offer(),
            started: Instant::now() - super::super::LINK_WINDOW,
        });
        assert_eq!(
            serde_json::to_value(control.poll_link().await.unwrap()).unwrap(),
            json!({"status": "expired"})
        );
        assert!(link.pending.lock().await.is_none());
        assert_eq!(
            serde_json::to_value(
                control
                    .complete_poll(LinkPoll::Waiting(offer(), 42))
                    .await
                    .unwrap()
            )
            .unwrap(),
            json!({
                "status": "waiting", "code": "ABCD",
                "link": "https://link.stremio.com/ABCD",
                "qr": "https://link.stremio.com/qr/ABCD", "seconds_left": 42
            })
        );
        assert!(session.credentials().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn linked_poll_stores_the_key_and_returns_only_the_connection() {
        let (_directory, _database, session, catalog) = storage().await;
        let link = LinkState::new();
        let control = Control {
            session: &session,
            link: &link,
            catalog: &catalog,
        };
        let status = control
            .complete_poll(LinkPoll::Linked("session-secret".to_owned()))
            .await
            .unwrap();
        let connection = control.settings().await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            json!({"status": "linked", "connection": {
                "linked_at": connection.linked_at, "updated_at": connection.updated_at
            }})
        );
        assert_eq!(
            session.credentials().await.unwrap().unwrap().auth_key,
            "session-secret"
        );
    }

    #[tokio::test]
    async fn unlink_cancels_pending_and_removes_every_account_owned_row() {
        let (_directory, database, session, catalog) = storage().await;
        seed_sources(&database).await;
        session.save_session("session-secret").await.unwrap();
        let link = LinkState::new();
        link.start(offer()).await;
        let control = Control {
            session: &session,
            link: &link,
            catalog: &catalog,
        };
        control.clear().await.unwrap();
        control.clear().await.unwrap();
        assert!(link.pending.lock().await.is_none());
        assert!(session.credentials().await.unwrap().is_none());
        assert!(control.settings().await.unwrap().is_none());
        let sources: Vec<String> =
            sqlx::query_scalar("SELECT source_id FROM library_items ORDER BY source_id")
                .fetch_all(database.pool())
                .await
                .unwrap();
        // The arrivals were confirmed against this account's addons, so they go
        // with the session rather than waiting for the next refresh to empty them.
        assert_eq!(sources, ["desktop-apps"]);
    }

    #[tokio::test]
    async fn session_failure_cancels_pending_but_keeps_catalog_rows() {
        let (_directory, database, session, catalog) = storage().await;
        seed_sources(&database).await;
        sqlx::query("DROP TABLE stremio_settings")
            .execute(database.pool())
            .await
            .unwrap();
        let link = LinkState::new();
        link.start(offer()).await;
        let control = Control {
            session: &session,
            link: &link,
            catalog: &catalog,
        };
        assert!(matches!(
            control.settings().await,
            Err(ControlError::Storage(_))
        ));
        assert!(matches!(
            control.clear().await,
            Err(ControlError::Storage(_))
        ));
        assert!(link.pending.lock().await.is_none());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM library_items")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(count, 3);
        assert!(matches!(
            control
                .complete_poll(LinkPoll::Linked("session-secret".to_owned()))
                .await,
            Err(ControlError::Link(_))
        ));
    }

    #[tokio::test]
    async fn catalog_failure_is_reported_after_the_session_is_cleared() {
        let (_directory, database, session, catalog) = storage().await;
        session.save_session("session-secret").await.unwrap();
        sqlx::query("DROP TABLE library_items")
            .execute(database.pool())
            .await
            .unwrap();
        let link = LinkState::new();
        link.start(offer()).await;
        let control = Control {
            session: &session,
            link: &link,
            catalog: &catalog,
        };
        assert!(matches!(
            control.clear().await,
            Err(ControlError::Catalog(_))
        ));
        assert!(link.pending.lock().await.is_none());
        assert!(session.credentials().await.unwrap().is_none());
    }
}
