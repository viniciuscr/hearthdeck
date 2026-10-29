//! Built-in catalog and launch source identifiers.
//!
//! The discovery registry remains open to other identifiers. These constants
//! identify sources whose behavior is understood across process boundaries.

pub const DESKTOP_APPS: &str = "desktop-apps";
pub const HEROIC: &str = "heroic";
pub const RETROARCH: &str = "retroarch";
pub const STREMIO: &str = "stremio";
pub const STREMIO_ARRIVALS: &str = "stremio-arrivals";

#[cfg(test)]
mod tests {
    use super::{DESKTOP_APPS, HEROIC, RETROARCH, STREMIO, STREMIO_ARRIVALS};

    #[test]
    fn built_in_sources_keep_their_persisted_identifiers() {
        assert_eq!(
            [DESKTOP_APPS, HEROIC, RETROARCH, STREMIO, STREMIO_ARRIVALS],
            [
                "desktop-apps",
                "heroic",
                "retroarch",
                "stremio",
                "stremio-arrivals"
            ]
        );
    }
}
