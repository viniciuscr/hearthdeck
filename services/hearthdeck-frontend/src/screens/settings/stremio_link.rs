//! The Stremio link flow: linking an account by code, and letting it go.
//!
//! Nothing in this flow ever holds a credential. Linking is a code the user
//! approves on a device where they are already signed in, so there is no
//! password to type on a television and none to store; the daemon keeps the
//! session the link produces, and this module is only ever told whether one
//! exists.
//!
//! The flow is a small state machine rather than loose fields in the shell:
//! every answer from the daemon moves it, and each move says what the shell
//! must do next - keep polling, publish the rows, re-read the connection - so
//! the decisions can be tested with no app around them.

use crate::providers::daemon::{StremioConnection, StremioLink};

/// The link a user is in the middle of approving: the code they are typing
/// elsewhere, the link that already carries it, and how long both have left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The four characters to enter at `link`.
    pub code: String,
    /// The page to open, which already carries the code.
    pub link: String,
    /// How long the code still works for.
    pub seconds_left: u64,
}

/// What the shell must do next once an answer from the daemon has landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The answer settled nothing that needs the shell.
    Nothing,
    /// The code is still being approved: ask the daemon again in a moment.
    Poll,
    /// The account is linked: ask the daemon to publish its rows now, and
    /// re-read the collections those rows land in.
    Publish,
    /// Nothing is pending any more: read what the connection now is rather
    /// than guessing from the answer that closed the flow.
    ReloadConnection,
}

/// One control this section offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Ask the daemon for a code, and show it. Always offered: re-linking is
    /// how a session Stremio has revoked gets repaired.
    Link,
    /// Forget the account, and the rows it published.
    Unlink,
}

/// Where the link flow stands, from the daemon's point of view.
#[derive(Clone, Debug, Default)]
pub struct Flow {
    /// The linked account. `None` covers both "not linked" and "not read
    /// yet", which `reading` tells apart.
    pub connection: Option<StremioConnection>,
    /// A read is in flight, so the section says it is checking rather than
    /// claiming that nothing is linked.
    pub reading: bool,
    /// The link being approved, if any. While one is pending its code is the
    /// only thing on this section worth reading.
    pub pending: Option<Pending>,
    /// A request is in flight, so nothing may be pressed twice.
    pub busy: bool,
    /// The last failure, in the daemon's or Stremio's own words.
    pub error: Option<String>,
    /// The last code expired before it was approved. Set so the section can
    /// say what happened to the code instead of silently drawing "not
    /// linked"; cleared by the next request.
    pub expired: bool,
}

/// The controls on offer, in drawing order. Linking stays available even once
/// an account is linked, so a revoked session is repairable without support.
pub fn actions(flow: &Flow) -> Vec<Action> {
    let mut actions = vec![Action::Link];
    // Unlinking is only drawn when there is an account to forget: a button
    // that could not work is worse than no button.
    if flow.connection.is_some() {
        actions.push(Action::Unlink);
    }
    actions
}

/// Whether pressing `action` would be accepted in the state `flow` describes.
/// Until the pad's focus walk reaches this screen, the shell uses this to
/// decide whether a control is drawn enabled; a control that could not be
/// confirmed is drawn disabled rather than focused and dead.
pub fn pressable(action: Action, flow: &Flow) -> bool {
    // A request in flight owns the flow: a second press is how a link gets
    // asked for twice.
    if flow.busy {
        return false;
    }
    match action {
        // Linking is always the way in, and the way back from a session
        // Stremio has revoked.
        Action::Link => true,
        // Nothing linked means nothing to forget.
        Action::Unlink => flow.connection.is_some(),
    }
}

impl Flow {
    /// The link button: ask the daemon for a code. The request is in flight
    /// until the daemon answers with one.
    ///
    /// A new request starts from a clean slate: the last failure and the last
    /// expiry both describe a code that is no longer the one being asked for.
    pub fn begin_link(&mut self) {
        self.busy = true;
        self.error = None;
        self.expired = false;
    }

    /// The daemon answered the request for a code.
    ///
    /// The buttons come back either way; a refusal says why and leaves no
    /// code behind, so the section never looks stuck on a link that was never
    /// issued. An issued code queues the first poll: the wait belongs to the
    /// phone in somebody's hand, so the flow keeps asking until the daemon
    /// stops saying the link is pending.
    pub fn link_started(&mut self, result: Result<Pending, String>) -> Effect {
        self.busy = false;
        let pending = match result {
            Ok(pending) => pending,
            Err(error) => {
                log::warn!("could not start the Stremio link: {error}");
                self.error = Some(error);
                return Effect::Nothing;
            }
        };
        self.error = None;
        self.expired = false;
        self.pending = Some(pending);
        Effect::Poll
    }

