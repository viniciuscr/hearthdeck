//! The settings screen's reads and writes against the daemon: the
//! smart-categories state, RomM connection and the Stremio link flow.

use cosmic::app::Task;
use cosmic::widget::text_input;

use crate::app_group::AppLibraryConfig;
use crate::screens::settings;
use crate::text_entry::Field;

use super::{
    App, CategorizationAction, Message, STREMIO_LINK_POLL_INTERVAL, STREMIO_PUBLISH_SETTLE,
};

impl App {
    pub(super) fn load_romm_settings(&mut self) -> Task<Message> {
        if self.romm.busy {
            return Task::none();
        }
        self.romm.busy = true;
        self.romm.error = None;
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .romm_settings()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::Romm(settings::romm::Message::Loaded(result))),
        )
    }

    pub(super) fn update_romm(&mut self, message: settings::romm::Message) -> Task<Message> {
        use settings::romm::Message as Romm;

        let changed = match message {
            Romm::UrlChanged(value) => {
                if !self.romm.busy && !self.romm.polling {
                    self.romm.base_url = value;
                }
                false
            }
            Romm::Loaded(result) => {
                self.romm.loaded(result);
                false
            }
            Romm::ConnectPressed => {
                if !self.romm.can_connect() {
                    return Task::none();
                }
                self.romm.begin();
                let client = self.daemon_client.clone();
                let base_url = self.romm.base_url.trim().to_owned();
                return Task::perform(
                    async move {
                        client
                            .start_romm_link(&base_url)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| cosmic::Action::App(Message::Romm(Romm::LinkStarted(result))),
                );
            }
            Romm::LinkStarted(result) => {
                if self.romm.link_started(result) {
                    return self.poll_romm_link();
                }
                false
            }
            Romm::LinkPolled(result) => match self.romm.link_polled(result) {
                settings::romm::Next::PollAgain => return self.poll_romm_link(),
                settings::romm::Next::Stop => false,
                settings::romm::Next::Connected => true,
            },
            Romm::SignInOpened => {
                if !self.romm.busy {
                    self.romm.sign_in = Some(settings::romm::SignIn::default());
                    self.romm.error = None;
                    // The dialog cannot be walked to, so the shell places the
                    // caret itself: the username field and the keyboard come up
                    // together, and the pad can type straight away.
                    self.text_entry.focus(Field::RommUsername);
                    return text_input::focus(settings::romm::username_id());
                }
                false
            }
            Romm::SignInClosed => {
                if !self.romm.busy {
                    self.romm.sign_in = None;
                }
                return self.end_text_entry();
            }
            Romm::UsernameChanged(value) => {
                if let Some(sign_in) = self.romm.sign_in.as_mut().filter(|_| !self.romm.busy) {
                    sign_in.username = value;
                }
                false
            }
            Romm::PasswordChanged(value) => {
                if let Some(sign_in) = self.romm.sign_in.as_mut().filter(|_| !self.romm.busy) {
                    sign_in.password = value;
                }
                false
            }
            Romm::SignInPressed => {
                let Some((username, password)) = self.romm.take_credentials() else {
                    return Task::none();
                };
                // The dialog keeps the outcome; the caret goes back to the
                // screen while the request runs, so the keyboard does not
                // outlive the form it was typing into.
                let ended = self.end_text_entry();
                let client = self.daemon_client.clone();
                let base_url = self.romm.base_url.trim().to_owned();
                return Task::batch([
                    ended,
                    Task::perform(
                        async move {
                            client
                                .romm_login(&base_url, &username, &password)
                                .await
                                .map_err(|error| error.to_string())
                        },
                        |result| cosmic::Action::App(Message::Romm(Romm::SignedIn(result))),
                    ),
                ]);
            }
            Romm::SignedIn(result) => {
                let changed = self.romm.signed_in(result);
                // A refused sign-in keeps the dialog with an empty password:
                // put the caret back in it, so the pad has somewhere to type
                // the retry without walking to a field it cannot reach.
                if self.romm.sign_in.is_some() && !self.romm.busy {
                    self.text_entry.focus(Field::RommPassword);
                    return text_input::focus(settings::romm::password_id());
                }
                self.text_entry.end();
                changed
            }
            Romm::RemovePressed => {
                if self.romm.busy || self.romm.polling || self.romm.connection.is_none() {
                    return Task::none();
                }
                self.romm.begin();
                let client = self.daemon_client.clone();
                return Task::perform(
                    async move {
                        client
                            .clear_romm_settings()
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| cosmic::Action::App(Message::Romm(Romm::Removed(result))),
                );
            }
            Romm::Removed(result) => self.romm.removed(result),
        };
        if !changed {
            return Task::none();
        }
        self.console.invalidate();
        self.console_platforms.clear();
        if self.config.sync_console_groups(&[]) {
            self.persist_config();
        }
        if self.cur_section == crate::app_group::Section::ConsoleGames {
            self.cur_group = None;
            self.facets.clear();
            self.facet_selection.clear();
            self.refilter();
        }
        if self.romm.connection.is_some() {
            self.load_consoles()
        } else {
            Task::none()
        }
    }

    /// One poll of the pending RomM approval; self-rescheduling like Stremio's.
    fn poll_romm_link(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                tokio::time::sleep(STREMIO_LINK_POLL_INTERVAL).await;
                client
                    .poll_romm_link()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Romm(settings::romm::Message::LinkPolled(result)))
            },
        )
    }

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
                    CategorizationAction::DetectCouch => client.detect_form_factors().await,
                }
                .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::CategorizationDone(result)),
        )
    }

    /// Rebuilds every section's tabs from the catalog and the console platforms,
    /// dropping what an older scan left behind and keeping the user's own
    /// folders.
    ///
    /// The library lands on its "all" tab: the indices the selection held name
    /// different tabs once the strips are rebuilt.
    pub(super) fn restore_default_categories(&mut self) -> Task<Message> {
        let platforms: Vec<(i64, String)> = self
            .console_platforms
            .iter()
            .map(|console| (console.id, console.label.clone()))
            .collect();
        if self
            .config
            .restore_default_groups(&self.catalog, &platforms)
        {
            self.persist_config();
        }
        self.apply_group(None)
    }

    /// Applies settings that changed outside the shell.
    ///
    /// Only what actually changed is rebuilt: a save that does not touch this
    /// section's tabs leaves the cursor and the scroll where the user left
    /// them, and the echo of a save this shell made itself moves nothing.
    pub(super) fn apply_live_config(&mut self, config: AppLibraryConfig) -> Task<Message> {
        let tabs_changed =
            config.sections.get(self.cur_section) != self.config.sections.get(self.cur_section);
        let forms_changed = config.form_overrides != self.config.form_overrides;
        self.config.desktop_input_apps = config.desktop_input_apps;
        if forms_changed {
            self.config.form_overrides = config.form_overrides;
            // The couch filter reads each entry's form, so an override moves
            // its tiles between the lists at once.
            self.config.apply_form_overrides(&mut self.catalog);
        }
        self.config.sections = config.sections;
        if tabs_changed {
            return self.apply_group(None);
        }
        if forms_changed {
            self.refilter();
        }
        Task::none()
    }
}
