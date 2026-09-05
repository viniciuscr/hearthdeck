// Bare-minimum quick-menu overlay for the "COSMIC (Test)" session (see
// packaging/arch/cosmic-test-session). A full-screen, semi-transparent
// wlr-layer-shell surface, toggled by COSMIC's global shortcut or a gamepad's
// Guide/Mode button, listing session actions - starting with "Close App".
//
// UNTESTED against a real Linux/Wayland/wgpu toolchain: written without
// access to one. The layer-shell wiring below mirrors real, shipped code
// read directly from cosmic-launcher's own src/app.rs (its
// `create_dummy_layer_surface`/`show`/`hide` functions) and cosmic-comp's
// own src/lib.rs (kiosk_child exit handling), not guessed from memory - but
// libcosmic's git-main API has no stable release and does change, so the
// Build this module on its target platform as part of the services workspace.
// It is Linux-only because it uses Wayland layer-shell.
#[cfg(target_os = "linux")]
mod input;
#[cfg(target_os = "linux")]
mod overlay;
#[cfg(target_os = "linux")]
mod shortcut;

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    overlay::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hearthdeck-overlay is supported only on Linux Wayland sessions");
}
