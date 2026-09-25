//! Optional internet research, kept behind a trait so the scan owns the policy
//! (whether to research at all, and how many lookups to run at once) and the
//! provider owns the source.

use async_trait::async_trait;

use super::error::Result;
use super::model::AppProfile;

/// Looks an application up online and returns what it learned.
///
/// Implementations describe the *upstream* project — its summary, developer,
/// homepage, keywords — never what is installed locally. [`AppProfile::merge_research`]
/// enforces that by refusing to overwrite discovery fields, so a provider cannot
/// rename an app the user already has.
#[async_trait]
pub trait AppResearcher: Send + Sync {
    /// Stable id, recorded in the scan report.
    fn id(&self) -> &'static str;

    /// Return an enriched profile, or `None` when the app is unknown upstream.
    /// An unreachable source is an error, so the scan can report which lookups
    /// failed instead of silently treating them as "no data".
    async fn research(&self, app: &AppProfile) -> Result<Option<AppProfile>>;
}

/// The default: research nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopResearcher;

#[async_trait]
impl AppResearcher for NoopResearcher {
    fn id(&self) -> &'static str {
        "none"
    }

    async fn research(&self, _app: &AppProfile) -> Result<Option<AppProfile>> {
        Ok(None)
    }
}
