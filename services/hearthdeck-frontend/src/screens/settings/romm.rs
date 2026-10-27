use std::sync::LazyLock;

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::theme;
use cosmic::widget::qr_code::{self, QRCode};
use cosmic::widget::{Id, column, container, dialog, row, text, text_input};

use crate::app;
use crate::fl;
use crate::providers::daemon::{RommLink, RommLinkOffer, RommSettings};
use crate::style::{
    TEXT_BODY, TEXT_TITLE, destructive_button_class, primary_action_button_class,
    standard_button_class,
};
use crate::text_entry::Field;

/// Side length of one QR cell; large enough to scan off a TV across a room.
const QR_CELL_SIZE: f32 = 6.0;

static USERNAME_ID: LazyLock<Id> = LazyLock::new(|| Id::new("romm-username"));
static PASSWORD_ID: LazyLock<Id> = LazyLock::new(|| Id::new("romm-password"));

/// The sign-in dialog's username field.
///
/// The dialog is not a place the pad can walk to, so the shell puts the caret
/// here itself when the dialog opens - and that is what raises the keyboard.
pub fn username_id() -> Id {
    USERNAME_ID.clone()
}

/// The sign-in dialog's password field, which the username submits into.
pub fn password_id() -> Id {
    PASSWORD_ID.clone()
}

#[derive(Clone)]
pub enum Message {
    UrlChanged(String),
    ConnectPressed,
    LinkStarted(Result<RommLinkOffer, String>),
    LinkPolled(Result<RommLink, String>),
    SignInOpened,
    SignInClosed,
    UsernameChanged(String),
    PasswordChanged(String),
    SignInPressed,
    SignedIn(Result<RommSettings, String>),
    RemovePressed,
    Loaded(Result<Option<RommSettings>, String>),
    Removed(Result<(), String>),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Input messages carry credentials.
        formatter
            .debug_tuple("Romm")
            .field(&std::mem::discriminant(self))
            .finish()
    }
}

pub struct Pending {
    pub code: String,
    pub link: String,
    pub seconds_left: u64,
    qr: Option<qr_code::Data>,
}

impl Pending {
    fn new(code: String, link: String, seconds_left: u64) -> Self {
        let qr = qr_code::Data::new(link.as_bytes())
            .inspect_err(|error| log::warn!("RomM link QR code failed: {error}"))
            .ok();
        Self {
            code,
            link,
            seconds_left,
            qr,
        }
    }
}

#[derive(Default)]
pub struct SignIn {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    Expired,
    Denied,
}

/// What the shell does after a link poll.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Stop,
    PollAgain,
    Connected,
}

#[derive(Default)]
pub struct Form {
    pub connection: Option<RommSettings>,
    pub base_url: String,
    pub pending: Option<Pending>,
    pub sign_in: Option<SignIn>,
    /// A poll chain is alive; a second Connect must not start another.
    pub polling: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub notice: Option<Notice>,
}

impl Form {
    pub fn can_connect(&self) -> bool {
        !self.busy && !self.polling && !self.base_url.trim().is_empty()
    }

    pub fn can_sign_in(&self) -> bool {
        !self.busy
            && !self.base_url.trim().is_empty()
            && self
                .sign_in
                .as_ref()
                .is_some_and(|s| !s.username.trim().is_empty() && !s.password.is_empty())
    }

    pub fn begin(&mut self) {
        self.busy = true;
        self.error = None;
        self.notice = None;
    }

    pub fn loaded(&mut self, result: Result<Option<RommSettings>, String>) {
        self.busy = false;
        match result {
            Ok(connection) => {
                self.base_url = connection
                    .as_ref()
                    .map_or_else(String::new, |s| s.base_url.clone());
                self.connection = connection;
                self.error = None;
            }
            Err(error) => self.failed(error),
        }
    }

    /// Returns whether a poll chain should start.
    pub fn link_started(&mut self, result: Result<RommLinkOffer, String>) -> bool {
        self.busy = false;
        match result {
            Ok(offer) => {
                self.pending = Some(Pending::new(
                    offer.code,
                    offer.link,
                    offer.expires_in_seconds,
                ));
                self.polling = true;
                true
            }
            Err(error) => {
                self.failed(error);
                false
            }
        }
    }

    pub fn link_polled(&mut self, result: Result<RommLink, String>) -> Next {
        let next = match result {
            Ok(RommLink::Waiting {
                code,
                link,
                seconds_left,
            }) if self.pending.is_some() => {
                match self.pending.as_mut() {
                    Some(pending) if pending.link == link => pending.seconds_left = seconds_left,
                    _ => self.pending = Some(Pending::new(code, link, seconds_left)),
                }
                return Next::PollAgain;
            }
            Ok(RommLink::Linked { settings }) => {
                self.loaded(Ok(Some(settings)));
                Next::Connected
            }
            Ok(RommLink::Expired) => {
                self.notice = Some(Notice::Expired);
                Next::Stop
            }
            Ok(RommLink::Denied) => {
                self.notice = Some(Notice::Denied);
                Next::Stop
            }
            Ok(RommLink::Idle | RommLink::Waiting { .. }) => Next::Stop,
            Err(error) => {
                self.failed(error);
                Next::Stop
            }
        };
        self.pending = None;
        self.polling = false;
        next
    }

