# Hearthdeck docs

The map: what each doc owns, and when to read it. The code is the truth; these
docs carry the *why* the code cannot.

## Rules

- **One doc owns one subject.** If a change alters a decision recorded here, update
  the owning doc in the same change. A doc that no longer matches the code is worse
  than no doc, because it misleads the next decision.
- **Decision first, then why, then traps.** Lead with what we do and the reason.
  Do not restate code; link the file when the reader needs the detail.
- **No unmaintained "current state" or "next step" sections.** They rot. Forward
  work lives only in `code-review-roadmap.md` (findings, debt) and
  `retroarch-integration.md` (phased roadmap). Delete a stale line rather than
  keeping it "just in case".
- **Prefer the short form.** A trap that cost an afternoon is worth five lines, not
  a page. If a section does not change a decision, it does not belong here.

## Invariants (must hold; every feature follows them)

| Doc | Owns |
| --- | --- |
| [`product-foundations.md`](product-foundations.md) | Product promise, controller/Back contract, focus model, catalog + metadata model, resource-discipline rules |

## How it works, and why (mechanism + rationale + traps)

| Doc | Owns |
| --- | --- |
| [`frontend-animations.md`](frontend-animations.md) | iced's animation model, the three transitions, motion tokens, the renderer layer-order trap |
| [`frontend-library.md`](frontend-library.md) | The library grid: RomM scopes, prefetch, display order, counts, virtualization, artwork path |
| [`backend-architecture.md`](backend-architecture.md) | Daemon/bridge shape, discovery providers, catalog flow, API, trust boundaries |
| [`kiosk-session.md`](kiosk-session.md) | Session topology, autologin, launch model, the full incident writeup and the do-not list |
| [`units-and-logs.md`](units-and-logs.md) | systemd user-unit traps, the logging pipeline, runbooks |
| [`observability.md`](observability.md) | Logging model, events, investigation order |
| [`metadata-enrichment.md`](metadata-enrichment.md) | Metadata sources, priority, freshness |
| [`appearance-system.md`](appearance-system.md) | Theme tokens, backdrops, accessibility, persistence |

## Decisions and roadmaps (living)

| Doc | Owns |
| --- | --- |
| [`retroarch-integration.md`](retroarch-integration.md) | RomM/RetroArch decisions, `romm.service` startup, open questions, phased roadmap |
| [`code-review-roadmap.md`](code-review-roadmap.md) | Phased review findings and known debt |

## Reference and process

| Doc | Owns |
| --- | --- |
| [`arch-package.md`](arch-package.md) | Arch package contents and troubleshooting |
| [`linux-acceptance.md`](linux-acceptance.md) | Manual acceptance checklist |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md), [`../DEVELOPER_QUICKSTART.md`](../DEVELOPER_QUICKSTART.md), [`../PRE_PUSH_WORKFLOW.md`](../PRE_PUSH_WORKFLOW.md) | Build, dev, and PR flow |
| [`../.github/copilot-instructions.md`](../.github/copilot-instructions.md) | Project map and the summary style for changes |
| `archive/` | Superseded docs, kept for history only. Never a source of current truth. |
