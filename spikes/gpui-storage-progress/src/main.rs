//! Spike: the library sidebar's storage footer, with gpui-kit's
//! `ProgressCircle` in place of the flat disk icon.
//!
//! `screens/library/sidebar.rs` draws a `drive-harddisk-solidstate-symbolic`
//! icon beside "Storage available" and the free-space string. Here that icon
//! becomes a ring filled to the *used* percentage, the number sits inside it,
//! and the caption and free-space string are unchanged.
//!
//! It is its own crate because gpui cannot be mounted inside the iced frontend:
//! it renders through its own window and event loop. This window is that
//! isolation - the footer on its own, nothing else to distract from it.

use std::ffi::CString;

use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, IntoElement, ParentElement as _, Render, Styled as _,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use nix::sys::statvfs::statvfs;

/// The shipped sidebar width, so the footer is judged at the size it will be
/// drawn.
const SIDEBAR_WIDTH: f32 = 260.0;
/// Larger than the icon it replaces, because a ring has to carry a number.
const RING_SIZE: f32 = 44.0;

#[derive(Clone, Copy)]
struct DiskUsage {
    total: u64,
    available: u64,
}

impl DiskUsage {
    /// How full the volume is, as the 0–100 the ring wants. An unreadable
    /// volume reports zero rather than NaN, since a widget cannot draw
    /// "unknown".
    fn used_percent(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let used = self.total.saturating_sub(self.available);
        (used as f64 / self.total as f64 * 100.0) as f32
    }
}

/// Usage of the filesystem the home directory lives on, read the same way the
/// shipped footer reads it.
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

/// Formats a byte count for humans, e.g. "128.4 GB". Same output as the
/// frontend's `system_status::human_size`.
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

struct StorageFooter {
    usage: Option<DiskUsage>,
}

impl Render for StorageFooter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (percent, number) = match self.usage {
            Some(usage) => (
                usage.used_percent(),
                format!("{:.0}%", usage.used_percent()),
            ),
            None => (0.0, "--".to_string()),
        };
        let available = match self.usage {
            Some(usage) => human_size(usage.available),
            None => "unavailable".to_string(),
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .items_center()
            .justify_center()
            .p(px(12.0))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        ProgressCircle::new("disk-usage")
                            .value(percent)
                            .color(theme.progress_bar)
                            .w(px(RING_SIZE))
                            .h(px(RING_SIZE))
                            .child(div().text_xs().child(number)),
                    )
                    .child(
                        v_flex()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Storage available"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.foreground)
                                    .child(available),
                            ),
                    ),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        // The COSMIC session this was judged in is dark; the kit defaults to
        // light, so ask for dark explicitly.
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);

        let usage = home_usage();
        if let Some(usage) = usage {
            println!(
                "volume: {} of {} used ({:.1}%), {} available",
                human_size(usage.total.saturating_sub(usage.available)),
                human_size(usage.total),
                usage.used_percent(),
                human_size(usage.available)
            );
        }

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(80.0), px(80.0)),
                size(px(SIDEBAR_WIDTH), px(120.0)),
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("hearthdeck - storage footer (gpui-kit spike)".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        gpui_kit::open_window(options, cx, move |_, cx| {
            cx.new(|_| StorageFooter { usage })
        })
        .expect("failed to open the spike window");
    });
}
