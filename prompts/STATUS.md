# Task board

The loop (`prompts/LOOP.md`) reads and updates this table. Times are UTC.

| # | Task | Upstream issues | State | Branch | Updated | Notes |
|---|---|---|---|---|---|---|
| 00 | [Quick Selection grows live while dragging](tasks/00-quick-selection-live.md) | #9145, #7266 | in-progress | claude/fix-00-quick-selection-live | 2026-10-25T03:00Z | The owner's main pain. Base upstream/main b07b7fab. §2: api.github.com is 403 from the VM, so I checked only upstream's git log (no fix there); PR #7302 fetched via git as pull/7302/head (open, changes requested), but its reviews can't be read, so I went ahead and will credit it. Review 1: merge after fixes. 2 blockers (fmt; wasm clock is constant 0 so no live growth on web), 4 majors (result depends on worker timing and loses Photoshop's short-drag behaviour; 96-335 ms UI stall on press after release; doc edit mid-drag means a 2 s sync regrow; full outline re-trace per update), 6 minors. layout_bench panic in perf --quick is pre-existing on upstream/main. Review 2: merge after fixes. Round-1 findings verified fixed, but the command-deferral queue added for the post-release freeze has 2 proven blockers (a command lands on another document while the stroke is lost; commands escape automation's permission check) plus lost replies and history steps jumping the queue. Commit trails release by 5-9 s on brisk 24 MP strokes; 8.4 updates/s. Decision: remove the queue and reuse the existing job_conflict refusal (the project's pattern, smaller, fixes N1-N4 at the root); the stroke always commits on its own document; grow segment regions in parallel; squash to 2-3 commits. Fix round 2 of 3 running |
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