    /// One poll of the pending link came back.
    pub fn link_polled(&mut self, result: Result<StremioLink, String>) -> Effect {
        // Still deciding: the countdown is the daemon's to say, so the code on
        // screen follows its answer rather than counting down locally past what
        // the daemon believes.
        match result {
            Ok(StremioLink::Waiting {
                code,
                link,
                seconds_left,
            }) => {
                self.pending = Some(Pending {
                    code,
                    link,
                    seconds_left,
                });
                Effect::Poll
            }
            // Approved: the code has done its job, and the account the daemon
            // holds is the one recorded here. The shell publishes the rows now
            // rather than waiting for the provider's own interval, and re-reads
            // the collections those rows land in once it has had time to
            // publish.
            Ok(StremioLink::Linked { connection }) => {
                self.pending = None;
                self.error = None;
                self.expired = false;
                self.connection = Some(connection);
                log::info!("stremio account linked");
                Effect::Publish
            }
            // The window closed while the user was still deciding: the dead
            // code comes down and the section says what happened to it, since
            // silence would leave the user waiting on a code nothing is
            // watching any more.
            Ok(StremioLink::Expired) => {
                self.pending = None;
                self.error = None;
                self.expired = true;
                Effect::Nothing
            }
            // Nothing was pending: the link resolved between polls, or the
            // daemon restarted. Re-read rather than guess which.
            Ok(StremioLink::Idle) => {
                self.pending = None;
                Effect::ReloadConnection
            }
            // Polling a daemon that is not answering would go on forever: the
            // flow ends, the code goes, and the section says why.
            Err(error) => {
                log::warn!("could not poll the Stremio link: {error}");
                self.pending = None;
                self.error = Some(error);
                Effect::Nothing
            }
        }
    }

    /// The unlink button: forget the linked account.
    pub fn begin_unlink(&mut self) {
        self.busy = true;
        self.error = None;
    }

    /// The daemon answered the unlink request.
    ///
    /// The account and the code go in one stroke, so nothing about the old
    /// session is left to draw. A refusal keeps the account: a failed request
    /// is never a reason to tell the user their account is gone.
    pub fn unlinked(&mut self, result: Result<(), String>) -> Effect {
        self.busy = false;
        match result {
            Ok(()) => {
                self.connection = None;
                self.pending = None;
                self.error = None;
                self.expired = false;
                log::info!("stremio account unlinked");
                Effect::Nothing
            }
            Err(error) => {
                log::warn!("could not unlink the Stremio account: {error}");
                self.error = Some(error);
                Effect::Nothing
            }
        }
    }

    /// Whether an account is linked is being read.
    pub fn begin_read(&mut self) {
        self.reading = true;
    }

