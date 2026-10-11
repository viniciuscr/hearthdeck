---
name: reporting
description: "How to write the final report after finishing a Hearthdeck task: short, plain, and only what the diff cannot say. Load before writing any summary, wrap-up, status message, or PR body. Activates on final report, summary, wrap-up, status, 'what changed', PR description, and at the end of every task."
---

# Reporting: short, plain, no padding

## CRITICAL RULE: the default report is a handful of lines

If it reads like a changelog, an essay, or a consultant's deck, it is wrong. The diff already
holds everything. The report holds only what the diff cannot say.

**Budget: 3–6 lines.** A heading, a table or a bullet list has to be earned — by a decision the
reader must make, a tradeoff they should weigh, or a finding that changes the plan. Nothing else
justifies one.

## What it contains

1. **What changed, and where** — one line.
2. **The one non-obvious thing** — what a reader of the diff would get wrong. Skip it if there
   genuinely is not one.
3. **How it was proved** — the command, and its result.

That is the whole report. If a commit message already says something, do not say it again.

## What never goes in

- A restatement of the diff, hunk by hunk.
- A list of the commits.
- Background on the architecture, the file, or the language — unless the change is unintelligible
  without it.
- A "next steps" or "what I'd suggest" section nobody asked for.
- Anything you would not say out loud to a colleague across a desk.

## WRONG / CORRECT

WRONG — headings, tables and several hundred words, for a one-line fix:

> ## Summary
> I updated `grid.rs` to fix the scroll behaviour. The library grid renders the catalog, which is
> fetched from the daemon... [and on, for several screens]

CORRECT:

> The grid now follows the cursor when the pad walks out of the viewport — `grid.rs:93`. The bug
> was that `grid_scroll_offset` was only ever *written* by the scrollable reporting where it was,
> never by a step, so the cursor walked off the screen while the content sat still.
> `Metrics::scroll_to_row` computes the offset now, and the new test fails without it.
> `just check-fast hearthdeck-frontend`: green, 248 tests.

## Teaching is allowed one sentence, not a section

Explaining a Rust construct is worth it only when that construct *is* the crux of the change — the
`fn` pointer in `button_class`, the offset recorded before the scrollable reports it. Say it in a
sentence and move on.

## Length is for the reader, not the writer

Less is more is not modesty, it is the mechanism: a wall of text buries the one line that mattered,
and the reader stops reading. When in doubt, cut the paragraph.
