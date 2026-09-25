# hearthdeck-ai

Local-model features for the Hearthdeck library. Each functionality lives in its
own module; the shared interface to the model is what they all build on.

| Module | Job |
| --- | --- |
| `ai` | The interface to the model. Load a Laya checkpoint, ask a set of questions about one state, read the answers. Knows nothing about what is being asked. |
| `categorization` | The first functionality: decide what each installed app *is* and which category tabs a library should have. |

A functionality depends on `ai`, never the other way around. Adding one means a
new module beside `categorization` that supplies its own state, questions and
interpretation.

## Shape of a run

```text
AppProfile ──► [AppResearcher] ──► Categorizer ──► LibraryScanner ──► ScanReport
 (record)        (optional net)   (laya|heuristic)   (aggregate)     (tabs + verdicts)
```

| Piece | Job |
| --- | --- |
| `AppProfile` | Everything a decision may use: title, summary, description, developer, exec line, categories, keywords, store, platform, URLs. |
| `AppResearcher` | Optional online lookup that fills gaps. Merging is one-way: upstream data never overwrites what discovery found installed. |
| `Categorizer` | One verdict per app. `HeuristicCategorizer` reproduces the old frontend behavior; `LayaCategorizer` asks the model. |
| `LibraryScanner` | Runs an engine over the whole library off the async runtime, then aggregates. |
| `ScanReport` | Serializable: per-app verdicts, candidate tabs, and the apps that fit no category. |

`LayaCategorizer` is the seam between the two modules: it is a `Categorizer`
(categorization) that runs on a `Laya` (ai).

## What the model is asked

Laya is an encoder, not a generator: it scores a list of candidate answers
rather than writing text. So the prompt is fixed and small — three `noul`
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
confidence).

## Cost

`Laya::load` downloads a checkpoint from the Hugging Face Hub on first use and
holds it in memory, which is why a scan is one-shot: build the engine, run the
scan, drop it. The scanner runs the decision pass on a blocking worker and never
loads a model when only the heuristic engine is used.

The `laya` feature is on by default and pulls in `candle`. For a fast, hermetic
build — which is what CI and the unit tests use, since no test loads a
checkpoint — use:

```sh
cargo check -p hearthdeck-ai --no-default-features
```

`cuda` and `metal` pass through to the respective Laya backends.

## Command line

The binary is a thin dispatcher: one subcommand per functionality, and the
library does the work.

```sh
hearthdeck-ai scan --library <PATH> --output <PATH> [--engine heuristic|laya] ...
```

The daemon spawns exactly that `scan` line as a child process, so a scan's
memory is released when the child exits and nothing stays resident beside the
daemon.
