use std::collections::HashMap;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use evdev::{AbsoluteAxisCode as Axis, Device, EventSummary, KeyCode};
use hearthdeck_protocol::input::{
    Action, Delivery, Event, Hello, QUEUE, Role, SessionContext, transport,
};
use hearthdeck_protocol::{InputMode, InputProfile, paths};
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::linux::{self, AxisRange};
use crate::mapping::{Control, Mapper};
use crate::ownership::{Owner, Ownership, Signal};

struct Pad {
    id: u64,
    path: PathBuf,
    device: Device,
    ranges: HashMap<Axis, AxisRange>,
}

struct Client {
    id: u64,
    hello: Hello,
    events: mpsc::Sender<Event>,
    _cancel: oneshot::Sender<()>,
}

enum Connection {
    Open(Client),
    Closed(u64),
}

struct Broker {
    pads: Vec<Pad>,
    clients: Vec<Client>,
    state: Ownership,
    mapper: Mapper,
    output: evdev::uinput::VirtualDevice,
    mode: InputMode,
    session: SessionContext,
    session_at: Instant,
    focused: Option<String>,
    capture: Option<u64>,
    keyboard: Option<tokio::process::Child>,
    next_pad: u64,
    started: Instant,
    mouse_at: Instant,
}

pub(crate) async fn run() -> Result<()> {
    linux::init_logging();
    let path = transport::socket_path().context("XDG_RUNTIME_DIR is not set")?;
    // A second broker must not unlink a live listener.
    anyhow::ensure!(
        UnixStream::connect(&path).await.is_err(),
        "input broker already running"
    );
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let mode_path = paths::input_socket_path().context("XDG_RUNTIME_DIR is not set")?;
    let mode_socket = linux::bind_control_socket(&mode_path)?;
    let mut broker = Broker {
        pads: Vec::new(),
        clients: Vec::new(),
        state: Ownership::default(),
        mapper: Mapper::default(),
        output: linux::create_virtual_device()?,
        mode: InputMode::default(),
        capture: None,
        keyboard: None,
        next_pad: 1,
        session: SessionContext::default(),
        session_at: Instant::now(),
        focused: None,
        started: Instant::now(),
        mouse_at: Instant::now(),
    };
    let (tx, mut rx) = mpsc::channel(QUEUE);
    let mut connections = tokio::task::JoinSet::new();
    let mut poll = tokio::time::interval(Duration::from_millis(8));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut scan = tokio::time::interval(Duration::from_secs(2));
    let mut command = [0; hearthdeck_protocol::input::MAX_FRAME + 1];
    let mut focus = crate::focus::monitor();
    let mut next_client = 1;
    info!(?path, "semantic input broker ready");
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (socket, _) = accepted?;
                if connections.len() >= 8 { continue; }
                let id = next_client;
                next_client += 1;
                connections.spawn(connection(socket, id, tx.clone()));
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result { warn!(%error, "input client task failed"); }
            }
            Some(message) = rx.recv() => {
                match message {
                    Connection::Open(client) => broker.connect(client)?,
                    Connection::Closed(id) => broker.disconnect(id)?,
                }
            }
            _ = scan.tick() => broker.scan()?,
            _ = poll.tick() => broker.poll()?,
            changed = focus.changed() => {
                if changed.is_ok() {
                    broker.focused = focus.borrow_and_update().clone();
                    broker.reconcile()?;
                }
            }
            received = mode_socket.recv(&mut command) => {
                let length = received?;
                if length <= hearthdeck_protocol::input::MAX_FRAME
                    && let Ok(session) = serde_json::from_slice::<SessionContext>(&command[..length])
                {
                    broker.mode = session.mode;
                    broker.session = session;
                    broker.session_at = Instant::now();
                    broker.reconcile()?;
                } else { warn!("invalid broker session context; bridge must use the same input mode"); }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    broker.release()?;
    linux::emit(&mut broker.output, broker.mapper.set_active(false))?;
    std::fs::remove_file(path)?;
    std::fs::remove_file(mode_path)?;
    Ok(())
}

