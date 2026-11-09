//! Stremio's addon protocol, for the one question the arrivals banner asks:
//! does this account have a stream for this title?
//!
//! The account's addon collection is the list Stremio itself would offer the
//! person, so confirming an arrival against it asks exactly what pressing Play
//! in Stremio would ask. `addonCollectionGet` returns that collection; every
//! addon whose manifest declares a `stream` resource is then asked
//! `GET <transport>/stream/<type>/<id>.json`, and a non-empty `streams` array is
//! the answer. These are the addon protocol's own shapes, not another
//! undocumented service.
//!
//! No transport URL is ever quoted into an error or a log line. A configured
//! addon carries the person's settings in its path — a debrid key among them —
//! so failures name the addon by host only.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tracing::debug;

/// How long one addon gets to answer. Stream addons scrape indexers or a debrid
/// account, so this is deliberately longer than the account API's timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// How much of an addon's answer is read. A stream list is a few kilobytes; the
/// cap keeps a broken addon from filling memory through a kiosk's refresh.
const MAX_STREAM_BODY: u64 = 1 << 20;

/// The stream addons an account has installed, and a client to ask them with.
#[derive(Clone, Debug)]
pub(crate) struct StreamAddons {
    addons: Vec<StreamAddon>,
    client: reqwest::Client,
}

/// One addon that can answer a stream request, with what it said it serves.
#[derive(Clone, Debug)]
struct StreamAddon {
    /// The transport URL without the `manifest.json` segment.
    base: String,
    /// The host alone, for a log line: the path can carry configuration.
    host: String,
    /// The title types the manifest says it serves streams for.
    types: Vec<String>,
}

impl StreamAddons {
    /// The account's stream-capable addons, read with its session key.
    pub(crate) async fn installed(auth_key: &str) -> Result<Self> {
        let body = crate::stremio::account_call("addonCollectionGet", auth_key, json!({})).await?;
        let collection = body
            .get("result")
            .ok_or_else(|| anyhow!("the Stremio addon collection answered without a result"))?;
        Ok(Self {
            addons: stream_addons(collection)?,
            client: reqwest::Client::new(),
        })
    }

    /// Whether the account has anything to play with at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.addons.is_empty()
    }

    /// How many addons would be asked, for a health line.
    pub(crate) fn len(&self) -> usize {
        self.addons.len()
    }

    /// Whether any installed addon answers with at least one stream.
    ///
    /// `Ok(false)` is a decision: at least one addon answered, and none had a
    /// stream. `Err` means no addon could answer, which is a failure to decide
    /// rather than an answer — the caller keeps the last snapshot instead of
    /// dropping a title because an addon happened to be down.
    pub(crate) async fn has_stream(&self, kind: &str, id: &str) -> Result<bool> {
        let mut answered = 0;
        let mut last_error = None;
        for addon in self.addons.iter().filter(|addon| addon.serves(kind)) {
            match addon.has_stream(&self.client, kind, id).await {
                Ok(true) => return Ok(true),
                Ok(false) => answered += 1,
                Err(error) => {
                    debug!(
                        addon = %addon.host,
                        kind,
                        id,
                        %error,
                        "a stream addon could not be read"
                    );
                    last_error = Some(error);
                }
            }
        }
        match last_error {
            Some(error) if answered == 0 => Err(error),
            _ => Ok(false),
        }
    }
}

impl StreamAddon {
    fn serves(&self, kind: &str) -> bool {
        self.types.iter().any(|served| served == kind)
    }

    /// The addon protocol's resource URL for one title.
    ///
    /// The id has already been narrowed to an IMDb title id by the caller, so it
    /// needs no escaping.
    fn stream_url(&self, kind: &str, id: &str) -> String {
        format!("{}/stream/{kind}/{id}.json", self.base)
    }

