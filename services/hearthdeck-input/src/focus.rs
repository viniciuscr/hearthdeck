//! COSMIC focus observation is a platform adapter, not a capture grant.
use cosmic_client_toolkit::toplevel_info::{ToplevelInfoHandler, ToplevelInfoState};
use cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State;
use sctk::output::{OutputHandler, OutputState};
use sctk::registry::{ProvidesRegistryState, RegistryState};
use tokio::sync::watch;
use wayland_client::{Connection, QueueHandle, globals::registry_queue_init, protocol::wl_output};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;

pub(crate) fn monitor() -> watch::Receiver<Option<String>> {
    let (tx, rx) = watch::channel(None);
    std::thread::spawn(move || {
        loop {
            if let Err(error) = run(tx.clone()) {
                tracing::warn!(%error, "COSMIC focus unavailable; compatibility and game keyboard disabled");
            }
            tx.send_replace(None);
            if tx.is_closed() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
    rx
}

struct Focus {
    output: OutputState,
    registry: RegistryState,
    toplevels: ToplevelInfoState,
    tx: watch::Sender<Option<String>>,
}

fn run(tx: watch::Sender<Option<String>>) -> anyhow::Result<()> {
    let connection = connect()?;
    let (globals, mut queue) = registry_queue_init(&connection)?;
    let qh = queue.handle();
    let registry = RegistryState::new(&globals);
    let mut focus = Focus {
        output: OutputState::new(&globals, &qh),
        toplevels: ToplevelInfoState::new(&registry, &qh),
        registry,
        tx,
    };
    loop {
        queue.blocking_dispatch(&mut focus)?;
        let active = focus
            .toplevels
            .toplevels()
            .find(|window| window.state.contains(&State::Activated))
            .map(|window| window.app_id.clone());
        focus.tx.send_if_modified(|previous| {
            if *previous == active {
                return false;
            }
            *previous = active;
            true
        });
        if focus.tx.is_closed() {
            return Ok(());
        }
    }

    fn connect() -> anyhow::Result<Connection> {
        if let Ok(connection) = Connection::connect_to_env() {
            return Ok(connection);
        }
        // The target starts before cosmic-comp publishes its socket to systemd.
        let environment = std::process::Command::new("systemctl")
            .args(["--user", "show-environment"])
            .output()?;
        anyhow::ensure!(
            environment.status.success(),
            "could not read compositor activation environment"
        );
        let environment = String::from_utf8(environment.stdout)?;
        let display = environment
            .lines()
            .find_map(|line| line.strip_prefix("WAYLAND_DISPLAY="))
            .ok_or_else(|| anyhow::anyhow!("compositor has not published WAYLAND_DISPLAY"))?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is not set"))?;
        let socket =
            std::os::unix::net::UnixStream::connect(std::path::Path::new(&runtime).join(display))?;
        Ok(Connection::from_socket(socket)?)
    }
}

impl ProvidesRegistryState for Focus {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    sctk::registry_handlers!(OutputState);
}

impl OutputHandler for Focus {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ToplevelInfoHandler for Focus {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevels
    }
    fn new_toplevel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
    }
    fn update_toplevel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
    }
    fn toplevel_closed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
    }
    fn info_done(&mut self, _: &Connection, _: &QueueHandle<Self>) {}
}

sctk::delegate_output!(Focus);
sctk::delegate_registry!(Focus);
cosmic_client_toolkit::delegate_toplevel_info!(Focus);