async fn connection(mut socket: UnixStream, id: u64, tx: mpsc::Sender<Connection>) {
    let result = async {
        let uid = std::fs::metadata("/proc/self")?.uid();
        if socket.peer_cred()?.uid() != uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "different user",
            ));
        }
        let hello: Hello =
            tokio::time::timeout(Duration::from_secs(2), transport::read(&mut socket))
                .await
                .map_err(io::Error::other)??;
        if !hello.valid() {
            transport::write(
                &mut socket,
                &Event::Failed("invalid input role or version".into()),
            )
            .await?;
            return Ok(());
        }
        let (events, mut rx) = mpsc::channel(QUEUE);
        let (cancel, mut cancelled) = oneshot::channel();
        tx.send(Connection::Open(Client {
            id,
            hello,
            events,
            _cancel: cancel,
        }))
        .await
        .map_err(io::Error::other)?;
        let (mut reader, mut writer) = socket.split();
        let mut byte = [0];
        loop {
            tokio::select! {
                biased;
                _ = &mut cancelled => break,
                _ = reader.read(&mut byte) => break,
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    transport::write(&mut writer, &event).await?;
                }
            }
        }
        Ok::<(), io::Error>(())
    }
    .await;
    if let Err(error) = result {
        warn!(%error, id, "input client disconnected");
    }
    if let Err(error) = tx.send(Connection::Closed(id)).await {
        warn!(%error, "broker stopped before client cleanup");
    }
}

impl Broker {
    fn connect(&mut self, client: Client) -> Result<()> {
        if self
            .clients
            .iter()
            .any(|other| other.hello.role == client.hello.role)
        {
            // Replace a closing subscription without replaying its queued actions.
            let ids = self
                .clients
                .iter()
                .filter(|other| other.hello.role == client.hello.role)
                .map(|other| other.id)
                .collect::<Vec<_>>();
            for id in ids {
                self.disconnect(id)?;
            }
        }
        let id = client.id;
        let capture = client.hello.delivery == Delivery::Overlay;
        self.clients.push(client);
        if capture {
            linux::emit(&mut self.output, self.mapper.set_active(false))?;
            self.state.suspend();
            if self.keyboard.is_some()
                || self
                    .clients
                    .iter()
                    .any(|client| client.hello.delivery == Delivery::TextEntry)
            {
                self.send(
                    id,
                    Event::Failed("Dismiss the keyboard before opening the overlay".into()),
                )?;
                return Ok(());
            }
            if let Some(error) = grab_all(
                &mut self.pads,
                |pad| pad.device.grab(),
                |pad| pad.device.ungrab(),
            )? {
                warn!(%error, "overlay capture denied; partial grabs rolled back");
                self.send(
                    id,
                    Event::Failed(format!("Controller capture failed: {error}")),
                )?;
                self.reconcile()?;
                return Ok(());
            }
            self.capture = Some(id);
        }
        self.reconcile()?;
        self.send(id, Event::Ready)
    }

    fn send(&mut self, id: u64, event: Event) -> Result<()> {
        if let Some(client) = self.clients.iter().find(|client| client.id == id)
            && client.events.try_send(event).is_err()
        {
            warn!(id, "slow input consumer disconnected; capture revoked");
            self.disconnect(id)?;
        }
        Ok(())
    }

    fn disconnect(&mut self, id: u64) -> Result<()> {
        self.clients.retain(|client| client.id != id);
        if self.capture == Some(id) {
            self.release()?;
        }
        self.reconcile()
    }

    fn release(&mut self) -> Result<()> {
        if self.capture.take().is_some() {
            for pad in &mut self.pads {
                pad.device
                    .ungrab()
                    .context("failed to release capture; exiting to close device descriptors")?;
            }
        }
        Ok(())
    }

    fn revoke(&mut self, reason: String) -> Result<()> {
        let owner = self.capture;
        self.release()?;
        self.reconcile()?;
        if let Some(id) = owner {
            self.send(id, Event::Failed(reason))?;
        }
        Ok(())
    }

    fn reconcile(&mut self) -> Result<()> {
        let frontend = self
            .clients
            .iter()
            .find(|client| client.hello.role == Role::Frontend)
            .map(|client| client.hello.delivery);
        let owner = if self.keyboard.is_some() {
            Owner::Keyboard
        } else if self.capture.is_some() {
            Owner::Overlay
        } else {
            match frontend {
                Some(Delivery::Frontend) => Owner::Frontend,
                Some(Delivery::TextEntry) => Owner::TextEntry,
                Some(Delivery::Handoff) => Owner::Observe,
                _ if self.mode.profile == InputProfile::Desktop && self.session_has_focus() => {
                    Owner::Desktop
                }
                _ => Owner::Observe,
            }
        };
        if owner != self.state.owner {
            linux::emit(&mut self.output, self.mapper.set_active(false))?;
            self.state.set_owner(owner);
            info!(
                ?owner,
                "controller delivery changed; waiting for per-device neutral"
            );
        }
        linux::emit(
            &mut self.output,
            self.mapper.set_active(owner == Owner::Desktop),
        )?;
        Ok(())
    }

