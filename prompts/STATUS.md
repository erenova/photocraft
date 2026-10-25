# Task board

The loop (`prompts/LOOP.md`) reads and updates this table. Times are UTC.

**Handoff 2026-10-25:** the first cloud session stopped at the owner's request. Task 00 is paused mid review round 2; resume it first (row 00). Nothing else was started.

| # | Task | Upstream issues | State | Branch | Updated | Notes |
|---|---|---|---|---|---|---|
| 00 | [Quick Selection grows live while dragging](tasks/00-quick-selection-live.md) | #9145, #7266 | todo | claude/fix-00-quick-selection-live | 2026-10-25T03:54Z | **Resume first**: paused mid review round 2 when the owner stopped the session. Branch at 3f25fbb2 (reviewed, gates green, but its command-deferral queue has round-2 blockers, so not PR-ready). Round-2 WIP saved as notes/00-quick-selection-live/round-2-wip.patch (applies cleanly on 3f25fbb2). Next steps: task file › Status at handoff. Review round 3 is the last allowed |
| 01 | [Magic Wand matches Photoshop](tasks/01-magic-wand-parity.md) | #4902, #9145, #5775 | todo | | 2026-10-24T19:00Z | The owner's corner-clicking workflow |
| 02 | [Background removal works end to end](tasks/02-background-removal-workflow.md) | (audit) | todo | | 2026-10-24T19:00Z | Audit; adds new tasks for big gaps |
| 03 | [Remove Background cuts out product shots](tasks/03-remove-background-products.md) | #9145, #2788 | todo | | 2026-10-24T19:00Z | Model-free path |
| 04 | [Repeated pastes don't crash; recovery is current](tasks/04-paste-crash-recovery.md) | #7582 | todo | | 2026-10-24T19:00Z | Critical upstream |
| 05 | [Smart filters reopen, edit and delete again](tasks/05-smart-filters-reopen.md) | #9055 | todo | | 2026-10-24T19:00Z | Regression since 0.13 |
| 06 | [Convert to Smart Object acts on the layer it's given; perf --quick passes](tasks/06-convert-smart-object-target.md) | (none filed) | todo | | 2026-10-25T03:20Z | Found while running task 00's gates: layout_bench panics on upstream/main, likely smart_cmds.rs:1385 checking the active layer instead of the `layer` param |

## States

| State | Meaning | Who sets it |
|---|---|---|
| `todo` | not started | loop, owner |
| `in-progress` | a session is working on it (the lock; stale after 4 hours) | loop |
| `blocked` | can't be finished here; the note says why | loop |
| `ready-for-pr` | branch pushed, PR text in `prs/NN-slug.md` | loop |
| `pr-open #N` | the owner opened upstream PR #N | owner or loop |
| `changes-requested` | upstream review asked for changes, or the PR conflicts | loop |
| `merged` | upstream merged it | loop |
| `upstream-fixed` | someone else fixed it upstream first | loop |
| `dropped` | not worth doing, or not Photoshop behaviour; the note says why | loop, owner |
