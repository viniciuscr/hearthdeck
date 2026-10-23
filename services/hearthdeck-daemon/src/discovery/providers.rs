#[cfg(target_os = "linux")]
pub mod desktop_apps;
#[cfg(target_os = "linux")]
pub mod heroic;
// Not platform-gated like the two above: these providers talk to an HTTP API and
// read nothing from the host.
pub mod stremio;
pub mod stremio_arrivals;
