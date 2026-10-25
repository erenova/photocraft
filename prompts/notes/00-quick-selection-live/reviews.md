# 00 · Quick Selection live: review record

The findings of each review round, so a fresh session can continue without the old conversation.
Findings name files and lines as they were at the reviewed commit; line numbers drift. The
current status of each finding is in `../../tasks/00-quick-selection-live.md` › *Status at
handoff*.

## Round 1 (2026-10-24, branch at 705e11d7 on upstream 14a4ab9b)

Verdict: merge after fixes. A round-2 reviewer verified everything below fixed except M2 (fixed
by a deferral queue that round 2 rejected) and m2 (partly).

| # | Finding | Fix that landed |
|---|---|---|
| B1 | `cargo fmt --all --check` failed | Every commit rustfmt'd |
| B2 | wasm: `gpu_canvas::now_ms()` is a constant 0, so no live growth on the web and the trail vanished; throttle stamped before the grow | A `Throttle` on the frame clock (`ctx.input(i.time)`), interval max(100 ms, 2× last cost), unit-tested |
| M1 | Segments were seeded by their own points and cut by worker timing, so a short drag in a 300 px square selected the square (slow machine) or a 3,196 px band (fast machine), breaking the Photoshop-fitted `quick_select_a_short_drag_takes_what_a_click_does_not` | Cut by travel (max(1.25 × brush, 24) px); each segment also carries the previous segment's travel; test `a_short_drag_takes_the_square_however_it_is_cut` |
| M2 | `flush` grew segments on the UI thread at the next press and before every command (96–335 ms stalls) | A deferral queue, rejected in round 2 (N1–N4) |
| M3 | A document edit mid-drag caused a 2 s synchronous regrow at release and a commit that differed from the preview | Rebase on the worker using a generation counter |
| M4 | The outline was re-traced over the whole visible selection on the UI thread at every update (p90 28.7 ms with Select All + Subtract) | The worker traces with `canvas::outline_key`; p90 1.7 ms |
| m1 | Segment length uncapped | Bounded by travel |
| m2 | Hostile `segments` could freeze the app (100,000 sync segmentations, dead `check`) | `MAX_SEGMENTS = 4096` and `ctx.check()`; headless still uncancellable (round 2) |
| m3 | A worker error fell back to a synchronous regrow | The stroke ends with an error status |
| m4 | A later Replace segment wiped a subtraction | Later Replace segments add |
| m5 | Bench not in `perf/budgets.toml` | P68–P73 registered |
| m6 | `docs/ui-parity.md` history row misplaced | Moved to the top |

## Round 2 (2026-10-25, branch at 3f25fbb2 on upstream b4a297fe)

Verdict: merge after fixes. The live preview is solid (off the UI thread; release == last preview
== journal replay; modes and modifiers right; every gate green except the pre-existing
`layout_bench` panic). The round-1 M2 fix brought in a command-deferral queue with proven defects,
and segments queue without limit.

The reviewer's proof tests (temporary unit tests for `crates/ui-egui/src/quick_select_ui_tests.rs`'s
module) and 24 MP bench script are in `review-2/`. Turn the tests into real regression tests.

### Decision (orchestrator)

**Remove the deferral queue**; don't patch it. While a released stroke still grows or commits,
register it like a job on **its own document**: document-editing commands on that document get the
standard `job_conflict` refusal ("… is still running on this document; wait for it to finish or
cancel it (Esc)"); control callers with `wait` wait for it as `run_waiting` does for jobs; Esc
cancels; other documents and pure view/UI state are unaffected. The stroke always commits on the
document it was drawn on, whichever tab is active, and is never dropped because the active tab
changed. No `{"pending":…}` replies anywhere. Reason: it's the project's existing pattern, smaller,
and it removes N1–N4 at the root; a second dispatch queue inside a Quick Selection PR is unlikely
to be accepted upstream.

### Findings

**Blockers** (both removed by the decision)

- **N1. A deferred command could run on another document while the stroke was silently lost.**
  `quick_select_ui.rs:535-541` dropped a stroke whose document wasn't active, `drop_front`
  (575-581) ran the queue anyway, and `jobs.rs:325-330` let `file.new`/`file.close`/`file.closeAll`
  run at once. Tab clicks and `window.nextDocument` bypass `run_command`. Proof: release on A,
  `select.all` (deferred), `file.new`. A got no `select.quick`; B got A's Select All; status "".
- **N2. Deferred commands escaped the automation authorization gate.** `defer` ran at
  `lib.rs:1305`, before the `automation_command` check at :1318; `run_after` (611-623) ran
  `app.run` with `authorize = None`. Proof: a denied `layer.new.layer` inside `actions.play` ran
  with local authority while a stroke was finishing.

**Major**

- **N3. Deferral broke replies for every caller.** The control channel answered
  `{"pending":true}` for `engine.execute`, `ui.menu.invoke`, even `engine.commands`; errors went
  only to the status bar. UI callers broke too: `canvas.rs:6596-6616` `commit_brush_stroke`
  (withdrew its live offer, re-rendered on the UI thread), `move_box.rs:32` (`layer.pickAt`),
  `slice_ui.rs:331`, `view_cmds.rs:962`. After the fix, every `app.run` caller must get a real
  result or a real error.
- **N4. `jobs::independent_of_document` let history-making `view.*` commands jump the queue**
  (`view.newGuide`, `view.moveGuide`, `view.clearGuides`, `view.colorSamplers.*`,
  `view.clearSlices`): journal `["view.newGuide","select.quick"]`, ⌘Z removed the stroke, and the
  stroke was rebased. Any history-making command is a document edit.
- **N5. Segments queue without limit; the commit trails release by seconds.** At 24 MP with a
  30 px brush, step 16 commits 1.6–1.8 s after release (3.9 s with ⌘D mid-drag); step 32, a normal
  brisk stroke, 4.9–5.8 s (8.8 s with ⌘D). The worker grows segments one after another, only the
  front stroke advances, stale-generation `Task::Grow`s are still grown, Esc can't cancel. Fix: grow
  queued segments' **regions in parallel** (a region depends only on the snapshot and its points,
  `QuickSource::region`; only `combine_region` must run in order, so results stay deterministic);
  put the generation on `Task::Grow` so stale tasks are skipped; let Esc cancel. Target: a brisk
  stroke commits within about 0.5 s of release at 24 MP; add bench rows for step 16/32 and ⌘D at
  step 32, with budgets.
