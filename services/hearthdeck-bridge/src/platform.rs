#[cfg(target_os = "linux")]
pub const DESKTOP_APPS_SOURCE: &str = "desktop-apps";
#[cfg(target_os = "macos")]
pub const MACOS_APPS_SOURCE: &str = "macos-apps";

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{discover_applications, launch_application};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{discover_applications, launch_application};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn discover_applications(_source_id: &str) -> Result<Vec<DiscoveredApplication>> {
    anyhow::bail!("application discovery is unsupported on this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn launch_application(_source_id: &str, _application_id: &str) -> Result<()> {
    anyhow::bail!("application launch is unsupported on this platform")
}
