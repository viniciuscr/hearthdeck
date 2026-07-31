mod bridge;
mod control;
mod input;

use std::{num::NonZeroU32, time::Duration};

use anyhow::{Context, Result};
use smithay_client_toolkit::reexports::client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};

use crate::linux::{
    bridge::ManagedSession,
    control::{ControlListener, OverlayCommand},
    input::{GamepadAction, InputHandle},
};

const OVERLAY_NAMESPACE: &str = "io.github.viniciuscr.hearthdeck.overlay";
const POLL_INTERVAL: Duration = Duration::from_millis(8);

pub fn run() -> Result<()> {
    hearthdeck_observability::init("hearthdeck-overlay", "hearthdeck_overlay=info");
    if let Some(command) = control::command_from_args()? {
        return control::send(command);
    }

    let connection = Connection::connect_to_env()
        .context("could not connect the overlay to the exposed Gamescope Wayland socket")?;
    let (globals, mut event_queue) = registry_queue_init(&connection)?;
    let queue_handle = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &queue_handle)
        .context("Gamescope does not expose wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &queue_handle)
        .context("Gamescope does not expose wlr-layer-shell")?;
    let shm = Shm::bind(&globals, &queue_handle).context("Gamescope does not expose wl_shm")?;
    // Kept alive and reused for every layer surface the overlay creates; a
    // wl_region isn't tied to a single surface.
    let input_region = Region::new(&compositor)?;
    connection.flush()?;
    let pool = SlotPool::new(1, &shm)?;

    let mut overlay = Overlay {
        control: ControlListener::bind()?,
        gamepad: input::start(),
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &queue_handle),
        queue_handle: queue_handle.clone(),
        compositor,
        layer_shell,
        shm,
        _input_region: input_region,
        layer: None,
        pool,
        width: 0,
        height: 0,
        visible: false,
        selection: Selection::Resume,
        active_session: None,
        message: None,
        needs_redraw: false,
        closed: false,
    };

    while !overlay.closed {
        event_queue.dispatch_pending(&mut overlay)?;
        overlay.process_commands();
        overlay.redraw_if_needed(&queue_handle)?;
        connection.flush()?;
        // Prepare the read before waiting. Dropping the guard on timeout is
        // intentional: it cancels the read without blocking controller input.
        if let Some(guard) = connection.prepare_read() {
            let socket = guard.connection_fd();
            let mut descriptors = [rustix::event::PollFd::new(
                &socket,
                rustix::event::PollFlags::IN | rustix::event::PollFlags::ERR,
            )];
            let timeout = rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: POLL_INTERVAL.as_nanos() as _,
            };
            if rustix::event::poll(&mut descriptors, Some(&timeout))? > 0 {
                guard.read()?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Selection {
    Resume,
    CloseGame,
}

impl Selection {
    fn toggle(self) -> Self {
        match self {
            Self::Resume => Self::CloseGame,
            Self::CloseGame => Self::Resume,
        }
    }
}

struct Overlay {
    control: ControlListener,
    gamepad: InputHandle,
    registry_state: RegistryState,
    output_state: OutputState,
    queue_handle: QueueHandle<Self>,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    // Retaining this empty region keeps every surface created from it mouse
    // and touch click-through; a wl_region isn't tied to one surface.
    _input_region: Region,
    // None while hidden: the surface is fully destroyed rather than merely
    // unmapped (see `hide`), so there is nothing to hold onto between shows.
    layer: Option<LayerSurface>,
    pool: SlotPool,
    width: u32,
    height: u32,
    visible: bool,
    selection: Selection,
    active_session: Option<ManagedSession>,
    message: Option<String>,
    needs_redraw: bool,
    closed: bool,
}

impl Overlay {
    fn process_commands(&mut self) {
        for command in self.control.drain() {
            self.handle_control(command);
        }
        while let Ok(action) = self.gamepad.actions.try_recv() {
            self.handle_gamepad(action);
        }
    }

    fn handle_control(&mut self, command: OverlayCommand) {
        match command {
            OverlayCommand::Toggle if self.visible => self.hide(),
            OverlayCommand::Toggle | OverlayCommand::Show => self.show(),
            OverlayCommand::Hide => self.hide(),
        }
    }

    fn handle_gamepad(&mut self, action: GamepadAction) {
        match action {
            GamepadAction::Toggle => self.handle_control(OverlayCommand::Toggle),
            GamepadAction::Hide => self.hide(),
            GamepadAction::ToggleSelection => {
                self.selection = self.selection.toggle();
                self.needs_redraw = true;
            }
            GamepadAction::Activate => match self.selection {
                Selection::Resume => self.hide(),
                Selection::CloseGame => {
                    self.hide();
                    if let Err(error) = bridge::stop_active_session() {
                        self.message = Some(error.to_string());
                    }
                }
            },
        }
    }

    fn show(&mut self) {
        self.visible = true;
        self.message = None;
        self.selection = Selection::Resume;
        self.active_session = match bridge::active_session() {
            Ok(session) => session,
            Err(error) => {
                self.message = Some(error.to_string());
                None
            }
        };
        self.gamepad.set_visible(true);
        self.create_layer();
    }

    // Creates a fresh layer surface for this show(). The compositor will send
    // a `configure` event with the assigned size, which is what actually
    // triggers the first redraw (see `configure` below).
    fn create_layer(&mut self) {
        if self.layer.is_some() {
            return;
        }
        let surface = self.compositor.create_surface(&self.queue_handle);
        let layer = self.layer_shell.create_layer_surface(
            &self.queue_handle,
            surface,
            Layer::Overlay,
            Some(OVERLAY_NAMESPACE),
            None,
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer
            .wl_surface()
            .set_input_region(Some(self._input_region.wl_region()));
        layer.commit();
        self.width = 0;
        self.height = 0;
        self.layer = Some(layer);
    }

    // Destroying the layer surface (rather than attaching a null buffer and
    // committing) guarantees the compositor stops displaying it: that is
    // dictated by the wl_surface/wlr-layer-shell protocol itself, not by a
    // compositor's own interpretation of a null-buffer commit. Gamescope's
    // layer-shell support does not reliably unmap a surface on a null-buffer
    // commit, which previously left the overlay's last frame on screen
    // indefinitely. Recreating the surface on the next `show` costs one
    // extra `configure` round trip, which is not perceptible.
    fn hide(&mut self) {
        self.visible = false;
        self.active_session = None;
        self.gamepad.set_visible(false);
        self.layer = None;
        self.width = 0;
        self.height = 0;
        self.needs_redraw = false;
    }

    fn redraw_if_needed(&mut self, queue_handle: &QueueHandle<Self>) -> Result<()> {
        if !self.visible || !self.needs_redraw || self.width == 0 || self.height == 0 {
            return Ok(());
        }
        let Some(layer) = self.layer.as_ref() else {
            return Ok(());
        };
        let width = self.width;
        let height = self.height;
        let stride = width as i32 * 4;
        let (buffer, canvas) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        )?;
        render(
            canvas,
            width,
            height,
            self.selection,
            self.active_session.as_ref(),
            self.message.as_deref(),
        );
        layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        layer
            .wl_surface()
            .frame(queue_handle, FrameCallbackData(layer.wl_surface().clone()));
        buffer.attach_to(layer.wl_surface())?;
        layer.commit();
        self.needs_redraw = false;
        Ok(())
    }
}

impl CompositorHandler for Overlay {
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _scale_factor: i32,
    ) {
        self.needs_redraw = true;
    }

    fn transform_changed(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _transform: wl_output::Transform,
    ) {
        self.needs_redraw = true;
    }

    fn frame(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Overlay {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for Overlay {
    fn closed(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _layer: &LayerSurface,
    ) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // Stale configure from a surface already destroyed by a `hide` that
        // raced this event: nothing to redraw onto.
        if self.layer.is_none() {
            return;
        }
        self.width = NonZeroU32::new(configure.new_size.0).map_or(1280, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(720, NonZeroU32::get);
        self.needs_redraw = self.visible;
    }
}

impl ShmHandler for Overlay {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Overlay);

impl ProvidesRegistryState for Overlay {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

smithay_client_toolkit::delegate_dispatch2!(Overlay);

fn render(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    selection: Selection,
    session: Option<&ManagedSession>,
    message: Option<&str>,
) {
    fill(
        canvas,
        width,
        height,
        0,
        0,
        width,
        height,
        Color::new(0xA8, 0x00, 0x00, 0x00),
    );
    let panel_width = width.saturating_sub(48).min(760);
    let panel_height = 340.min(height.saturating_sub(48));
    let panel_x = (width.saturating_sub(panel_width)) / 2;
    let panel_y = (height.saturating_sub(panel_height)) / 2;
    fill(
        canvas,
        width,
        height,
        panel_x,
        panel_y,
        panel_width,
        panel_height,
        Color::new(0xF5, 0x17, 0x1A, 0x22),
    );
    stroke(
        canvas,
        width,
        height,
        panel_x,
        panel_y,
        panel_width,
        panel_height,
        2,
        Color::new(0xFF, 0x6C, 0xCF, 0xA2),
    );
    let scale = (panel_width / 180).clamp(3, 5);
    let text_x = panel_x + 32;
    let mut text_y = panel_y + 30;
    text(
        canvas,
        width,
        height,
        text_x,
        text_y,
        scale,
        "HEARTHDECK",
        Color::new(0xFF, 0xA9, 0xF4, 0xD3),
    );
    text_y += scale * 12;
    text(
        canvas,
        width,
        height,
        text_x,
        text_y,
        scale,
        "GAME OVERLAY",
        Color::new(0xFF, 0xFF, 0xFF, 0xFF),
    );
    text_y += scale * 15;
    text(
        canvas,
        width,
        height,
        text_x,
        text_y,
        scale,
        if session.is_some() {
            "MANAGED GAME ACTIVE"
        } else {
            "NO MANAGED GAME ACTIVE"
        },
        Color::new(0xFF, 0xC3, 0xCC, 0xDB),
    );
    if let Some(session) = session {
        text_y += scale * 12;
        text(
            canvas,
            width,
            height,
            text_x,
            text_y,
            scale.saturating_sub(1).max(2),
            &session.application_id,
            Color::new(0xFF, 0xA7, 0xB0, 0xC0),
        );
    }
    text_y += scale * 19;
    option(
        canvas,
        width,
        height,
        text_x,
        text_y,
        panel_width.saturating_sub(64),
        scale,
        selection == Selection::Resume,
        "RESUME GAME",
    );
    text_y += scale * 15;
    option(
        canvas,
        width,
        height,
        text_x,
        text_y,
        panel_width.saturating_sub(64),
        scale,
        selection == Selection::CloseGame,
        "CLOSE GAME",
    );
    text_y += scale * 19;
    text(
        canvas,
        width,
        height,
        text_x,
        text_y,
        scale.saturating_sub(1).max(2),
        message.unwrap_or("DPAD SELECT   A CONFIRM   B CLOSE"),
        Color::new(0xFF, 0xA7, 0xB0, 0xC0),
    );
}

#[allow(clippy::too_many_arguments)]
fn option(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    option_width: u32,
    scale: u32,
    selected: bool,
    label: &str,
) {
    let option_height = scale * 11;
    let color = if selected {
        Color::new(0xFF, 0x3E, 0xA6, 0x7A)
    } else {
        Color::new(0xFF, 0x2B, 0x30, 0x3C)
    };
    fill(
        canvas,
        width,
        height,
        x,
        y,
        option_width,
        option_height,
        color,
    );
    let label_x = if selected {
        text(
            canvas,
            width,
            height,
            x + scale * 3,
            y + scale * 2,
            scale,
            ">",
            Color::new(0xFF, 0xFF, 0xFF, 0xFF),
        );
        x + scale * 10
    } else {
        x + scale * 3
    };
    text(
        canvas,
        width,
        height,
        label_x,
        y + scale * 2,
        scale,
        label,
        Color::new(0xFF, 0xFF, 0xFF, 0xFF),
    );
}

#[derive(Clone, Copy)]
struct Color {
    alpha: u8,
    red: u8,
    green: u8,
    blue: u8,
}

impl Color {
    const fn new(alpha: u8, red: u8, green: u8, blue: u8) -> Self {
        Self {
            alpha,
            red,
            green,
            blue,
        }
    }

    fn bytes(self) -> [u8; 4] {
        [self.blue, self.green, self.red, self.alpha]
    }
}

#[allow(clippy::too_many_arguments)]
fn fill(canvas: &mut [u8], width: u32, height: u32, x: u32, y: u32, w: u32, h: u32, color: Color) {
    let x_end = x.saturating_add(w).min(width);
    let y_end = y.saturating_add(h).min(height);
    let bytes = color.bytes();
    for row in y..y_end {
        for column in x..x_end {
            let index = ((row * width + column) * 4) as usize;
            canvas[index..index + 4].copy_from_slice(&bytes);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    line: u32,
    color: Color,
) {
    fill(canvas, width, height, x, y, w, line, color);
    fill(
        canvas,
        width,
        height,
        x,
        y.saturating_add(h).saturating_sub(line),
        w,
        line,
        color,
    );
    fill(canvas, width, height, x, y, line, h, color);
    fill(
        canvas,
        width,
        height,
        x.saturating_add(w).saturating_sub(line),
        y,
        line,
        h,
        color,
    );
}

#[allow(clippy::too_many_arguments)]
fn text(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    mut x: u32,
    y: u32,
    scale: u32,
    value: &str,
    color: Color,
) {
    for character in value.chars() {
        let glyph = glyph(character);
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    fill(
                        canvas,
                        width,
                        height,
                        x + column * scale,
                        y + row as u32 * scale,
                        scale,
                        scale,
                        color,
                    );
                }
            }
        }
        x += scale * 6;
    }
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10011, 0b10101, 0b10101, 0b10101, 0b11001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b11100,
        ],
        '>' => [
            0b10000, 0b01000, 0b00100, 0b00010, 0b00100, 0b01000, 0b10000,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '/' => [
            0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b00000, 0b00000,
        ],
        _ => [0; 7],
    }
}
