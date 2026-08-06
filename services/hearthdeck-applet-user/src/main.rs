// COSMIC panel applet: shows the logged-in username in the top bar.
//
// cosmic-panel discovers this binary by its app ID
// (io.github.viniciuscr.hearthdeck.AppletUser). It must be on $PATH or
// registered in a desktop file with that ID. See packaging/arch/PKGBUILD
// (installed to /usr/bin/) and cosmic-test-session's apply_cosmic_overrides
// (wired into plugins_wings).
#[cfg(target_os = "linux")]
mod applet;

#[cfg(target_os = "linux")]
fn main() -> cosmic::iced::Result {
    applet::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hearthdeck-applet-user is supported only on Linux Wayland sessions");
}
