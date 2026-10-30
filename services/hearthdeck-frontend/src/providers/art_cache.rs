//! Downloaded artwork on disk: posters, covers and RomM assets, each stored
//! once under a stable name so a second request reads the file instead.

use std::{
    env,
    hash::{DefaultHasher, Hasher},
    path::PathBuf,
    time::Duration,
};

/// How long one artwork download may take. Artwork comes from third-party
/// hosts, and a stalled one must not hold a rail back for the daemon's 30s.
pub(super) const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);

/// The cached file for `key`, whatever extension it was saved with.
pub(crate) fn cached(key: &str) -> Option<String> {
    // `svg` and `ico` matter for RomM's bundled console art, which is served
    // in those formats; omitting them made platform icons re-download on every
    // refresh because the cache lookup never matched.
    ["jpg", "jpeg", "png", "webp", "svg", "ico"]
        .into_iter()
        .map(|extension| cache_dir().join(file_name(key, extension)))
        .find(|path| path.metadata().is_ok_and(|metadata| metadata.len() >= 16))
        .map(|path| path.to_string_lossy().into_owned())
}

/// Sends `request` and stores a successful body under `key`.
///
/// The extension comes from the response, never the URL: a Metahub poster URL
/// carries none, and a webp saved as `.png` is a file the decoder refuses.
pub(super) async fn download(request: reqwest::RequestBuilder, key: String) -> Option<String> {
    let response = request.send().await.ok()?.error_for_status().ok()?;
    let extension = image_extension(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    );
    let bytes = response.bytes().await.ok()?;
    tokio::task::spawn_blocking(move || store(&key, extension, &bytes))
        .await
        .ok()
        .flatten()
}

/// The file name a cached download is stored under: a stable hash of its key,
/// then the given extension.
///
/// Pure and free of any network or disk access, which is the point: the whole of
/// the extension-mismatch bug lived in this naming decision, so a test can pin it
/// here. Every writer names its file through it, and [`cached`]'s probe rebuilds
/// the same names, so what one writes the others find.
fn file_name(key: &str, extension: &str) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(key.as_bytes());
    format!("{:016x}.{extension}", hasher.finish())
}

fn store(key: &str, extension: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() < 16 {
        return None;
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(file_name(key, extension));
    std::fs::write(&path, bytes).ok()?;
    Some(path.to_string_lossy().into_owned())
}

fn image_extension(content_type: Option<&str>) -> &'static str {
    let Some(content_type) = content_type else {
        return "jpg";
    };
    let content_type = content_type.to_ascii_lowercase();
    if content_type.starts_with("image/png") {
        "png"
    } else if content_type.starts_with("image/webp") {
        "webp"
    } else if content_type.starts_with("image/svg") {
        "svg"
    } else if content_type.contains("icon") {
        // Covers both `image/x-icon` and `image/vnd.microsoft.icon`; RomM's
        // bundled console art ships `.ico` for platforms with no `.svg`.
        "ico"
    } else {
        "jpg"
    }
}

fn cache_dir() -> PathBuf {
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("hearthdeck/icons")
}

#[cfg(test)]
mod tests {
    use super::{file_name, image_extension};

    /// The extension a cached cover is stored with must come from the response,
    /// not the URL. Metahub poster URLs carry no extension, so the old URL-derived
    /// name was always `<hash>.png` — and a webp poster saved as `.png` is a file
    /// the image decoder refuses, which draws the placeholder. That is exactly why
    /// only some Continue Watching tiles were blank, and always the same ones.
    #[test]
    fn cached_icon_name_follows_the_response_content_type() {
        let poster = "https://images.metahub.space/poster/medium/tt0149460/img";

        assert_eq!(image_extension(Some("image/webp")), "webp");
        assert_eq!(image_extension(Some("image/jpeg")), "jpg");
        assert_eq!(image_extension(Some("image/png")), "png");

        // An extension-free URL no longer forces "png": the name carries the
        // extension the response asked for.
        assert!(file_name(poster, image_extension(Some("image/webp"))).ends_with(".webp"));
        assert!(file_name(poster, image_extension(Some("image/jpeg"))).ends_with(".jpg"));

        // One URL, any extension: one stem. That is what lets `cached`'s
        // multi-extension probe find whatever `download` wrote.
        let stem = |name: String| name.rsplit_once('.').unwrap().0.to_string();
        assert_eq!(
            stem(file_name(poster, "png")),
            stem(file_name(poster, "webp"))
        );
    }
}