- **N6. P69 can't pass as written.** The bench pointer moves about 351 doc px/s, so with 37.5 px
  segments at most about 9.4 segments/s; compute is about 107 ms per segment (ceiling about 9.3/s).
  Make the P69 scenario meaningful (faster pointer, or measure preview lag behind the pointer);
  parallel growth (N5) is the realistic way to about 10/s. Keep `enforce = false` only if it still
  misses.

**m2 (partly fixed).** Headless (CLI / MCP serve) `select.quick` with up to 4096 hostile segments
runs inline with no cancellation; one call could run about an hour at 24 MP with a 300 px brush
(estimate). Cap total work (segment count × brush area, or total stroke length) generously for
real strokes; absurd strokes → `Err`; test it.

**Minor**

- **N7.** New wasm-only warning: `worker` and `worker_generation` never read
  (`quick_select_ui.rs:234-235`); cfg-gate them so `cargo xtask wasm` is warning-free for ui-egui.
- **N8.** `log/devlog.md` said the bench ran at 2000×1500 (it was 6000×4000); `ui-parity.md`'s
  history row should be dated 2026-10-25.
- **N9.** An `Ok(None)` from a stale-generation worker failure marked the rebased stroke failed
  (`collect`, :410-413); ignore results from old generations.
- **N10.** Invented behaviour to remove: "Stroke dropped: its document is no longer the active
  one" and the deferral (Photoshop commits on release). Size from pen pressure (spec item 6) is not
  implemented; it wasn't before either and isn't an acceptance criterion.

**Nits**

- Squash into 2–3 commits that each pass fmt, clippy and tests: (1) engine: segmented
  `select.quick`; (2) UI: live Quick Selection with modes; (3) optionally the bench, budgets and
  docs. The history had churn (8a81acb2 adds `quick_select_in_stroke`, dac7e609 reverts it; several
  subjects describe replaced designs; the last is a fixup).
- P70 had one 13.4 ms frame against its enforced 16 ms max: little headroom on the VM.

### Round-2 numbers (24 MP, 6000×4000, 4-core VM, release, 30 px brush unless noted)

| Run | Updates/s (median gap) | Longest UI frame | Commit after release | Before: release stall |
|---|---|---|---|---|
| Default drag (step 8) | 8.4 (117 ms) | 4.1 ms | 0.10–0.12 s | 491–495 ms |
| Select All + Subtract | 7.4–8.2 | 4.7 / 13.4 ms | 0.15–0.35 s | |
| Press right after release | 8.4 | 5.7–7.6 ms | 0.29–0.36 s | |
| Command right after release | 8.3–8.4 | 4.5 ms | 0.29–0.30 s | |
| ⌘D mid-drag | 9.3 | 3.4 ms | 0.74–0.78 s | |
| Step 16 (fast pointer) | 9.3 | ≤ 5.5 ms | 1.6–1.8 s; ⌘D 3.9 s | 1.36 s |
| Step 32 (brisk pointer) | 9.3 | ≤ 6.4 ms | **4.9–5.8 s**; ⌘D **8.8 s** | 0.94 s |
| Step 4 (slow pointer) | 4.4 | ≤ 6.7 ms | 0.12–0.29 s | |
| 100 px brush | 2.5 | ≤ 7.4 ms | 0.37–0.79 s | 602 ms |

Gates the round-2 reviewer ran at 3f25fbb2: fmt (on all 10 commits), tests (7,100 passed), clippy,
layers, wasm (new warning N7), panic_hunt, automation `--test it` (78 passed), scorecard
(unchanged) all pass; `cargo xtask perf --quick` exits 1 only from the pre-existing
`layout_bench.rs:175` panic (also on upstream/main).
