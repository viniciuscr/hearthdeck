---
description: Always-on minimal-token behavior with Serena-first execution.
---

# Token Efficiency Defaults

- Invoke the `ponytail` skill for coding tasks unless the user explicitly says `stop ponytail` or `normal mode`.
- Keep output minimal by default: code first, then only short necessary notes unless the user asks for detail.
- Do not do optional work (internet research, commit-style archaeology, broad repo audits) unless the user explicitly asks for it or it is required to unblock the task.
- Prefer Serena tools first for code navigation and edits (`find_symbol`, `get_symbols_overview`, `replace_symbol_body`, `replace_content`); use non-Serena tools only when Serena is not a fit.
- Run only the smallest existing validation command that covers the changed behavior.
- Avoid speculative abstractions and dependency additions; reuse existing code, stdlib, and platform features first.
- Keep edits in the main model by default; delegate to cheaper agents only for mechanical, independent, low-ambiguity tasks (e.g. search sweeps, test/lint runs, data collection).
- Do not force delegation as policy; delegate only when it is clearly cheaper/faster than direct execution.
- Commit workflow must be zero-research: do not inspect git history for style.
- Commit message must be short and direct: one subject line describing what was implemented in this interaction.
