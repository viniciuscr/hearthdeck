use cosmic::Element;
use cosmic::theme;
use cosmic::widget::{column, row, settings, text, text_input};

use crate::app;
use crate::fl;
use crate::providers::daemon::RommSettings;
use crate::style::{destructive_button_class, primary_action_button_class};

#[derive(Clone)]
pub enum Message {
    UrlChanged(String),
    TokenChanged(String),
    SavePressed,
    RemovePressed,
    Loaded(Result<Option<RommSettings>, String>),
    Saved(Result<RommSettings, String>),
    Removed(Result<(), String>),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Input messages can contain credentials, including a token pasted into the URL.
        formatter
            .debug_tuple("Romm")
            .field(&std::mem::discriminant(self))
            .finish()
    }
}

#[derive(Default)]
pub struct Form {
    pub connection: Option<RommSettings>,
    pub base_url: String,
    pub token: String,
    pub busy: bool,
    pub error: Option<String>,
}

impl Form {
    pub fn can_save(&self) -> bool {
        !self.busy && !self.base_url.trim().is_empty() && !self.token.trim().is_empty()
    }

    pub fn loaded(&mut self, result: Result<Option<RommSettings>, String>) {
        self.busy = false;
        match result {
            Ok(connection) => {
                self.base_url = connection
                    .as_ref()
                    .map_or_else(String::new, |s| s.base_url.clone());
                self.connection = connection;
                self.token.clear();
                self.error = None;
            }
            Err(error) => self.failed(error),
        }
    }

    pub fn saved(&mut self, result: Result<RommSettings, String>) -> bool {
        match result {
            Ok(connection) => {
                self.loaded(Ok(Some(connection)));
                true
            }
            Err(error) => {
                self.busy = false;
                self.failed(error);
                false
            }
        }
    }

    pub fn removed(&mut self, result: Result<(), String>) -> bool {
        match result {
            Ok(()) => {
                self.loaded(Ok(None));
                true
            }
            Err(error) => {
                self.busy = false;
                self.failed(error);
                false
            }
        }
    }

    fn failed(&mut self, error: String) {
        log::warn!("RomM settings request failed: {error}");
        self.error = Some(error);
    }
}

pub fn view(form: &Form) -> Element<'_, app::Message> {
    let spacing = theme::spacing();
    let mut url = text_input("http://127.0.0.1:8080", &form.base_url);
    let mut token = text_input(fl!("romm-token"), &form.token).password();
    if !form.busy {
        url = url.on_input(|value| app::Message::Romm(Message::UrlChanged(value)));
        token = token.on_input(|value| app::Message::Romm(Message::TokenChanged(value)));
    }
    let status = if form.busy {
        fl!("romm-working")
    } else if form.connection.as_ref().is_some_and(|s| s.configured) {
        fl!("romm-configured")
    } else {
        fl!("romm-not-configured")
    };
    let mut section = settings::section()
        .title(fl!("romm-heading"))
        .add(text::body(fl!("romm-intro")))
        .add(text::body(status))
        .add(column![text::body(fl!("romm-url")), url].spacing(spacing.space_xs))
        .add(column![text::body(fl!("romm-token")), token].spacing(spacing.space_xs));
    if let Some(error) = &form.error {
        section = section.add(text::caption(fl!("romm-failed", reason = error.as_str())));
    }
    section
        .add(
            row![
                super::action_button(
                    fl!("romm-save"),
                    app::Message::Romm(Message::SavePressed),
                    primary_action_button_class(),
                    form.can_save(),
                ),
                super::action_button(
                    fl!("romm-remove"),
                    app::Message::Romm(Message::RemovePressed),
                    destructive_button_class(),
                    !form.busy && form.connection.is_some(),
                ),
            ]
            .spacing(spacing.space_s),
        )
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> RommSettings {
        RommSettings {
            base_url: "http://127.0.0.1:8080".into(),
            configured: true,
        }
    }

    #[test]
    fn loading_settings_restores_the_url_but_never_a_token() {
        let mut form = Form::default();
        form.loaded(Ok(Some(connection())));
        assert_eq!(form.base_url, "http://127.0.0.1:8080");
        assert!(form.token.is_empty());
        assert!(!form.can_save());
    }

    #[test]
    fn saving_requires_both_fields_and_no_pending_request() {
        let mut form = Form::default();
        for (url, token, busy, expected) in [
            ("", "secret", false, false),
            ("http://localhost:8080", " ", false, false),
            ("http://localhost:8080", "secret", true, false),
            ("http://localhost:8080", "secret", false, true),
        ] {
            form.base_url = url.into();
            form.token = token.into();
            form.busy = busy;
            assert_eq!(form.can_save(), expected, "url={url:?}, busy={busy}");
        }
    }

    #[test]
    fn refused_writes_keep_the_saved_connection_and_draft_for_retry() {
        let mut form = Form::default();
        form.loaded(Ok(Some(connection())));
        form.token = "replacement".into();
        form.busy = true;
        assert!(!form.saved(Err("invalid URL".into())));
        assert_eq!(form.error.as_deref(), Some("invalid URL"));
        assert_eq!(form.token, "replacement");
        assert!(form.connection.is_some());
        assert!(form.can_save());
        assert!(!form.removed(Err("daemon unavailable".into())));
        assert!(form.connection.is_some());
    }

    #[test]
    fn a_failed_read_keeps_the_draft_and_allows_retry() {
        let mut form = Form {
            base_url: "http://localhost:8080".into(),
            token: "replacement".into(),
            busy: true,
            ..Form::default()
        };
        form.loaded(Err("daemon unavailable".into()));
        assert_eq!(form.error.as_deref(), Some("daemon unavailable"));
        assert_eq!(form.base_url, "http://localhost:8080");
        assert_eq!(form.token, "replacement");
        assert!(form.can_save());
    }

    #[test]
    fn successful_writes_clear_the_token_and_removal_clears_the_url() {
        let mut form = Form {
            token: "secret".into(),
            busy: true,
            ..Form::default()
        };
        assert!(form.saved(Ok(connection())));
        assert!(form.token.is_empty());
        assert!(form.connection.is_some());
        assert!(!form.busy);
        assert!(form.removed(Ok(())));
        assert!(form.base_url.is_empty());
        assert!(form.connection.is_none());
    }

    #[test]
    fn credential_input_is_redacted_from_message_debug_output() {
        for message in [
            Message::TokenChanged("private-secret".into()),
            Message::UrlChanged("http://user:private-secret@localhost".into()),
        ] {
            assert!(!format!("{message:?}").contains("private-secret"));
        }
    }
}
