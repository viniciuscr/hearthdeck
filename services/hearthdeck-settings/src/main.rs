//! Hearthdeck's settings window.
//!
//! It ships as its own process beside the library app, and the library app is
//! what opens it: gpui renders through its own window and event loop, so it
//! cannot be mounted inside a libcosmic/iced `Element`. The divider is drawn
//! where it is cheap to hold: **everything here reads; everything that writes
//! stays in the library app**, which already owns those flows.
//!
//! Which means the contract this window keeps:
//!
//! - Escape (or the pad's Back) closes it. Focus returns to the library app,
//!   which never left the page it was on — so one Back, one result.
//! - It pairs with the loopback daemon itself, exactly as the library app does,
//!   so nothing secret crosses the process boundary.
//! - It reads once, at launch. Opening settings again is the refresh.

mod daemon;

use std::ffi::CString;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, IntoElement, KeyBinding, ParentElement as _,
    Render, SharedString, Styled as _, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    point, px, size,
};
use gpui_component::button::Button;
use gpui_component::progress::ProgressCircle;
use gpui_component::{ActiveTheme as _, Theme, ThemeMode, h_flex, v_flex};
use nix::sys::statvfs::statvfs;

use daemon::{Snapshot, read_snapshot};

gpui::actions!(hearthdeck_settings, [CloseWindow]);

/// The shipped settings page's column, so the two are read the same way on a TV.
const PAGE_WIDTH: f32 = 900.0;
const WINDOW_HEIGHT: f32 = 720.0;
const RING_SIZE: f32 = 44.0;

struct DiskUsage {
    total: u64,
    available: u64,
}

impl DiskUsage {
    fn used_percent(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let used = self.total.saturating_sub(self.available);
        (used as f64 / self.total as f64 * 100.0) as f32
    }
}

fn home_usage() -> Option<DiskUsage> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let path = CString::new(home).ok()?;
    let vfs = statvfs(path.as_c_str()).ok()?;
    let unit = vfs.fragment_size();
    Some(DiskUsage {
        total: vfs.blocks() * unit,
        available: vfs.blocks_available() * unit,
    })
}

/// Same output as the frontend's `system_status::human_size`.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

struct SettingsWindow {
    snapshot: Snapshot,
    disk: Option<DiskUsage>,
}

impl SettingsWindow {
    fn section(title: &str, rows: Vec<AnyElement>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_1()
            .child(div().text_sm().child(SharedString::from(title.to_string())))
            .children(rows)
    }

    fn row(label: impl IntoElement, value: impl IntoElement) -> AnyElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_2()
            .child(label)
            .child(value)
            .into_any_element()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let value = |text: String| div().text_sm().child(SharedString::from(text));

        let daemon_row = if self.snapshot.daemon {
            "Connected"
        } else {
            "Unavailable"
        };
        let readiness_row = if self.snapshot.total_systems == 0 {
            "Nothing reported yet".to_string()
        } else {
            format!(
                "{} of {} systems ready",
                self.snapshot.ready_systems, self.snapshot.total_systems
            )
        };
        let romm_row = match &self.snapshot.romm {
            Some(romm) if romm.configured => format!("Configured at {}", romm.base_url),
            Some(romm) => format!("Connected, not signed in to {}", romm.base_url),
            None => "Not set up".to_string(),
        };
        let stremio_row = match &self.snapshot.stremio_linked_at {
            Some(linked_at) => format!("Linked since {linked_at}"),
            None => "Not linked".to_string(),
        };

        let (percent, ring_number) = match &self.disk {
            Some(disk) => (disk.used_percent(), format!("{:.0}%", disk.used_percent())),
            None => (0.0, "--".to_string()),
        };
        let available = match &self.disk {
            Some(disk) => human_size(disk.available),
            None => "unavailable".to_string(),
        };

        v_flex()
            .w_full()
            .h_full()
            .bg(theme.background)
            .items_center()
            .justify_center()
            .p(px(24.0))
            .child(
                // `w_full` inside a cap, never a fixed width: this window is
                // opened on a TV and on a desktop, and the session's scale
                // factor can make the same window 541 logical pixels wide on
                // one and 948 on the other. A fixed column would hang off the
                // edge of the smaller one.
                v_flex()
                    .w_full()
                    .max_w(px(PAGE_WIDTH))
                    .flex_shrink_0()
                    .gap_5()
                    .child(div().text_2xl().child("Settings"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Read-only here: changes are made in the library app."),
                    )
                    .child(Self::section(
                        "Status",
                        vec![
                            Self::row("Daemon", value(daemon_row.to_string())),
                            Self::row("Retro readiness", value(readiness_row)),
                        ],
                    ))
                    .child(Self::section(
                        "Connections",
                        vec![
                            Self::row("RomM", value(romm_row)),
                            Self::row("Stremio", value(stremio_row)),
                        ],
                    ))
                    .child(Self::section(
                        "Storage",
                        vec![Self::row(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    ProgressCircle::new("disk")
                                        .value(percent)
                                        .color(theme.progress_bar)
                                        .w(px(RING_SIZE))
                                        .h(px(RING_SIZE))
                                        .child(div().text_xs().child(ring_number)),
                                )
                                .child("Storage available"),
                            value(available),
                        )],
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child("Escape or Back closes this window."),
                            )
                            .child(
                                Button::new("close")
                                    .label("Close")
                                    .on_click(|_, _window, cx| cx.quit()),
                            ),
                    ),
            )
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("hearthdeck_settings=info")),
        )
        .init();

    let snapshot = read_snapshot();
    let disk = home_usage();

    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        // The session this is drawn in is dark; the kit defaults to light.
        Theme::change(ThemeMode::Dark, None, cx);
        cx.bind_keys([KeyBinding::new("escape", CloseWindow, None)]);
        // App-level, not on the root element: `on_action` on the root would wrap
        // it in an interactive box whose sizing is not the window's, and the
        // whole page would lay out off-canvas.
        cx.on_action(|_: &CloseWindow, cx: &mut App| cx.quit());

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(200.0), px(60.0)),
                size(px(PAGE_WIDTH + 48.0), px(WINDOW_HEIGHT)),
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Hearthdeck - Settings".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        // What `gpui_kit::open_window` did: the application content under a
        // `Root`, which is what gives components their overlay and focus layers.
        let opened = cx.open_window(options, move |window, cx| {
            let view = cx.new(|_| SettingsWindow {
                snapshot: snapshot.clone(),
                disk,
            });
            cx.new(|cx| gpui_base::Root::new(view, window, cx))
        });
        if let Err(error) = opened {
            tracing::error!(%error, "failed to open the settings window");
        }
    });
}
