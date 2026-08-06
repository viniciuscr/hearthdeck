---
name: token-saver
description: Strict low-token execution mode for coding tasks: no overthinking, no optional detours, Serena-first.
---

# Token Saver

Use this skill when the user wants the shortest path to a correct solution with minimal chatter and no unnecessary investigation.

## Rules

1. Solve only the requested scope (YAGNI).
2. Reuse existing repo helpers first, then stdlib/platform, then existing dependencies.
3. Skip optional internet research and exploratory scans unless explicitly requested or required.
4. Use Serena tools first for symbol discovery and edits; avoid whole-file reads unless necessary.
5. Keep responses short and actionable; do not add long explanations unless requested.
6. Validate with the narrowest existing test/lint/build command that covers the change.
7. Main model does implementation/iteration by default; use cheaper agents only for mechanical, independent, low-ambiguity work.
8. Never force delegation globally; delegate only when it is a clear cost/time win.

## Quick defaults

- Default simplification mode: `ponytail full`.
- One-question rule: ask at most one clarifying question only when blocked by ambiguity.
- No commit-style lookup unless the user asks to commit.

## Example

- Request: "Fix this null crash in parser."
- Apply: inspect call path, fix shared root cause once, run targeted test, return short diff summary.