    /// Takes the credentials out of the dialog; the password leaves the form here.
    pub fn take_credentials(&mut self) -> Option<(String, String)> {
        if !self.can_sign_in() {
            return None;
        }
        let sign_in = self.sign_in.as_mut()?;
        let credentials = (
            sign_in.username.trim().to_owned(),
            std::mem::take(&mut sign_in.password),
        );
        self.begin();
        Some(credentials)
    }

    /// Returns whether the connection changed.
    pub fn signed_in(&mut self, result: Result<RommSettings, String>) -> bool {
        match result {
            Ok(connection) => {
                self.sign_in = None;
                self.pending = None;
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
                self.pending = None;
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

fn romm(message: Message) -> app::Message {
    app::Message::Romm(message)
}

pub fn view(form: &Form) -> Element<'_, app::Message> {
    let spacing = theme::spacing();
    let mut url = text_input("http://127.0.0.1:8080", &form.base_url)
        .size(TEXT_BODY)
        .on_focus(app::Message::TextFieldFocused(Field::RommUrl))
        .on_unfocus(app::Message::TextFieldBlurred(Field::RommUrl))
        // Accepting the URL puts the keyboard away; the value already applies
        // as it is typed.
        .on_submit(|_| app::Message::TextFieldSubmitted(Field::RommUrl));
    if !form.busy && !form.polling {
        url = url.on_input(|value| romm(Message::UrlChanged(value)));
    }
    let status = if form.busy {
        fl!("romm-working")
    } else if form.connection.as_ref().is_some_and(|s| s.configured) {
        fl!("romm-configured")
    } else {
        fl!("romm-not-configured")
    };
    let section = super::titled_section(fl!("romm-heading"))
        .add(super::summary(status, fl!("romm-intro")))
        .add(super::item(fl!("romm-url"), url))
        .add_maybe(form.pending.as_ref().map(pending_block))
        .add_maybe(form.notice.map(|notice| {
            super::body_line(match notice {
                Notice::Expired => fl!("romm-link-expired"),
                Notice::Denied => fl!("romm-link-denied"),
            })
        }))
        .add_maybe(
            form.error
                .as_ref()
                .map(|error| super::error_line(fl!("romm-failed", reason = error.as_str()))),
        );
    section
        .add(super::button_row(
            row![
                super::action_button(
                    fl!("romm-connect"),
                    romm(Message::ConnectPressed),
                    primary_action_button_class(),
                    form.can_connect(),
                ),
                super::action_button(
                    fl!("romm-sign-in"),
                    romm(Message::SignInOpened),
                    standard_button_class(),
                    !form.busy && !form.base_url.trim().is_empty(),
                ),
                super::action_button(
                    fl!("romm-remove"),
                    romm(Message::RemovePressed),
                    destructive_button_class(),
                    !form.busy && !form.polling && form.connection.is_some(),
                ),
            ]
            .spacing(spacing.space_s),
        ))
        .into()
}

fn pending_block(pending: &Pending) -> Element<'_, app::Message> {
    let spacing = theme::spacing();
    let details = column![
        super::body_line(fl!("romm-pending", seconds = pending.seconds_left)),
        text::title1(pending.code.as_str()).size(TEXT_TITLE),
        super::caption_line(pending.link.clone()),
    ]
    .spacing(spacing.space_xs);
    let mut body = row![].spacing(spacing.space_m);
    if let Some(qr) = &pending.qr {
        body = body.push(QRCode::new(qr).cell_size(QR_CELL_SIZE));
    }
    container(body.push(details))
        .padding(spacing.space_s)
        .width(Length::Fill)
        .class(theme::Container::Primary)
        .into()
}

/// The username/password fallback, drawn by the app as a modal dialog.
pub fn sign_in_dialog(form: &Form) -> Option<Element<'_, app::Message>> {
    let sign_in = form.sign_in.as_ref()?;
    let spacing = theme::spacing();
    let mut username = text_input(fl!("romm-username"), &sign_in.username)
        .size(TEXT_BODY)
        .id(USERNAME_ID.clone())
        .on_focus(app::Message::TextFieldFocused(Field::RommUsername))
        .on_unfocus(app::Message::TextFieldBlurred(Field::RommUsername));
    let mut password = text_input(fl!("romm-password"), &sign_in.password)
        .password()
        .size(TEXT_BODY)
        .id(PASSWORD_ID.clone())
        .on_focus(app::Message::TextFieldFocused(Field::RommPassword))
        .on_unfocus(app::Message::TextFieldBlurred(Field::RommPassword));
    if !form.busy {
        username = username
            .on_input(|value| romm(Message::UsernameChanged(value)))
            // Enter moves to the password. Nothing can walk to it, so this is
            // how the pad gets from one half of the form to the other.
            .on_submit(|_| app::Message::TextFieldSubmitted(Field::RommUsername));
        password = password
            .on_input(|value| romm(Message::PasswordChanged(value)))
            .on_submit(|_| app::Message::TextFieldSubmitted(Field::RommPassword));
    }
    let mut controls = column![username, password].spacing(spacing.space_s);
    if let Some(error) = &form.error {
        controls = controls.push(super::error_line(fl!(
            "romm-failed",
            reason = error.as_str()
        )));
    }
    Some(
        dialog()
            .title(fl!("romm-sign-in-title"))
            .body(fl!("romm-sign-in-body"))
            .control(controls)
            .primary_action(super::action_button(
                fl!("romm-sign-in"),
                romm(Message::SignInPressed),
                primary_action_button_class(),
                form.can_sign_in(),
            ))
            .secondary_action(super::action_button(
                fl!("romm-cancel"),
                romm(Message::SignInClosed),
                standard_button_class(),
                !form.busy,
            ))
            .into(),
    )
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

    fn offer() -> RommLinkOffer {
        RommLinkOffer {
            code: "ABCD-1234".into(),
            link: "http://192.168.1.5:8080/pair?code=ABCD-1234".into(),
            expires_in_seconds: 600,
        }
    }

    fn linking_form() -> Form {
        let mut form = Form {
            base_url: "http://127.0.0.1:8080".into(),
            ..Form::default()
        };
        form.begin();
        assert!(form.link_started(Ok(offer())));
        form
    }

    #[test]
    fn a_started_link_shows_a_scannable_code_and_blocks_a_second_chain() {
        let form = linking_form();
        let pending = form.pending.as_ref().expect("pending link");
        assert_eq!(pending.code, "ABCD-1234");
        assert!(pending.qr.is_some());
        assert!(!form.can_connect());
    }

    #[test]
    fn waiting_keeps_polling_and_counts_down() {
        let mut form = linking_form();
        let next = form.link_polled(Ok(RommLink::Waiting {
            code: "ABCD-1234".into(),
            link: offer().link,
            seconds_left: 42,
        }));
        assert_eq!(next, Next::PollAgain);
        assert_eq!(form.pending.as_ref().map(|p| p.seconds_left), Some(42));
    }

    #[test]
    fn approval_saves_the_connection_and_ends_the_chain() {
        let mut form = linking_form();
        assert_eq!(
            form.link_polled(Ok(RommLink::Linked {
                settings: connection()
            })),
            Next::Connected
        );
        assert!(form.pending.is_none());
        assert!(form.connection.is_some());
        assert!(form.can_connect());
    }

    #[test]
    fn expiry_denial_and_errors_end_the_chain_with_a_reason() {
        for (result, notice, error) in [
            (Ok(RommLink::Expired), Some(Notice::Expired), false),
            (Ok(RommLink::Denied), Some(Notice::Denied), false),
            (Err("daemon unavailable".to_owned()), None, true),
        ] {
            let mut form = linking_form();
            assert_eq!(form.link_polled(result), Next::Stop);
            assert!(form.pending.is_none());
            assert!(!form.polling);
            assert_eq!(form.notice, notice);
            assert_eq!(form.error.is_some(), error);
        }
    }

    #[test]
    fn a_poll_after_the_code_was_dropped_stops_instead_of_reviving_it() {
        let mut form = Form {
            polling: true,
            ..Form::default()
        };
        let next = form.link_polled(Ok(RommLink::Waiting {
            code: "ABCD-1234".into(),
            link: offer().link,
            seconds_left: 42,
        }));
        assert_eq!(next, Next::Stop);
        assert!(form.pending.is_none());
        assert!(!form.polling);
    }

    #[test]
    fn signing_in_hands_over_the_password_once_and_keeps_it_out_of_the_form() {
        let mut form = Form {
            base_url: "http://127.0.0.1:8080".into(),
            sign_in: Some(SignIn {
                username: " admin ".into(),
                password: "hunter2".into(),
            }),
            ..Form::default()
        };
        assert_eq!(
            form.take_credentials(),
            Some(("admin".to_owned(), "hunter2".to_owned()))
        );
        assert!(form.sign_in.as_ref().is_some_and(|s| s.password.is_empty()));
        assert!(form.busy);
        assert!(form.take_credentials().is_none());

        assert!(!form.signed_in(Err("RomM refused the credentials".into())));
        assert!(
            form.sign_in.is_some(),
            "a refusal keeps the dialog for retry"
        );
        assert!(form.signed_in(Ok(connection())));
        assert!(form.sign_in.is_none());
        assert!(form.connection.is_some());
    }

    #[test]
    fn removal_clears_the_url_and_a_refused_removal_keeps_the_connection() {
        let mut form = Form::default();
        form.loaded(Ok(Some(connection())));
        assert!(!form.removed(Err("daemon unavailable".into())));
        assert!(form.connection.is_some());
        assert!(form.removed(Ok(())));
        assert!(form.base_url.is_empty());
        assert!(form.connection.is_none());
    }

    #[test]
    fn credential_input_is_redacted_from_message_debug_output() {
        for message in [
            Message::PasswordChanged("private-secret".into()),
            Message::UsernameChanged("private-secret".into()),
            Message::UrlChanged("http://private-secret@localhost".into()),
        ] {
            assert!(!format!("{message:?}").contains("private-secret"));
        }
    }
}