    fn scan(&mut self) -> Result<()> {
        if self.keyboard.is_some() {
            return Ok(());
        }
        for (path, mut device) in linux::gamepad_devices() {
            if self.pads.iter().any(|pad| pad.path == path) {
                continue;
            }
            device.set_nonblocking(true)?;
            if self.capture.is_some()
                && let Err(error) = device.grab()
            {
                self.revoke(format!("New controller could not be captured: {error}"))?;
            }
            let id = self.next_pad;
            self.next_pad += 1;
            let ranges = device
                .get_absinfo()?
                .map(|(axis, info)| (axis, AxisRange::new(info.minimum(), info.maximum())))
                .collect();
            let mut pad = Pad {
                id,
                path,
                device,
                ranges,
            };
            // Seed held state from the kernel; opening a held pad must not arm it.
            let keys = pad.device.get_key_state()?;
            for key in keys.iter() {
                if let Some(control) = key_control(key) {
                    self.state.button(id, control, true, 0, false);
                }
            }
            let axes = pad
                .device
                .get_absinfo()?
                .map(|(code, info)| (code, info.value()))
                .collect::<Vec<_>>();
            for (code, value) in axes {
                for (control, pressed) in axis_controls(&pad, code, value) {
                    self.state.button(id, control, pressed, 0, false);
                }
            }
            self.state.button(
                id,
                Control::Guide,
                keys.contains(KeyCode::BTN_MODE),
                0,
                false,
            );
            // Discard pre-snapshot events; only subsequent edges are eligible.
            match pad.device.fetch_events() {
                Ok(events) => {
                    events.for_each(drop);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            self.pads.push(pad);
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<()> {
        self.reconcile()?;
        if let Some(child) = &mut self.keyboard {
            if let Some(status) = child.try_wait()? {
                info!(
                    ?status,
                    "keyboard adapter exited; returning controller ownership"
                );
                self.keyboard = None;
                self.reconcile()?;
                self.scan()?;
            }
            return Ok(());
        }
        let now = u64::try_from(self.started.elapsed().as_millis())?;
        let keyboard_enabled = self.mode.retro_osk && self.session_has_focus();
        let mut signals = Vec::new();
        let mut disconnected = Vec::new();
        for pad in &mut self.pads {
            let events = match pad.device.fetch_events() {
                Ok(events) => events.collect::<Vec<_>>(),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) => {
                    warn!(%error, path = ?pad.path, "controller disconnected");
                    disconnected.push(pad.id);
                    continue;
                }
            };
            for event in events {
                let armed = self.state.armed(pad.id);
                match event.destructure() {
                    EventSummary::Key(_, code, value) if value != 2 => {
                        if let Some(control) = key_control(code) {
                            signals.extend(self.state.button(
                                pad.id,
                                control,
                                value != 0,
                                now,
                                keyboard_enabled,
                            ));
                        }
                        if code == KeyCode::BTN_MODE && value == 1 {
                            linux::emit(&mut self.output, self.mapper.remove_device(pad.id))?;
                        }
                        if armed && !self.state.guide_held(pad.id) {
                            linux::emit(
                                &mut self.output,
                                linux::map_key(&mut self.mapper, pad.id, code, value),
                            )?;
                        }
                    }
                    EventSummary::AbsoluteAxis(_, code, value) => {
                        for (control, pressed) in axis_controls(pad, code, value) {
                            signals.extend(self.state.button(
                                pad.id,
                                control,
                                pressed,
                                now,
                                keyboard_enabled,
                            ));
                        }
                        if armed && !self.state.guide_held(pad.id) {
                            linux::emit(
                                &mut self.output,
                                linux::map_axis(
                                    &mut self.mapper,
                                    pad.id,
                                    code,
                                    value,
                                    pad.ranges.get(&code).copied(),
                                ),
                            )?;
                        }
                    }
                    _ => {}
                }
            }
        }
        for id in disconnected {
            self.pads.retain(|pad| pad.id != id);
            self.state.remove(id);
            linux::emit(&mut self.output, self.mapper.remove_device(id))?;
        }
        signals.extend(self.state.tick(now));
        for signal in signals {
            let owner = self.state.owner;
            self.deliver(signal)?;
            if self.state.owner != owner {
                break;
            }
        }
        if self.mouse_at.elapsed() >= Duration::from_millis(16) {
            self.mouse_at = Instant::now();
            if let Some(event) = self.mapper.mouse_tick() {
                linux::emit(&mut self.output, [event])?;
            }
        }
        Ok(())
    }

    fn session_has_focus(&self) -> bool {
        self.session_at.elapsed() < Duration::from_secs(5)
            && self.session.matches_focus(self.focused.as_deref())
    }

    fn deliver(&mut self, signal: Signal) -> Result<()> {
        match signal {
            Signal::Keyboard => self.open_keyboard(),
            Signal::Action(action) => {
                let role = if action == Action::ToggleOverlay || self.capture.is_some() {
                    Role::Overlay
                } else {
                    Role::Frontend
                };
                if let Some(id) = self
                    .clients
                    .iter()
                    .find(|client| client.hello.role == role)
                    .map(|client| client.id)
                {
                    self.send(id, Event::Action(action))?;
                }
                Ok(())
            }
        }
    }

    fn open_keyboard(&mut self) -> Result<()> {
        let result = (|| {
            let program = std::env::var_os("HEARTHDECK_GAMEPAD_OSK_ADAPTER")
                .context("Game keyboard unavailable: set a verified foreground HEARTHDECK_GAMEPAD_OSK_ADAPTER")?;
            linux::emit(&mut self.output, self.mapper.set_active(false))?;
            self.release()?;
            // The keyboard adapter may read controllers itself. Close our descriptors
            // for its entire foreground lifetime instead of pretending it is semantic.
            for pad in self.pads.drain(..) {
                self.state.remove(pad.id);
            }
            let child = tokio::process::Command::new(program)
                .kill_on_drop(true)
                .spawn()?;
            self.keyboard = Some(child);
            self.reconcile()
        })();
        if let Err(error) = result {
            warn!(%error, "keyboard handoff failed");
            let ids = self
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>();
            for id in ids {
                self.send(id, Event::Failed(error.to_string()))?;
            }
            self.reconcile()?;
        }
        Ok(())
    }
}

fn grab_all<T>(
    devices: &mut [T],
    mut grab: impl FnMut(&mut T) -> io::Result<()>,
    mut ungrab: impl FnMut(&mut T) -> io::Result<()>,
) -> Result<Option<io::Error>> {
    for index in 0..devices.len() {
        if let Err(error) = grab(&mut devices[index]) {
            for device in &mut devices[..index] {
                ungrab(device)
                    .context("capture rollback failed; broker must exit to release descriptors")?;
            }
            return Ok(Some(error));
        }
    }
    Ok(None)
}

fn key_control(code: KeyCode) -> Option<Control> {
    Some(match code {
        KeyCode::BTN_MODE => Control::Guide,
        KeyCode::BTN_SOUTH => Control::South,
        KeyCode::BTN_EAST => Control::East,
        KeyCode::BTN_WEST => Control::West,
        KeyCode::BTN_NORTH => Control::North,
        KeyCode::BTN_TL => Control::LeftBumper,
        KeyCode::BTN_TR => Control::RightBumper,
        KeyCode::BTN_START => Control::Start,
        KeyCode::BTN_SELECT => Control::Select,
        KeyCode::BTN_TL2 => Control::LeftTrigger,
        KeyCode::BTN_TR2 => Control::RightTrigger,
        KeyCode::BTN_DPAD_UP => Control::DpadUp,
        KeyCode::BTN_DPAD_DOWN => Control::DpadDown,
        KeyCode::BTN_DPAD_LEFT => Control::DpadLeft,
        KeyCode::BTN_DPAD_RIGHT => Control::DpadRight,
        _ => Control::Other(code.code()),
    })
}

fn axis_controls(pad: &Pad, code: Axis, value: i32) -> Vec<(Control, bool)> {
    let range = pad.ranges.get(&code).copied();
    let value = linux::normalize_centered(value, range);
    let (negative, positive) = match code {
        Axis::ABS_X => (Control::StickLeft, Control::StickRight),
        Axis::ABS_Y => (Control::StickUp, Control::StickDown),
        Axis::ABS_HAT0X => (Control::DpadLeft, Control::DpadRight),
        Axis::ABS_HAT0Y => (Control::DpadUp, Control::DpadDown),
        Axis::ABS_RX => (Control::PointerLeft, Control::PointerRight),
        Axis::ABS_RY => (Control::PointerUp, Control::PointerDown),
        Axis::ABS_Z | Axis::ABS_RZ => {
            let control = if code == Axis::ABS_Z {
                Control::LeftTrigger
            } else {
                Control::RightTrigger
            };
            return vec![(control, value > 0)];
        }
        _ => return Vec::new(),
    };
    vec![(negative, value < -350), (positive, value > 350)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_capture_rolls_back_every_prior_grab() {
        let mut devices = [(false, false), (false, true), (false, false)];
        assert!(
            grab_all(
                &mut devices,
                |(grabbed, fail)| {
                    if *fail {
                        return Err(io::Error::other("busy"));
                    }
                    *grabbed = true;
                    Ok(())
                },
                |(grabbed, _)| {
                    *grabbed = false;
                    Ok(())
                }
            )
            .unwrap()
            .is_some()
        );
        assert_eq!(devices, [(false, false), (false, true), (false, false)]);
    }

    #[test]
    fn rollback_failure_is_fatal_instead_of_granting_partial_capture() {
        let mut devices = [false, true];
        let result = grab_all(
            &mut devices,
            |fail| {
                if *fail {
                    Err(io::Error::other("busy"))
                } else {
                    Ok(())
                }
            },
            |_| Err(io::Error::other("ungrab failed")),
        );
        assert!(result.unwrap_err().to_string().contains("rollback failed"));
    }

    #[tokio::test]
    async fn semantic_delivery_and_peer_disconnect_reach_the_broker() {
        let (server, mut peer) = UnixStream::pair().unwrap();
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let task = tokio::spawn(connection(server, 42, tx));
        transport::write(
            &mut peer,
            &Hello {
                version: hearthdeck_protocol::input::VERSION,
                role: Role::Frontend,
                delivery: Delivery::Frontend,
            },
        )
        .await
        .unwrap();
        let Some(Connection::Open(client)) = rx.recv().await else {
            panic!("missing registration");
        };
        assert_eq!(client.id, 42);
        client.events.try_send(Event::Ready).unwrap();
        client.events.try_send(Event::Action(Action::Back)).unwrap();
        assert_eq!(
            transport::read::<Event>(&mut peer).await.unwrap(),
            Event::Ready
        );
        assert_eq!(
            transport::read::<Event>(&mut peer).await.unwrap(),
            Event::Action(Action::Back)
        );
        drop(peer);
        assert!(matches!(rx.recv().await, Some(Connection::Closed(42))));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_cancelled_full_queue_disconnects_without_replaying_its_backlog() {
        let (server, mut peer) = UnixStream::pair().unwrap();
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let task = tokio::spawn(connection(server, 7, tx));
        transport::write(
            &mut peer,
            &Hello {
                version: hearthdeck_protocol::input::VERSION,
                role: Role::Overlay,
                delivery: Delivery::Overlay,
            },
        )
        .await
        .unwrap();
        let Some(Connection::Open(client)) = rx.recv().await else {
            panic!("missing registration");
        };
        for _ in 0..QUEUE {
            client
                .events
                .try_send(Event::Action(Action::Confirm))
                .unwrap();
        }
        assert!(matches!(
            client.events.try_send(Event::Ready),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        drop(client);
        assert!(transport::read::<Event>(&mut peer).await.is_err());
        assert!(matches!(rx.recv().await, Some(Connection::Closed(7))));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn invalid_roles_are_rejected_before_registration() {
        let (server, mut peer) = UnixStream::pair().unwrap();
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let task = tokio::spawn(connection(server, 9, tx));
        transport::write(
            &mut peer,
            &Hello {
                version: hearthdeck_protocol::input::VERSION,
                role: Role::Frontend,
                delivery: Delivery::Overlay,
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            transport::read::<Event>(&mut peer).await.unwrap(),
            Event::Failed(_)
        ));
        assert!(matches!(rx.recv().await, Some(Connection::Closed(9))));
        task.await.unwrap();
    }
}
