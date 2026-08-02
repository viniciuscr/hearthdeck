// Disposable spike, shipped temporarily so it can be run on kiosk hardware
// with just `sudo pacman -Syu` (see packaging/arch/PKGBUILD) — not a real,
// permanent Hearthdeck component. Confirms, on real hardware, that a plain
// Xwayland client marking itself with the GAMESCOPE_EXTERNAL_OVERLAY=1
// property is composited on top by Gamescope without resizing/shrinking
// whatever else is running (Hearthdeck itself, or a bridge-launched app),
// and disappears cleanly on exit.
//
// Once the finding is confirmed either way, delete this crate directory and
// remove references to it from services/Cargo.toml's members,
// packaging/arch/PKGBUILD's package(), and docs/arch-package.md.

use std::thread;
use std::time::{Duration, Instant};

use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

const RUN_SECONDS: u64 = 120;
// Loud magenta so it's unmistakable against Hearthdeck's own UI.
const FILL_COLOR: u32 = 0x00FF_00FF;

fn intern(conn: &impl Connection, name: &str) -> Result<Atom, Box<dyn std::error::Error>> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];

    let window = conn.generate_id()?;
    let (width, height) = (640u16, 360u16);

    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        window,
        screen.root,
        100,
        100,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(FILL_COLOR)
            .event_mask(EventMask::EXPOSURE | EventMask::KEY_PRESS),
    )?;

    // Identify the window normally (this must behave like a regular,
    // ICCCM-managed top-level client, NOT override-redirect, so Gamescope's
    // steamcompmgr treats it as a real window it can special-case via the
    // GAMESCOPE_EXTERNAL_OVERLAY property, rather than ignoring it the way
    // override-redirect windows are ignored elsewhere in that code path).
    let title = b"hearthdeck-overlay-spike";
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        title,
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        intern(&conn, "_NET_WM_NAME")?,
        intern(&conn, "UTF8_STRING")?,
        title,
    )?;

    // The one property under test.
    let external_overlay_atom = intern(&conn, "GAMESCOPE_EXTERNAL_OVERLAY")?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        external_overlay_atom,
        AtomEnum::CARDINAL,
        &[1u32],
    )?;

    conn.map_window(window)?;
    conn.flush()?;

    println!("Window mapped with GAMESCOPE_EXTERNAL_OVERLAY=1.");
    println!("Expected if the finding holds: a solid magenta box stays on");
    println!("top of whatever else is running (Hearthdeck itself, or a");
    println!("bridge-launched app in its own nested Gamescope), and neither");
    println!("one resizes/shrinks/blurs while this is up.");
    println!(
        "Exiting automatically in {}s, or press Ctrl+C now.",
        RUN_SECONDS
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(RUN_SECONDS) {
        while let Some(_event) = conn.poll_for_event()? {
            // Draining the event queue only; content doesn't matter here.
        }
        thread::sleep(Duration::from_millis(200));
    }

    conn.destroy_window(window)?;
    conn.flush()?;
    println!("Window destroyed. Confirm it disappeared cleanly.");

    Ok(())
}
