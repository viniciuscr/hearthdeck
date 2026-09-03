use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest = Path::new(&out_dir).join("config.rs");

    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());
    let app_id = "org.hearthdeck.HearthDeck";

    fs::write(
        &dest,
        format!(
            "pub const APP_ID: &str = \"{app_id}\";\npub const VERSION: &str = \"{version}\";\n"
        ),
    )
    .expect("failed to write config.rs");

    println!("cargo:rustc-env=COSMIC_CONFIG_PATH={}", dest.display());
}
