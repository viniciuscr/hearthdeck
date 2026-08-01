# Metadata Enrichment

## Goal

Hearthdeck maintains a local materialized metadata catalog. It does not scrape random
websites by application name and it does not perform network enrichment while
serving a frontend request.

```text
Application discovery -> library_items
Metadata provider -> catalog_enrichments
CatalogStore join -> API library item metadata
```

Discovery answers "what is installed and how can it launch?" Metadata answers
"what is it, who made it, where is its project, and what media is available?"
They have separate schedules, workers, tables, and provenance.

## Minimum Catalog Metadata

Every library item has a stable minimum presentation record, even when no rich
metadata provider matches it:

- identity: item ID, title, content kind, launch ID, and icon reference;
- summary: AppStream description/summary when available, otherwise the desktop
  entry comment, otherwise the title;
- categories: AppStream categories when available, otherwise Freedesktop
  desktop-entry categories, otherwise `Other`;
- provenance: the winning metadata provider or `desktop-entry`.

The client uses these categories as app-library tabs rather than placing every
desktop application in one provider-wide list. A desktop entry with the
Freedesktop `Game` category is classified as a game; future game providers own
their more specific genres and store categories.

Project HTTP(S) URLs are emitted as typed metadata URL entries. The client
renders each valid URL as a dedicated action, such as **Website**, **Source
code**, or **Report issue**; link types are never presented as inert facts.

## Source Policy

| Provider | Trust | Intended data | Policy |
| --- | --- | --- | --- |
| `appstream-local` | Local distro/Flatpak metadata | Summary, description, categories, developer, license, project URLs, icon, screenshots | Default Linux provider |
| Future Flatpak AppStream | Configured Flatpak remote metadata | Flatpak-specific app metadata and media | Separate provider, refresh on remote update/manual request |
| Future Steam/GOG/Epic adapters | Official client/API metadata | Games, artwork, store links, achievements | One provider per platform |
| Future GitHub enrichment | Only an AppStream/official VCS URL | Repository and release links | Explicit opt-in, cached, rate-limited |

Application-name search is never a metadata matching strategy. Providers match
stable application IDs, desktop IDs, bundle IDs, or source-specific platform
IDs. This prevents incorrect attribution.

## AppStream

`appstream-local` indexes local `metainfo` and `appdata` XML documents from
XDG data directories. It stores a normalized record keyed by the primary
component ID and all declared desktop/launchable IDs. The enrichment payload
includes summary, description, developer, project license, categories, URLs,
icon reference, screenshot source URLs, and `provenance: appstream-local`.

`appstream-local` is Linux-only. macOS bundle discovery intentionally provides
identity and launch capability only; it does not guess metadata by application
name. A future macOS metadata adapter can be introduced as its own provider
once an authoritative bundle-linked source is selected.

The daemon stores screenshot URLs as metadata only. A future asset-cache module
must download, validate, resize, evict, and serve cached media; frontend code
must never load provider URLs directly as trusted local assets. Heroic artwork
URLs are a documented temporary exception in the current client and must not be
copied to new providers; see `docs/product-foundations.md` for the target
media policy.

## Freshness And Priority

`catalog_enrichments` stores each provider snapshot independently with its
provider ID, priority, and update timestamp. Catalog reads select the highest
priority matching enrichment for an installed application's `launch_id`.
This allows a future official store provider to override general metadata
without deleting AppStream data.

## Operations

Metadata providers run once at daemon startup and on their own interval.
Refresh one provider through the authenticated API:

```sh
POST /v1/metadata/appstream-local/refresh
```

The request returns `202 Accepted`; completion emits:

```json
{
  "type": "metadata_changed",
  "provider_id": "appstream-local",
  "record_count": 42
}
```

Live catalog clients reload when they receive this event.
