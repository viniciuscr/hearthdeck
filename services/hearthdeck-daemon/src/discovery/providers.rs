#[cfg(target_os = "linux")]
pub mod desktop_apps;
#[cfg(target_os = "linux")]
pub mod heroic;
// Not platform-gated like the two above: this provider talks to an HTTP API and
// reads nothing from the host.
pub mod stremio;
