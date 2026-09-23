# hearthdeck-categorizer

Laya-backed categorization for the Hearthdeck library.

The frontend currently decides what an app *is* with hand-maintained tables in
`app_group.rs`: a list of streaming-service names (`is_watch_entry`), a list of
emulator names (`is_emulator_entry`), and a whitelist of freedesktop categories
mapped one-to-one onto tabs (`sync_category_groups`). This crate replaces those
lookups with a local model that reads a full application record and answers a
fixed set of questions about it.

## Shape of a run

```text
AppProfile ──► [AppResearcher] ──► Categorizer ──► LibraryScanner ──► ScanReport
 (record)        (optional net)     (laya|heuristic)   (aggregate)      (tabs + verdicts)
```

| Piece | Job |
| --- | --- |
| `AppProfile` | Everything a decision may use: title, summary, description, developer, exec line, categories, keywords, store, platform, URLs. |
| `AppResearcher` | Optional online lookup that fills gaps. Merging is one-way: upstream data never overwrites what discovery found installed. |
| `Categorizer` | One verdict per app. `HeuristicCategorizer` reproduces today's behavior; `LayaCategorizer` asks the model. |
| `LibraryScanner` | Runs an engine over the whole library off the async runtime, then aggregates. |
| `ScanReport` | Serializable: per-app verdicts, candidate tabs, and the apps that fit no category. |

## What the model is asked

Laya is a bidirectional encoder, not a generator: it scores a list of candidate
answers rather than writing text. So the prompt is fixed and small — three `noul`
propositions plus one `choice`:

- `is_game` — a video game (not a utility, not a media player)
- `is_emulator` — a console/arcade emulator or retro-game frontend
- `is_watch_service` — a streaming or broadcast video service
- `category` — one of the taxonomy's labels, or an explicit "none of these"

The section comes out of the two boolean traits (`Section::from_traits`); the
category only counts when it agrees with that section. An app that lands on
"none of these" sets `needs_category`, which is the signal that the taxonomy
itself should grow.

Because Laya cannot invent labels, "which categories should exist" is split in
two: `Taxonomy` supplies the candidates, and `LibraryScanner` decides which of
them earned a tab (`CategoryProposal::recommended`, from member count and mean
confidence). The baseline taxonomy encodes this project's opinion about what a
living-room library should look like.

## Cost

`LayaCategorizer::load` downloads a checkpoint from the Hugging Face Hub on first
use and holds it in memory, which is why a scan is one-shot: build the engine,
run the scan, drop it. The scanner runs the decision pass on a blocking worker
and never loads a model when only the heuristic engine is used.

The `laya` feature is on by default and pulls in `candle`. For a fast, hermetic
build — which is what CI and the unit tests use, since no test loads a
checkpoint — use:

```sh
cargo check -p hearthdeck-categorizer --no-default-features
```

`cuda` and `metal` pass through to the respective Laya backends.

## Status

Library only. The service that drives a first full scan, and the button that
triggers a later one, are the next step: they build a `LibraryScanner`, stream
`ScanProgress` out to the UI, and persist the `ScanReport`.