    async fn has_stream(&self, client: &reqwest::Client, kind: &str, id: &str) -> Result<bool> {
        let response = client
            .get(self.stream_url(kind, id))
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            // `without_url` is load-bearing: the path may carry the account's
            // own addon configuration, and a `reqwest::Error` quotes the URL it
            // failed on.
            .map_err(|error| {
                anyhow!(
                    "the {} addon did not answer: {}",
                    self.host,
                    error.without_url()
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            bail!(
                "the {} addon refused a stream request with HTTP {status}",
                self.host
            );
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_STREAM_BODY)
        {
            bail!(
                "the {} addon answered with more than {MAX_STREAM_BODY} bytes",
                self.host
            );
        }
        let body = response.text().await.map_err(|error| {
            anyhow!(
                "the {} addon's answer could not be read: {}",
                self.host,
                error.without_url()
            )
        })?;
        if body.len() as u64 > MAX_STREAM_BODY {
            bail!(
                "the {} addon answered with more than {MAX_STREAM_BODY} bytes",
                self.host
            );
        }
        let parsed: Value = serde_json::from_str(&body).map_err(|_| {
            anyhow!(
                "the {} addon answered with a body that is not JSON",
                self.host
            )
        })?;
        // An addon that answers without a `streams` list did not answer the
        // question, so this is an error rather than an empty result.
        let streams = parsed
            .get("streams")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("the {} addon answered without a streams list", self.host))?;
        Ok(!streams.is_empty())
    }
}

/// One account's addon collection as the stream addons in it.
///
/// An addon that is malformed, or whose transport is not an HTTP URL, is skipped
/// rather than failing the collection: one broken entry must not take the
/// banner down with it.
fn stream_addons(collection: &Value) -> Result<Vec<StreamAddon>> {
    let addons = collection
        .get("addons")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("the Stremio addon collection answered without an addon list"))?;
    Ok(addons.iter().filter_map(stream_addon).collect())
}

fn stream_addon(addon: &Value) -> Option<StreamAddon> {
    let manifest = addon.get("manifest")?;
    let types = stream_types(manifest);
    if types.is_empty() {
        return None;
    }
    let transport = addon.get("transportUrl").and_then(Value::as_str)?;
    let (base, host) = transport_base(transport)?;
    Some(StreamAddon { base, host, types })
}

/// The title types a manifest serves streams for, empty when it serves none.
///
/// The protocol allows two spellings: `resources: ["stream"]`, which means every
/// type in `manifest.types`, and a resource object that names its own types.
fn stream_types(manifest: &Value) -> Vec<String> {
    let manifest_types = strings(manifest.get("types"));
    let Some(resources) = manifest.get("resources").and_then(Value::as_array) else {
        return Vec::new();
    };
    for resource in resources {
        let is_stream = match resource {
            Value::String(name) => name == "stream",
            Value::Object(_) => resource.get("name").and_then(Value::as_str) == Some("stream"),
            _ => false,
        };
        if !is_stream {
            continue;
        }
        let declared = strings(resource.get("types"));
        return if declared.is_empty() {
            manifest_types
        } else {
            declared
        };
    }
    Vec::new()
}

