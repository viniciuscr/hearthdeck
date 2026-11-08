//! The controller, as the library app forwards it.
//!
//! The broker gives the pad to **one** process - the library app - so this
//! window does not open a second controller reader: it would evict the library
//! app's connection (the broker keeps one consumer per role) and it would mean a
//! second navigation vocabulary. Instead the library app forwards the semantic
//! action it already received, over a unix socket it names in the environment
//! when it launches this window.
//!
//! Each line is turned into the *same* gpui action the keyboard is bound to, so
//! there is no navigation logic here at all: the pad and the keys meet at the
//! action, exactly as they do in the library app.
//!
//! A window with no socket is fully usable from a keyboard - the pad is an
//! addition, never a dependency.

use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

/// One semantic action the pad can ask for. It is the same set the window binds
/// to keys: move, confirm, back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
}

/// What one forwarded line means. An unknown word is ignored rather than
/// treated as an error: a newer library app must not break this window.
pub fn parse(line: &str) -> Option<Remote> {
    Some(match line.trim() {
        "up" => Remote::Up,
        "down" => Remote::Down,
        "left" => Remote::Left,
        "right" => Remote::Right,
        "confirm" => Remote::Confirm,
        "back" => Remote::Back,
        _ => return None,
    })
}

/// The socket the library app connects to. It names it in the environment when
/// it launches this window; the fallback keeps a hand-started window usable.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HEARTHDECK_SETTINGS_INPUT") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("hearthdeck-settings-input.sock"))
}

/// Listens for the library app's forwarded actions. `None` when there is no
/// socket to open - the window still runs, it just has no pad.
pub fn listen() -> Option<Receiver<Remote>> {
    let path = socket_path()?;
    listen_on(&path)
        .inspect_err(|error| {
            tracing::warn!(%error, path = %path.display(), "the input socket could not be bound");
        })
        .ok()
}

/// The listening half, with the path named: tests bind their own.
fn listen_on(path: &Path) -> std::io::Result<Receiver<Remote>> {
    // A window that was killed leaves its socket behind; the next one owns it.
    let _ = std::fs::remove_file(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(path)?;
    let (sender, receiver) = channel();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            read_actions(stream, &sender);
        }
        let _ = std::fs::remove_file(&owned);
    });
    Ok(receiver)
}

/// Reads one connection until it closes, then returns to the accept loop, so a
/// library app that restarts can connect again.
fn read_actions(stream: UnixStream, sender: &Sender<Remote>) {
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let Some(remote) = parse(&line) else {
            continue;
        };
        if sender.send(remote).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Remote, listen_on, parse};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    #[test]
    fn the_words_the_library_app_sends_are_the_ones_this_window_answers() {
        assert_eq!(parse("up"), Some(Remote::Up));
        assert_eq!(parse("down"), Some(Remote::Down));
        assert_eq!(parse("left"), Some(Remote::Left));
        assert_eq!(parse("right"), Some(Remote::Right));
        assert_eq!(parse("confirm"), Some(Remote::Confirm));
        assert_eq!(parse("back"), Some(Remote::Back));
        assert_eq!(
            parse("confirm\n"),
            Some(Remote::Confirm),
            "a line is a line"
        );
        assert_eq!(
            parse("search"),
            None,
            "an action this window has no use for"
        );
        assert_eq!(parse(""), None);
    }

    #[test]
    fn a_forwarded_action_reaches_the_window_in_order() {
        let path = std::env::temp_dir().join(format!(
            "hearthdeck-input-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let received = listen_on(&path).expect("a listening socket");
        let mut stream = UnixStream::connect(&path).expect("the window's socket");
        stream.write_all(b"down\nconfirm\n").expect("a write");
        drop(stream);

        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).expect("down"),
            Remote::Down
        );
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("confirm"),
            Remote::Confirm
        );
        let _ = std::fs::remove_file(&path);
    }
}
