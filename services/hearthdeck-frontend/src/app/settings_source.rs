//! The settings screen's reads and writes against the daemon: the
//! smart-categories state and the Stremio link flow.

use cosmic::app::Task;

use crate::screens::settings;

use super::{
    App, CategorizationAction, Message, STREMIO_LINK_POLL_INTERVAL, STREMIO_PUBLISH_SETTLE,
};

impl App {
    /// Fetches the smart-categories state, which is what the settings screen
    /// draws.
    ///
    /// Read again whenever the screen is opened rather than cached from startup,
    /// because the switch is a setting something else may have changed since.
    pub(super) fn load_categorization(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .categorization()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CategorizationLoaded(result)),
        )
    }

    /// Fetches the daemon's health, which is where the deployment says what it
    /// provides at all.
    ///
    /// Read whenever the settings screen is opened, for the same reason the state
    /// beside it is: the categorization program may have been installed or
    /// removed since the shell started.
    pub(super) fn load_health(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move { client.health().await.map_err(|error| error.to_string()) },
            |result| cosmic::Action::App(Message::HealthLoaded(result)),
        )
    }

    /// Reads whether a Stremio account is linked, and has the section say it is
    /// checking while the answer is on its way.
    ///
    /// Run whenever the settings screen opens, and again when a pending link
    /// resolves without saying what it resolved to.
    pub(super) fn load_stremio_connection(&mut self) -> Task<Message> {
        self.stremio.begin_read();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .stremio_connection()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::StremioConnectionLoaded(result)),
        )
    }

    /// Asks the daemon for a link code.
    ///
    /// The code is approved on a device where the user is already signed in,
    /// which is why this screen asks for no password: there is no secret here
    /// for it to collect.
    pub(super) fn start_stremio_link(&mut self) -> Task<Message> {
        self.stremio.begin_link();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .start_stremio_link()
                    .await
                    .map(|offer| settings::stremio_link::Pending {
                        code: offer.code,
                        link: offer.link,
                        seconds_left: offer.expires_in_seconds,
                    })
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::StremioLinkStarted(result)),
        )
    }

    /// One poll of the pending link, after `delay`.
    ///
    /// Self-rescheduling rather than a subscription: the chain ends the moment
    /// the daemon stops reporting that a link is waiting, so there is nothing
    /// left to unsubscribe from.
    pub(super) fn poll_stremio_link(&self, delay: std::time::Duration) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client
                    .poll_stremio_link()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::StremioLinkPolled(result)),
        )
    }

    /// Asks the daemon to publish the linked account's rows now.
    ///
    /// The provider would get there on its own interval, but a person who just
    /// approved a code expects the dashboard to fill in front of them.
    pub(super) fn refresh_stremio_source(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .refresh_source("stremio")
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::StremioSourceRefreshed(result)),
        )
    }

    /// Forgets the linked account, and the rows it published.
    pub(super) fn unlink_stremio(&mut self) -> Task<Message> {
        self.stremio.begin_unlink();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .unlink_stremio()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::StremioUnlinked(result)),
        )
    }

    /// Turns the flow's answer into the work that answer asks for: keep watching
    /// the code, publish what a link just produced, or re-read the connection.
    ///
    /// The flow owns which of those is right; this owns how each is run, so no
    /// arm in `update` has to know what a reply meant.
    pub(super) fn run_stremio_effect(
        &mut self,
        effect: settings::stremio_link::Effect,
    ) -> Task<Message> {
        use crate::screens::settings::stremio_link::Effect;
        match effect {
            Effect::Nothing => Task::none(),
            Effect::Poll => self.poll_stremio_link(STREMIO_LINK_POLL_INTERVAL),
            // Both halves go together: the publish is what fills the rows, and
            // the delayed read is what makes the rail holding them appear.
            Effect::Publish => Task::batch([
                self.refresh_stremio_source(),
                self.load_collections_after(STREMIO_PUBLISH_SETTLE),
            ]),
            Effect::ReloadConnection => self.load_stremio_connection(),
        }
    }

    /// Runs a categorization action, then re-reads what the daemon now thinks.
    ///
    /// No action writes to the screen's state directly: the daemon is asked, and
    /// then asked again, so what is drawn is always its answer rather than the
    /// shell's guess at it.
    pub(super) fn categorization_action(&mut self, action: CategorizationAction) -> Task<Message> {
        // A new action starts clean: whatever the last one left in the section's
        // status text is about something the user has already moved past.
        self.categorization_notice = None;
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                match action {
                    CategorizationAction::SetEnabled(on) => {
                        if on {
                            client.enable_categorization().await
                        } else {
                            client.disable_categorization().await
                        }
                    }
                    CategorizationAction::Scan => client.start_categorization_scan().await,
                    CategorizationAction::RemoveModel => client.delete_categorization_model().await,
                }
                .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CategorizationDone(result)),
        )
    }

    /// Publishes the last run's rails as dashboard collections.
    ///
    /// The report the publish reads is already the daemon's, so this asks for
    /// that work rather than calling the model again - and clears the last notice,
    /// because the user has now asked for something.
    pub(super) fn create_categorization_collections(&mut self) -> Task<Message> {
        self.categorization_notice = None;
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .create_categorization_collections()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CategorizationCollectionsCreated(result)),
        )
    }

    /// Hands the Applications tabs back to the ones derived from the entries.
    ///
    /// This is the way out of a scan the user does not want: their own folders
    /// survive, because only the scan's tabs are dropped. The selection follows
    /// the folder it was on rather than the index, because the index no longer
    /// names the same tab once the scan's tabs are gone.
    pub(super) fn restore_derived_categories(&mut self) {
        let selected = self
            .cur_group
            .and_then(|index| self.config.sections.get(self.cur_section).get(index))
            .cloned();
        if !self.config.sync_categorization(&[]) {
            return;
        }
        self.persist_config();
        self.cur_group = selected.and_then(|selected| {
            self.config
                .sections
                .get(self.cur_section)
                .iter()
                .position(|group| group == &selected)
        });
    }
}