    /// The connection read came back.
    ///
    /// A successful read is the whole answer, `None` included: an account
    /// that is not there is a fact, not a missing one. It also retires the
    /// expiry news — the daemon has spoken again, which is newer than the code
    /// the user lost. A failed read says why and leaves the last known answer
    /// alone rather than inventing one.
    pub fn connection_read(&mut self, result: Result<Option<StremioConnection>, String>) {
        self.reading = false;
        match result {
            Ok(connection) => {
                self.connection = connection;
                self.error = None;
                self.expired = false;
            }
            Err(error) => {
                log::warn!("could not read the Stremio connection: {error}");
                self.error = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Effect, Flow, Pending, actions, pressable};
    use crate::providers::daemon::{StremioConnection, StremioLink};

    fn connection() -> StremioConnection {
        StremioConnection {
            linked_at: "2026-10-07T10:00:00Z".to_owned(),
        }
    }

    fn code() -> Pending {
        Pending {
            code: "ABCD".to_owned(),
            link: "https://link.stremio.com/#/ABCD".to_owned(),
            seconds_left: 120,
        }
    }

    /// A started link shows its code and queues the first poll: the code is
    /// what the user acts on, and the poll is what notices they did.
    #[test]
    fn a_started_link_shows_its_code_and_queues_the_first_poll() {
        let mut flow = Flow::default();
        flow.begin_link();
        assert!(
            flow.busy,
            "a request is in flight while the code is asked for"
        );

        let effect = flow.link_started(Ok(code()));

        assert!(!flow.busy, "the buttons come back when the answer lands");
        assert_eq!(
            flow.pending.as_ref().map(|pending| pending.code.as_str()),
            Some("ABCD"),
            "the code the daemon issued is the one on screen"
        );
        assert_eq!(
            flow.pending.as_ref().map(|pending| pending.seconds_left),
            Some(120)
        );
        assert_eq!(effect, Effect::Poll, "and the flow keeps watching it");
    }

    /// A request the daemon refuses says why and releases the buttons, so the
    /// section never looks stuck on a code that was never issued.
    #[test]
    fn a_refused_start_says_why_and_releases_the_buttons() {
        let mut flow = Flow::default();
        flow.begin_link();

        let effect = flow.link_started(Err("daemon is down".to_owned()));

        assert!(!flow.busy, "the buttons come back either way");
        assert_eq!(flow.error.as_deref(), Some("daemon is down"));
        assert!(flow.pending.is_none(), "no code was issued to show");
        assert_eq!(effect, Effect::Nothing);
    }

    /// While the user is still deciding, the countdown on screen follows the
    /// daemon's and the flow keeps polling.
    #[test]
    fn a_waiting_code_counts_down_and_keeps_polling() {
        let mut flow = Flow::default();
        flow.begin_link();
        let _ = flow.link_started(Ok(code()));

        let effect = flow.link_polled(Ok(StremioLink::Waiting {
            code: "ABCD".to_owned(),
            link: "https://link.stremio.com/#/ABCD".to_owned(),
            seconds_left: 98,
        }));

        assert_eq!(
            flow.pending.as_ref().map(|pending| pending.seconds_left),
            Some(98),
            "the countdown is the daemon's to say"
        );
        assert_eq!(effect, Effect::Poll);
    }

    /// Approval ends the code's job and records the account: the rows go up
    /// and the rails that hold them are re-read.
    #[test]
    fn an_approved_code_becomes_the_linked_account() {
        let mut flow = Flow::default();
        flow.begin_link();
        let _ = flow.link_started(Ok(code()));

        let effect = flow.link_polled(Ok(StremioLink::Linked {
            connection: connection(),
        }));

        assert!(flow.pending.is_none(), "the code has done its job");
        assert_eq!(
            flow.connection
                .as_ref()
                .map(|found| found.linked_at.as_str()),
            Some("2026-10-07T10:00:00Z"),
            "the account the daemon holds is the one recorded"
        );
        assert!(!flow.expired);
        assert_eq!(
            effect,
            Effect::Publish,
            "the rows go up and the rails follow"
        );
    }

    /// A code that expired before approval comes down and says so: silence
    /// would leave the user waiting on a code nothing is watching any more.
    #[test]
    fn an_expired_code_says_so_instead_of_sitting_there() {
        let mut flow = Flow::default();
        flow.begin_link();
        let _ = flow.link_started(Ok(code()));

        let effect = flow.link_polled(Ok(StremioLink::Expired));

        assert!(flow.pending.is_none(), "the dead code comes down");
        assert!(flow.expired, "and the section says what happened to it");
        assert_eq!(effect, Effect::Nothing);
    }

    /// The daemon reporting no pending link is not a verdict about the
    /// account: the connection is read again rather than guessed.
    #[test]
    fn an_idle_answer_rereads_the_connection() {
        let mut flow = Flow::default();
        flow.begin_link();
        let _ = flow.link_started(Ok(code()));

        let effect = flow.link_polled(Ok(StremioLink::Idle));

        assert!(flow.pending.is_none());
        assert_eq!(effect, Effect::ReloadConnection);
    }

    /// A poll that fails drops the code and says why: the flow ends rather
    /// than polling forever against a daemon that is not answering.
    #[test]
    fn a_failed_poll_says_why_and_drops_the_code() {
        let mut flow = Flow::default();
        flow.begin_link();
        let _ = flow.link_started(Ok(code()));

        let effect = flow.link_polled(Err("daemon is down".to_owned()));

        assert!(flow.pending.is_none(), "nothing is watching this code");
        assert_eq!(flow.error.as_deref(), Some("daemon is down"));
        assert_eq!(effect, Effect::Nothing);
    }

    /// Unlinking forgets the account and the code in one stroke, so nothing
    /// about the old session is left to draw.
    #[test]
    fn unlinking_forgets_the_account_and_the_code() {
        let mut flow = Flow {
            connection: Some(connection()),
            pending: Some(code()),
            error: Some("stale".to_owned()),
            expired: true,
            ..Flow::default()
        };
        flow.begin_unlink();
        assert!(flow.busy, "the request is in flight until it is answered");

        let effect = flow.unlinked(Ok(()));

        assert!(!flow.busy);
        assert!(flow.connection.is_none(), "the account is gone");
        assert!(flow.pending.is_none(), "and so is any code");
        assert!(flow.error.is_none(), "nothing failed");
        assert!(!flow.expired);
        assert_eq!(effect, Effect::Nothing);
    }

    /// A refused unlink keeps the account and says why: a failed request is
    /// never a reason to tell the user their account is gone.
    #[test]
    fn a_refused_unlink_keeps_the_account_and_says_why() {
        let mut flow = Flow {
            connection: Some(connection()),
            ..Flow::default()
        };

        let effect = flow.unlinked(Err("daemon is down".to_owned()));

        assert!(!flow.busy);
        assert!(
            flow.connection.is_some(),
            "the account is still linked as far as anyone knows"
        );
        assert_eq!(flow.error.as_deref(), Some("daemon is down"));
        assert_eq!(effect, Effect::Nothing);
    }

    /// The section says it is checking while the connection is being read, and
    /// stops saying it the moment the answer lands.
    #[test]
    fn a_connection_read_ends_the_checking_state() {
        let mut flow = Flow::default();
        flow.begin_read();
        assert!(
            flow.reading,
            "checking, rather than claiming that nothing is linked"
        );

        flow.connection_read(Ok(Some(connection())));

        assert!(!flow.reading);
        assert_eq!(
            flow.connection
                .as_ref()
                .map(|found| found.linked_at.as_str()),
            Some("2026-10-07T10:00:00Z")
        );
    }

    /// A read that fails says why and stops checking, without inventing an
    /// answer about the account either way.
    #[test]
    fn a_failed_connection_read_says_why_without_guessing() {
        let mut flow = Flow::default();
        flow.begin_read();

        flow.connection_read(Err("daemon is down".to_owned()));

        assert!(!flow.reading);
        assert!(flow.connection.is_none(), "nothing was learned");
        assert_eq!(flow.error.as_deref(), Some("daemon is down"));
    }

    /// Linking is always offered - it is how a revoked session is repaired -
    /// and unlinking only when there is an account to forget.
    #[test]
    fn linking_is_offered_even_when_an_account_is_already_linked() {
        assert_eq!(actions(&Flow::default()), vec![Action::Link]);
        let flow = Flow {
            connection: Some(connection()),
            ..Flow::default()
        };
        assert_eq!(actions(&flow), vec![Action::Link, Action::Unlink]);
    }

    /// Nothing is pressable while a request is in flight: a second press
    /// mid-request is how a link gets asked for twice.
    #[test]
    fn nothing_is_pressable_while_a_request_is_in_flight() {
        let mut flow = Flow {
            connection: Some(connection()),
            ..Flow::default()
        };
        assert!(
            pressable(Action::Link, &flow),
            "idle, linking is the way in"
        );
        assert!(pressable(Action::Unlink, &flow));

        flow.begin_link();

        assert!(!pressable(Action::Link, &flow), "busy: nothing presses");
        assert!(!pressable(Action::Unlink, &flow));
    }

    /// An unlinked account has nothing to forget, so Unlink is not pressable
    /// even though the row is drawn for the linked case only.
    #[test]
    fn an_unlinked_account_cannot_unlink() {
        let flow = Flow::default();
        assert!(pressable(Action::Link, &flow));
        assert!(
            !pressable(Action::Unlink, &flow),
            "there is nothing to forget"
        );
    }

    /// The expiry news is older than any fresh answer: once the daemon has
    /// spoken about the account again, the code the user lost is no longer
    /// what this section is about.
    #[test]
    fn a_successful_connection_read_retires_the_expired_news() {
        let mut flow = Flow {
            expired: true,
            ..Flow::default()
        };
        flow.begin_read();

        flow.connection_read(Ok(None));

        assert!(!flow.expired, "the read is newer news than the expiry");
    }
}
