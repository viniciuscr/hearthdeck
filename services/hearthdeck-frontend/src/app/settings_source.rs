//! The settings screen's reads and writes against the daemon: the
//! smart-categories state, RomM connection and the Stremio link flow.

use cosmic::app::Task;
use cosmic::widget::text_input;

use crate::app_group::AppLibraryConfig;
use crate::screens::settings;
use crate::text_entry::Field;

use super::{App, Message, STREMIO_LINK_POLL_INTERVAL, STREMIO_PUBLISH_SETTLE};

impl App {
    pub(super) fn load_romm_settings(&mut self) -> Task<Message> {
        let effect = self.romm.update(settings::romm::Message::RefreshPressed);
        self.run_romm_effect(effect)
    }

    pub(super) fn update_romm(&mut self, message: settings::romm::Message) -> Task<Message> {
        let effect = self.romm.update(message);
        self.run_romm_effect(effect)
    }

    fn run_romm_effect(&mut self, effect: settings::romm::Effect) -> Task<Message> {
        use settings::romm::Effect;
        use settings::romm::Message as Romm;

        match effect {
            Effect::Nothing => return Task::none(),
            Effect::Load => {
                let client = self.daemon_client.clone();
                return Task::perform(
                    async move {
                        client
                            .romm_settings()
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| cosmic::Action::App(Message::Romm(Romm::Loaded(result))),
                );
            }
            Effect::Connect(base_url) => {
                let client = self.daemon_client.clone();
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
            Effect::Poll => return self.poll_romm_link(),
            Effect::FocusUsername => {
                self.text_entry.focus(Field::RommUsername);
                return text_input::focus(settings::romm::username_id());
            }
            Effect::FocusPassword => {
                self.text_entry.focus(Field::RommPassword);
                return text_input::focus(settings::romm::password_id());
            }
            Effect::EndTextEntry => return self.end_text_entry(),
            Effect::ReleaseTextEntry => {
                self.text_entry.end();
                return Task::none();
            }
            Effect::SignIn {
                base_url,
                username,
                password,
            } => {
                // The dialog keeps the outcome; the caret goes back to the
                // screen while the request runs, so the keyboard does not
                // outlive the form it was typing into.
                let ended = self.end_text_entry();
                let client = self.daemon_client.clone();
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
            Effect::Remove => {
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
            Effect::SignInSucceeded => {
                self.text_entry.end();
            }
            Effect::ConnectionChanged => {}
        }
        self.romm_connection_changed()
    }

    fn romm_connection_changed(&mut self) -> Task<Message> {
        self.invalidate_console_source();
        self.console_platforms.clear();
        self.tabs.sync_console_groups(&[]);
        if self.cur_section == crate::app_group::Section::ConsoleGames {
            self.cur_group = None;
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
                    .map(Box::new)
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Categorization(
                    settings::categorization::Message::Loaded(result),
                ))
            },
        )
    }

    pub(super) fn run_readiness_effect(
        &self,
        effect: settings::readiness::Effect,
    ) -> Task<Message> {
        match effect {
            settings::readiness::Effect::Nothing => Task::none(),
            settings::readiness::Effect::Load => self.load_retro_readiness(),
            settings::readiness::Effect::Sync => self.sync_retro_bios(),
            settings::readiness::Effect::Install => self.install_retro_cores(),
        }
    }

    /// Reads the emulator readiness report the settings card draws. Read
    /// whenever the screen is opened, for the same reason the state beside it
    /// is: a core may have been installed since the shell started.
    pub(super) fn load_retro_readiness(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .retro_readiness()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Readiness(settings::readiness::Message::Loaded(
                    result,
                )))
            },
        )
    }

    /// Fetches the BIOS files the active platforms are missing from the
    /// user's own BIOS source.
    pub(super) fn sync_retro_bios(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .sync_retro_bios()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Readiness(
                    settings::readiness::Message::BiosSynced(result),
                ))
            },
        )
    }

    /// Installs the missing core packages through the daemon's polkit-scoped
    /// helper. The daemon names the packages; the client only presses the
    /// button.
    pub(super) fn install_retro_cores(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .install_retro_cores()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Readiness(
                    settings::readiness::Message::CoresInstalled(result),
                ))
            },
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
            async move {
                client
                    .health()
                    .await
                    .map(|health| health.capabilities.categorization)
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Categorization(
                    settings::categorization::Message::CapabilityLoaded(result),
                ))
            },
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
            |result| {
                cosmic::Action::App(Message::Stremio(
                    settings::stremio_link::Message::ConnectionLoaded(result),
                ))
            },
        )
    }

    /// Asks the daemon for a link code.
    ///
    /// The code is approved on a device where the user is already signed in,
    /// which is why this screen asks for no password: there is no secret here
    /// for it to collect.
    fn start_stremio_link(&self) -> Task<Message> {
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
            |result| {
                cosmic::Action::App(Message::Stremio(
                    settings::stremio_link::Message::LinkStarted(result),
                ))
            },
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
            |result| {
                cosmic::Action::App(Message::Stremio(
                    settings::stremio_link::Message::LinkPolled(result),
                ))
            },
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
                    .refresh_source(hearthdeck_protocol::sources::STREMIO)
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Stremio(
                    settings::stremio_link::Message::SourceRefreshed(result),
                ))
            },
        )
    }

    /// Forgets the linked account, and the rows it published.
    fn unlink_stremio(&self) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .unlink_stremio()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Stremio(settings::stremio_link::Message::Unlinked(
                    result,
                )))
            },
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
            Effect::StartLink => self.start_stremio_link(),
            Effect::Unlink => self.unlink_stremio(),
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
    pub(super) fn run_categorization_effect(
        &self,
        effect: settings::categorization::Effect,
    ) -> Task<Message> {
        use settings::categorization::{Effect, Operation};
        let operation = match effect {
            Effect::Nothing => return Task::none(),
            Effect::Reload => return self.load_categorization(),
            Effect::Run(operation) => operation,
        };
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                match operation {
                    Operation::SetEnabled(on) => {
                        if on {
                            client.enable_categorization().await
                        } else {
                            client.disable_categorization().await
                        }
                    }
                    Operation::PublishRails => client.start_categorization_scan().await,
                    Operation::DetectCouch => client.detect_form_factors().await,
                }
                .map_err(|error| error.to_string())
            },
            |result| {
                cosmic::Action::App(Message::Categorization(
                    settings::categorization::Message::ActionFinished(result),
                ))
            },
        )
    }

    /// Applies settings that changed outside the shell.
    ///
    /// Only what actually changed is rebuilt: a save that does not touch this
    /// section's tabs leaves the cursor and the scroll where the user left
    /// them, and the echo of a save this shell made itself moves nothing.
    pub(super) fn apply_live_config(&mut self, config: AppLibraryConfig) -> Task<Message> {
        let forms_changed = config.form_overrides != self.config.form_overrides;
        self.config.desktop_input_apps = config.desktop_input_apps;
        if forms_changed {
            self.config.form_overrides = config.form_overrides;
            // The couch filter reads each entry's form, so an override moves
            // its tiles between the lists at once.
            self.catalog.project(&self.config);
            // The Desktop tab is there only while something wants a desktop.
            let had_group = self.cur_group.is_some();
            self.sync_local_catalog();
            self.refilter();
            if had_group && self.cur_group.is_none() {
                return Task::batch([self.list_replaced(), self.reveal_tab(0)]);
            }
        }
        Task::none()
    }
}
