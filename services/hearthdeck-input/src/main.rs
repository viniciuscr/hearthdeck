#[cfg(target_os = "linux")]
mod broker;
#[cfg(target_os = "linux")]
mod focus;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", test))]
mod mapping;
#[cfg(any(target_os = "linux", test))]
mod ownership;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    broker::run().await
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hearthdeck-input is only supported on Linux");
}
