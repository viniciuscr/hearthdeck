// Disposable spike, shipped temporarily so it can be run on kiosk hardware
// with just `sudo pacman -Syu` (see packaging/arch/PKGBUILD) — not a real,
// permanent Hearthdeck component. Confirms, on real hardware, two separate
// things a real side-menu overlay depends on:
//
// 1. A plain Xwayland client marking itself with the
//    GAMESCOPE_EXTERNAL_OVERLAY=1 property is composited on top by Gamescope
//    without resizing/shrinking whatever else is running (Hearthdeck itself,
//    or a bridge-launched app), and disappears cleanly on exit. (Confirmed
//    true in an earlier run of this same crate.)
// 2. Whether that window can have real per-pixel alpha, not just a single
//    whole-window opacity: a side menu needs an opaque/semi-opaque panel on
//    one side and a fully transparent region everywhere else, so the
//    content underneath shows through untouched where there's no menu.
//
// This draws a window split into three equal vertical bands so both
// questions can be checked in one run:
//   - left:   fully opaque magenta (sanity check against the first finding)
//   - middle: ~50% transparent magenta (does partial alpha blend at all?)
//   - right:  fully transparent (does the content underneath show through
//             completely, with nothing painted at all?)
//
// Once both findings are confirmed either way, delete this crate directory
// and remove references to it from services/Cargo.toml's members,
// packaging/arch/PKGBUILD's package(), and docs/arch-package.md.

use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::render::{self, ConnectionExt as _, PictOp, PictType, Pictformat};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ColormapAlloc, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    Rectangle, Visualid, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

const RUN_SECONDS: u64 = 120;
const WIDTH: u16 = 900;
const HEIGHT: u16 = 300;
const BAND_WIDTH: u16 = WIDTH / 3;

fn intern(conn: &impl Connection, name: &str) -> Result<Atom, Box<dyn std::error::Error>> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

/// Finds the ARGB32 (depth 32, direct, with a non-zero alpha mask) picture
/// format and its matching visual on the given screen — the standard visual
/// real per-pixel-transparent windows (transparent terminals, compositor
/// demos, etc.) use on any X11/Xwayland server.
fn find_argb32_visual(
    conn: &impl Connection,
    screen_num: usize,
) -> Result<(Visualid, Pictformat), Box<dyn std::error::Error>> {
    let formats = conn.render_query_pict_formats()?.reply()?;
    let argb32 = formats
        .formats
        .iter()
        .find(|format| {
            format.type_ == PictType::DIRECT && format.depth == 32 && format.direct.alpha_mask != 0
        })
        .ok_or("server has no ARGB32 picture format")?;

    let screen = formats
        .screens
        .get(screen_num)
        .ok_or("render extension has no data for this screen")?;
    let visual = screen
        .depths
        .iter()
        .flat_map(|depth| depth.visuals.iter())
        .find(|pict_visual| pict_visual.format == argb32.id)
        .ok_or("no visual on this screen matches the ARGB32 picture format")?;

    Ok((visual.visual, argb32.id))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let (visual_id, pict_format) = find_argb32_visual(&conn, screen_num)?;

    let colormap = conn.generate_id()?;
    conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, visual_id)?;

    let window = conn.generate_id()?;
    conn.create_window(
        32,
        window,
        screen.root,
        100,
        100,
        WIDTH,
        HEIGHT,
        0,
        WindowClass::INPUT_OUTPUT,
        visual_id,
        &CreateWindowAux::new()
            .border_pixel(0)
            .colormap(colormap)
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

    // The property under test for "stays on top without breaking the
    // session," proven true in an earlier run of this same crate.
    let external_overlay_atom = intern(&conn, "GAMESCOPE_EXTERNAL_OVERLAY")?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        external_overlay_atom,
        AtomEnum::CARDINAL,
        &[1u32],
    )?;

    conn.map_window(window)?;

    // Paint the three bands via XRender, which understands per-pixel alpha;
    // plain core-X11 GC fills do not carry alpha the way this needs.
    let picture = conn.generate_id()?;
    conn.render_create_picture(
        picture,
        window,
        pict_format,
        &render::CreatePictureAux::new(),
    )?;

    let left = Rectangle {
        x: 0,
        y: 0,
        width: BAND_WIDTH,
        height: HEIGHT,
    };
    let middle = Rectangle {
        x: BAND_WIDTH as i16,
        y: 0,
        width: BAND_WIDTH,
        height: HEIGHT,
    };
    let right = Rectangle {
        x: (BAND_WIDTH * 2) as i16,
        y: 0,
        width: WIDTH - BAND_WIDTH * 2,
        height: HEIGHT,
    };

    // Premultiplied-alpha magenta at full opacity.
    conn.render_fill_rectangles(
        PictOp::SRC,
        picture,
        render::Color {
            red: 0xffff,
            green: 0,
            blue: 0xffff,
            alpha: 0xffff,
        },
        &[left],
    )?;
    // Premultiplied-alpha magenta at ~50% opacity.
    conn.render_fill_rectangles(
        PictOp::SRC,
        picture,
        render::Color {
            red: 0x8000,
            green: 0,
            blue: 0x8000,
            alpha: 0x8000,
        },
        &[middle],
    )?;
    // Fully transparent: nothing should be visible here at all, and
    // whatever is underneath (Hearthdeck, or a bridge-launched app) should
    // show through exactly as if this window weren't there.
    conn.render_fill_rectangles(
        PictOp::SRC,
        picture,
        render::Color {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 0,
        },
        &[right],
    )?;

    conn.flush()?;

    println!("Window mapped with GAMESCOPE_EXTERNAL_OVERLAY=1, split into three bands:");
    println!("  left   = fully opaque magenta");
    println!("  middle = ~50% transparent magenta");
    println!("  right  = fully transparent");
    println!();
    println!("Check, against whatever is running underneath (Hearthdeck itself,");
    println!("or a bridge-launched app in its own nested Gamescope):");
    println!("  - left/middle stay on top without resizing/shrinking the session");
    println!("    (same finding as the earlier solid-color run of this tool)");
    println!("  - middle visibly blends with whatever is underneath, not just");
    println!("    with this window's own black/magenta content");
    println!("  - right shows the content underneath completely untouched, as");
    println!("    if this window's right third didn't exist at all");
    println!();
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

    conn.render_free_picture(picture)?;
    conn.destroy_window(window)?;
    conn.free_colormap(colormap)?;
    conn.flush()?;
    println!("Window destroyed. Confirm it disappeared cleanly.");

    Ok(())
}
