// Bare-minimum quick-menu overlay for the Hearthdeck session (see
// packaging/arch/hearthdeck-session). A full-screen, semi-transparent
// wlr-layer-shell surface, toggled by COSMIC's global shortcut or a gamepad's
// Guide/Mode button, listing session actions - starting with "Close App".
//
// Linux-only because it uses Wayland layer-shell.
#[cfg(target_os = "linux")]
mod input;
#[cfg(target_os = "linux")]
mod overlay;
#[cfg(target_os = "linux")]
mod shortcut;
#[cfg(target_os = "linux")]
mod state;

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    overlay::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hearthdeck-overlay is supported only on Linux Wayland sessions");
}
