// COSMIC panel applet: shows the logged-in username in the top bar.
//
// cosmic-panel discovers and spawns this applet by finding a .desktop file
// named after its app ID (io.github.viniciuscr.hearthdeck.AppletUser) and
// running that file's Exec= line - not by matching a same-named binary on
// $PATH directly, though Exec= does point at this binary, which the "must
// be on $PATH" part of that still requires. See
// packaging/arch/hearthdeck-applet-user.desktop, packaging/arch/PKGBUILD
// (both installed), cosmic-test-session's apply_cosmic_overrides (wired
// into plugins_wings), and docs/cosmic-panel-customization.md.
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
