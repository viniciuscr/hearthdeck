---
name: reporting
description: "How to write the final report after finishing a Hearthdeck task: a fixed five-line block, plain words, only what the diff cannot say. Load before writing any summary, wrap-up, status message, or PR body. Activates on final report, summary, wrap-up, status, 'what changed', PR description, and at the end of every task."
---

# Reporting: one block, always the same shape

## CRITICAL RULE: use this block, in this order, with these labels

```
Done:     what changed, and where. One line.
Watch out: the one thing a reader of the diff would get wrong. Drop the line if there is none.
Checked:  the command you ran, and its result.
Next:     what comes next. Drop the line if nothing does.
```

Nothing outside those labels. No headings, no tables, no bullet lists, no numbered
steps, no "summary" paragraph before the block, no sign-off after it.

The labels are the whole point: they are always the same and always in that order,
so the reader knows where to look before they read. A wall of prose has to be read
to be understood; this does not.

## Filling it in

- **Done** — the change and the file. Say what it does, not which functions you
  touched. `Done: the grid follows the cursor now — grid.rs.`
- **Watch out** — the trap. A behaviour that changed, a decision you made that a
  reader would otherwise assume was accidental, a thing that looks wrong but is
  deliberate. `/tmp`-style asides do not belong here.
- **Checked** — the real command and the real result. `just check-fast
  hearthdeck-frontend: 257 pass.` If you did not run one, write `Checked: not run`
  and nothing else.
- **Next** — one line, only if genuine work remains.

Plain sentences. No "leveraging", no "comprehensive", no restating the ask back.

## When you are stuck, unfinished, or wrong

Same block. Say it in the same place:

```
Done:     nothing — I reverted it.
Watch out: the tree is mid-edit and does not compile; do not build on it.
Checked:  not run.
Next:     redo X starting from Y.
```

Never dress a failure up as progress, and never bury it under what did work.

## WRONG / CORRECT

WRONG — everything below is padding. It buries the two facts that matter:

> ## Summary
> I updated the dashboard so that it renders its own top bar. The dashboard is a
> sibling of the library rather than a page inside it, which matters because...
>
> | File | Change |
> |---|---|
> | `top_bar.rs` | new |
> | `dashboard/mod.rs` | removed the old bar |
>
> I also renamed `dashboard_nav_button_class` to `nav_button_class` since...
> Let me know if you'd like me to also wire the system status next.

CORRECT:

```
Done:     the dashboard and settings screens share one top bar — widgets/top_bar.rs,
          drawn by both. Settings had no chrome at all before this.
Watch out: the class rename to `nav_button_class` also moved the old app's two uses.
Checked:  just check-fast hearthdeck-frontend — 257 pass, clippy clean.
Next:     the wifi/bluetooth icons in the bar's right-hand space.
```

## Length is the symptom, shape is the cause

Four labels that do not move beat any amount of "be concise". When in doubt, cut
the clause, not the label.