/// The transport URL split into a resource base and a host for log lines.
///
/// The collection stores the URL of the addon's manifest; its resources live
/// beside it, so the `manifest.json` segment is what is removed. A legacy
/// `stremio://` URL is the app's own spelling of `https://`. Anything that is
/// not HTTP is refused rather than handed to a client that would try it.
fn transport_base(transport: &str) -> Option<(String, String)> {
    let transport = transport.trim();
    let normalized = transport
        .strip_prefix("stremio://")
        .map_or_else(|| transport.to_owned(), |rest| format!("https://{rest}"));
    let url = reqwest::Url::parse(&normalized).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_owned();
    let mut base = url.to_string();
    if let Some(stripped) = base.strip_suffix("/manifest.json") {
        base = stripped.to_owned();
    } else if let Some(stripped) = base.strip_suffix("manifest.json") {
        base = stripped.to_owned();
    }
    let base = base.trim_end_matches('/').to_owned();
    (!base.is_empty()).then_some((base, host))
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{StreamAddons, stream_addon, stream_addons, stream_types, transport_base};

    fn collection(addons: Vec<serde_json::Value>) -> serde_json::Value {
        json!({ "addons": addons })
    }

    fn addon(transport: &str, manifest: serde_json::Value) -> serde_json::Value {
        json!({ "transportUrl": transport, "manifest": manifest })
    }

    #[test]
    fn a_manifest_declares_streams_as_a_string_or_as_a_resource_object() {
        let as_string = json!({
            "types": ["movie", "series"],
            "resources": ["catalog", "stream"],
        });
        assert_eq!(stream_types(&as_string), ["movie", "series"]);

        let as_object = json!({
            "types": ["movie", "series", "channel"],
            "resources": [
                { "name": "catalog", "types": ["movie"] },
                { "name": "stream", "types": ["movie"] },
            ],
        });
        assert_eq!(stream_types(&as_object), ["movie"]);

        // An object without its own types falls back to the manifest's.
        let inherited = json!({
            "types": ["series"],
            "resources": [{ "name": "stream" }],
        });
        assert_eq!(stream_types(&inherited), ["series"]);

        let catalog_only = json!({ "types": ["movie"], "resources": ["catalog", "meta"] });
        assert!(stream_types(&catalog_only).is_empty());

        let malformed = json!({ "types": ["movie"] });
        assert!(stream_types(&malformed).is_empty());
    }

    #[test]
    fn a_transport_url_loses_its_manifest_segment_and_keeps_a_config_path() {
        assert_eq!(
            transport_base("https://torrentio.strem.fun/manifest.json"),
            Some((
                "https://torrentio.strem.fun".to_owned(),
                "torrentio.strem.fun".to_owned()
            ))
        );
        assert_eq!(
            transport_base("https://torrentio.strem.fun/debrid-config|sort=seeders/manifest.json"),
            Some((
                "https://torrentio.strem.fun/debrid-config|sort=seeders".to_owned(),
                "torrentio.strem.fun".to_owned()
            ))
        );
        // A URL with no manifest segment is already a base.
        assert_eq!(
            transport_base("https://addons.example.test/"),
            Some((
                "https://addons.example.test".to_owned(),
                "addons.example.test".to_owned()
            ))
        );
    }

    #[test]
    fn a_legacy_stremio_transport_becomes_https_and_other_schemes_are_refused() {
        assert_eq!(
            transport_base("stremio://addons.example.test/manifest.json"),
            Some((
                "https://addons.example.test".to_owned(),
                "addons.example.test".to_owned()
            ))
        );
        for refused in [
            "file:///etc/passwd",
            "data:application/json,{}",
            "",
            "not a url",
        ] {
            assert_eq!(transport_base(refused), None, "{refused} must be refused");
        }
    }

    #[test]
    fn the_collection_keeps_only_addons_that_serve_streams() {
        let collection = collection(vec![
            addon(
                "https://v3-cinemeta.strem.io/manifest.json",
                json!({ "types": ["movie"], "resources": ["catalog", "meta"] }),
            ),
            addon(
                "https://streams.example.test/manifest.json",
                json!({ "types": ["movie", "series"], "resources": ["stream"] }),
            ),
            // Malformed, or not HTTP, or no manifest: skipped, not fatal.
            json!({ "transportUrl": "https://broken.example.test/manifest.json" }),
            addon(
                "file:///tmp/addon/manifest.json",
                json!({ "types": ["movie"], "resources": ["stream"] }),
            ),
        ]);

        let addons = stream_addons(&collection).expect("a readable collection");

        assert_eq!(addons.len(), 1, "only the stream addon survives");
        assert_eq!(addons[0].host, "streams.example.test");
        assert_eq!(addons[0].types, ["movie", "series"]);
    }

    #[test]
    fn a_manifest_without_an_addon_list_is_an_error_but_a_bad_entry_is_not() {
        assert!(stream_addons(&json!({ "result": {} })).is_err());
        let addons = stream_addons(&collection(vec![])).expect("an empty collection");
        assert!(addons.is_empty());
        assert!(StreamAddons::is_empty(&StreamAddons {
            addons,
            client: reqwest::Client::new(),
        }));
    }

    #[test]
    fn an_addon_serves_the_types_its_manifest_names() {
        let addon = stream_addon(&addon(
            "https://streams.example.test/manifest.json",
            json!({ "types": ["movie", "series"], "resources": [{ "name": "stream", "types": ["movie"] }] }),
        ))
        .expect("a stream addon");

        assert!(addon.serves("movie"));
        assert!(!addon.serves("series"));
        assert_eq!(
            addon.stream_url("movie", "tt1234567"),
            "https://streams.example.test/stream/movie/tt1234567.json"
        );
    }
}
