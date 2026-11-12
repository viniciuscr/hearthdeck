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
  work belongs in tracked issues or an explicitly maintained integration roadmap,
  such as `retroarch-integration.md`. Delete a stale line rather than
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
| [`frontend-rewrite.md`](frontend-rewrite.md) | COSMIC frontend boundaries, shared widgets, screen state and design decisions |
| [`backend-architecture.md`](backend-architecture.md) | Daemon/bridge shape, discovery providers, catalog flow, API, trust boundaries |
| [`units-and-logs.md`](units-and-logs.md) | systemd user-unit traps, the logging pipeline, runbooks |
| [`observability.md`](observability.md) | Logging model, events, investigation order |
| [`metadata-enrichment.md`](metadata-enrichment.md) | Metadata sources, priority, freshness |
| [`appearance-system.md`](appearance-system.md) | Theme tokens, backdrops, accessibility, persistence |

## Decisions and roadmaps (living)

| Doc | Owns |
| --- | --- |
| [`input-ownership.md`](input-ownership.md) | Controller broker, keyboard handoff, host acceptance and recovery |
| [`retroarch-integration.md`](retroarch-integration.md) | RomM/RetroArch decisions, `romm.service` startup, open questions, phased roadmap |
| [`stremio-integration.md`](stremio-integration.md) | Stremio account linking, the verified `api.strem.io` shapes, Continue Watching rail, credential storage decision |
| [`settings-window.md`](settings-window.md) | Settings as a GPUI process: why the split, the Back/focus contract, the two rollbacks, packaging |
| [`spikes/gpui-progress.md`](spikes/gpui-progress.md) | gpui/gpui-kit verdict: component quality, crate cost, and why it cannot be dropped into an iced screen |

## Reference and process

| Doc | Owns |
| --- | --- |
| [`arch-package.md`](arch-package.md) | Current minimal COSMIC session, Arch package contents and troubleshooting |
| [`linux-acceptance.md`](linux-acceptance.md) | Manual acceptance checklist |
| [`immich-backup.md`](immich-backup.md) | Phone photo/video backup: Podman Quadlet stack, external SSD layout, upgrades, restore, reinstall, pre-wipe check |
| [`romm-backup.md`](romm-backup.md) | RomM database dump, external SSD layout, machine-wipe restore, disk-loss limits |
| [`backup-settings.md`](backup-settings.md) | Off-site rclone backup: package/user split, safety rules, and the Backup page's contract |
| [`provider-credentials.md`](provider-credentials.md) | Cloud provider credentials: who owns the secret, the 0600 store, pCloud `getauth`, and why rclone's config stays empty |
| [`cloudflare-tunnel.md`](cloudflare-tunnel.md) | Public HTTPS access to Immich: Cloudflare Tunnel Quadlet, token handling, boot behaviour, reinstall from the repository |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md), [`../DEVELOPER_QUICKSTART.md`](../DEVELOPER_QUICKSTART.md), [`../PRE_PUSH_WORKFLOW.md`](../PRE_PUSH_WORKFLOW.md) | Build, dev, and PR flow |
| [`../.github/copilot-instructions.md`](../.github/copilot-instructions.md) | Project map and the summary style for changes |
| `archive/` | Superseded docs, kept for history only: the pre-rewrite frontend review, the retired Gamescope session (`archive/kiosk-session.md`), and the closed settings UI roadmap. Never a source of current truth. |
