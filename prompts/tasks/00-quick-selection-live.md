# 00 · Quick Selection grows live while you drag, as in Photoshop

- **Upstream issues:** storytold/photocraft#9145 (Quick Selection "feels very erratic and
  sometimes crashes"; Alt-drag moves the window on Linux Mint), #7266 (selection options missing
  from the Quick Selection options bar)
- **Branch:** `claude/fix-00-quick-selection-live`
- **Suggested PR title:** `Quick Selection: the selection grows while you drag, as in Photoshop (#9145)`
- **Time box:** 8 hours of work. This is the owner's top priority: take the time to do it well.

## The report

The fork's owner: "Holding the mouse and dragging doesn't keep selecting a bigger area like in
Photoshop. It's click and leave: the rest of the drag does nothing."

What they're describing is Photoshop's **Quick Selection** tool: you paint over the subject and
the selection grows under the brush as you drag. In Photoshop, the Magic Wand (same `W` slot)
ignores dragging: it selects on the click and nothing else (task 01 covers the wand). In
PhotoCraft today, Quick Selection *does* compute a selection, but only when the mouse button is
released, and during the drag it shows only a faint white trail. So the drag looks dead, which
is exactly the report.

## Photoshop behaviour (the spec)

Sources: Adobe Help, "Paint a selection with Quick Selection tool"
(https://helpx.adobe.com/photoshop/desktop/make-selections/automatic-color-based-selections/paint-a-selection-with-quick-selection-tool.html),
and Adobe's tutorial (https://www.adobe.com/learn/photoshop/web/use-quick-selection-tool). Adobe's
domains may be unreachable from the VM; the points below are what those pages describe.

1. **The selection grows as you paint.** While the button is held, the selection outline updates
   under the brush as the pointer moves and expands to follow the edges of the shape nearby.
   On a big image the outline can lag behind the pointer and catch up; the app never freezes
   (inferred: observed behaviour, not stated as a rule by Adobe).
2. A click without dragging selects a region around the click.
3. Stopping, then clicking or dragging nearby, extends the selection to the new area.
4. **Options bar modes:** *New selection*, *Add to selection*, *Subtract from selection* (three
   buttons; Quick Selection has no Intersect). *New* is the default when nothing is selected.
   **After the first stroke the tool switches itself to *Add*** and stays there.
5. **Modifiers:** hold Alt (Option on macOS) to subtract temporarily; letting go of it returns to
   Add. Shift adds.
6. Brush tip: `[` and `]` shrink and grow it; the options bar's brush picker sets size, hardness
   and spacing, with size from pen pressure or the stylus wheel.
7. Options: *Sample All Layers*; *Auto-Enhance* (PhotoCraft: `enhance_edge`), which smooths the
   boundary and flows it further toward image edges.
8. Cursor: a circle at the tip size with **+** (add) or **−** (subtract) inside it.
9. History: one step per stroke, named "Quick Selection".

## What PhotoCraft does today (upstream `main` at b07b7fab5)

- `crates/ui-egui/src/retouch_ui.rs`, `finish_stroke`, the `Tool::QuickSelection` arm: runs
  `select.quick` **once, at release**, with every point of the stroke. Mode is
  `if mods.alt { "subtract" } else { "add" }`: there is no *New* mode and no auto-switch.
- `crates/ui-egui/src/canvas.rs`: while dragging, only the trail is drawn (search
  `Tool::QuickSelection => Color32::from_white_alpha(40)`).
- Engine: `crates/engine/src/smartselect_cmds.rs`, `quick_selection` (`select.quick`, params
  `{"points":[[x,y],…],"size":px,"mode":"add|subtract|replace","sampleAllLayers","enhanceEdge"}`).
  Algorithm: `crates/algo/src/segment/quick.rs`, `quick_select`.
- **Cost.** Measured 2026-10-24 on a 12-core desktop with
  `cargo run --release -p photocraft-algo --example bench_smartselect` (6016×6016 image): a
  2-point stroke takes **60 ms with a 30 px brush and 330 ms with a 300 px brush**. Expect about
  3× slower on the 4-core VM. Recomputing the whole stroke on every pointer move can't work.

## Upstream work to check first

- **PR #7302** "Selection tools: mode buttons on Quick and Object Selection options bars": open,
  conflicting, and a maintainer requested changes. Read the PR and every review. If it has merged
  by the time you start, build on it. If not, implement the modes here (they're part of this
  spec), keep its good ideas, avoid what the reviewers rejected, and credit it in the PR text.
- PR #8740 mentions a "drag freeze" for AMD on DX12: unrelated to this, but don't fight it.

## What to build

The design is yours; these constraints aren't:

- **Live.** The selection outline updates during the drag, at least about 10 times a second for
  a 30 px brush on a 24 MP image on the VM. The UI thread is never blocked longer than one
  frame (16 ms) by this work. On native, compute off the UI thread using the app's existing
  pattern for background work (find it before inventing one: look at `crates/engine/src/jobs.rs`
  and how live brush strokes run). Drop or supersede stale results. wasm has no threads: there,
  throttle and work in small increments on the UI thread. Never break `cargo xtask wasm`.
- **Incremental.** Each update processes only the stroke's new segment and merges it into the
  stroke's running region (add or subtract), the way Photoshop grows as you go.
- **No jump on release.** What's shown when the button goes up is exactly what gets committed.
- **Still a command.** The commit goes through `select.quick` (AGENTS.md rule 1) as one history
  step, "Quick Selection". Replaying the journal with the same params reproduces the same
  selection. If incremental results differ from one `quick_select` call over all points, define
  `select.quick`'s result as the incremental one (same segment order and chunking), so the live
  preview, the commit and the replay all agree.
- **Modes and modifiers** exactly as in the spec: New/Add/Subtract buttons on the Quick Selection
  options bar (reuse `panels::selection_mode_buttons` if it fits), auto-switch to Add after the
  first stroke, Alt subtracts while held, Shift adds. Read the modifier during the stroke, not
  only at press time, as Photoshop does. Many Linux window managers (MATE, Xfce, Cinnamon) take
  Alt+drag to move the window (#9145), so the *Subtract* button must work without any key, and
  pressing Alt after the button went down must still subtract.
- **Feedback.** While dragging, the outline (marching ants or the app's selection overlay) is
  what shows progress. Keep the brush circle with its +/− badge (`tool_feedback.rs`). Drop the
  faint trail once the live outline replaces it, or keep it only as a stand-in until the first
  result arrives.
- **Never crash.** #9145 reports crashes. Fuzz `select.quick` with NaN, infinite, huge and
  negative coordinates, an empty list, 100,000 points, and sizes of 0 and 1e9, plus the new live
  path with the same inputs. Every panic you find gets a regression test and a fix.

## Acceptance criteria

- [ ] A harness test in ui-egui drags Quick Selection across a synthetic two-colour image and
      asserts that a selection exists **before** release and is larger at a later mid-drag frame.
- [ ] Released selection == the last live preview == replaying the `select.quick` journal entry.
- [ ] New → Add auto-switch; Alt subtracts (pressed before *or* during the drag); the Subtract
      button works with no key; Shift adds; undo removes the whole stroke in one step.
- [ ] Bad params → `Err`, never a panic; `panic_hunt` stays green.
- [ ] Numbers in the PR text: preview update rate and longest UI-thread stall during a drag on a
      24 MP image, before and after (AGENTS.md rule 8).
- [ ] Snapshot PNGs (offscreen `snapshot` example or the control channel's `ui.screenshot`) of a
      stroke mid-drag and after release, on a public-domain image.
- [ ] `docs/ui-parity.md` (and `docs/gaps.md` if it lists this) updated, with the date.

## Gates

```sh
cargo test -p photocraft-algo -p photocraft-engine -p photocraft-ui-egui
cargo clippy -p photocraft-algo -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo test -p photocraft-engine --test panic_hunt -- --ignored
cargo xtask perf --quick
cargo xtask scorecard   # only if a scorecard row or budget moves; commit docs/scorecard.md
```

## Out of scope

Magic Wand (task 01). Object Selection, Select Subject and the quality of the segmentation
itself, unless the live path exposes a bug in it.

## Status at handoff (2026-10-25, session stopped by the owner)

**Where things are**

- Branch `claude/fix-00-quick-selection-live` on origin is at **3f25fbb2**: 10 commits on upstream
  b4a297fe. It is the reviewed round-2 state. The selection grows live off the UI thread, release
  == last preview == journal replay, New/Add/Subtract and Alt/Shift work, and every gate passes
  except the pre-existing `layout_bench` panic. It is **not ready for a PR**, because its
  command-deferral queue has the round-2 blockers (N1 wrong document, N2 authorization bypass).
- Review history, every finding and the 24 MP numbers: `../notes/00-quick-selection-live/reviews.md`.
  The round-2 reviewer's proof tests and bench script are in `../notes/00-quick-selection-live/review-2/`.
- **Round-2 work in progress, uncommitted when stopped:**
  `../notes/00-quick-selection-live/round-2-wip.patch` (15 files, +1139/−446, binary-safe). It
  applies cleanly on 3f25fbb2 and reproduces the stopped working tree exactly. The implementer's
  notes on it are the last entry of `../notes/00-quick-selection-live/devlog.md`. It was kept out
  of the branch so that the branch stays at a reviewed, green state.

**What the patch does** (from the implementer's notes; unverified by a reviewer)

- **Queue removed.** A released stroke commits as `select.quick` on its own document. While
  segments still grow, it runs as a background job named "Quick Selection" (`QuickCommit::growing`).
  The document is locked by the standard `job_conflict`; Esc / `jobs.cancel` cancel it; there are no
  `{pending}` replies. A press while that job runs chains the next stroke onto the committed result.
- **`jobs::independent_of_document`:** `view.*` and `gradient.presets.apply` no longer count as
  document-independent (N4).
- **Worker moved** to `crates/ui-egui/src/quick_select_ui/worker.rs`. Queued segments' regions grow
  in parallel and combine in order; regions are cached across a rebase that keeps the pixels (⌘D).
- **Work caps:** `max_segments(size)` (2^28 px² of windows) and `MAX_POINTS` 250k (m2).
- **Algo:** a new `quick_select_speculating` that must give the same region as `quick_select`.
  Verify that the algo tests' pinned results are untouched.
- **24 MP, 30 px, commit after release** (before → after):

  | Drag | Before | After |
  |---|---|---|
  | step 8 | 102–119 ms | 136–220 ms |
  | step 16 | 1.6–1.8 s | 0.34–0.52 s |
  | step 32 | 4.9–5.8 s | 2.1–4.3 s |
  | ⌘D at step 32 | 8.8 s | 2.1–4.7 s |

  Longest UI frame ≤ 8.4 ms. Parallel regions reach only about 1.6× on the VM's 3 worker threads,
  so the 0.5 s target at step 32 isn't reachable here.

**Next steps, in order**

1. Bootstrap (LOOP.md §0). Then `git switch -C claude/fix-00-quick-selection-live
   origin/claude/fix-00-quick-selection-live` and `git apply ../pc-loop/prompts/notes/00-quick-selection-live/round-2-wip.patch`.
2. Build and test the touched crates (`CARGO_INCREMENTAL=0 cargo test -p photocraft-algo -p
   photocraft-engine -p photocraft-ui-egui --lib --tests`) and fix what fails. The patch's gates
   were not run when the session stopped.
3. Finish round 2 against `reviews.md`:
   - turn `review-2/review_tests.rs` into real tests (N1–N4 must now pass);
   - N5: decide honestly what the VM can reach; record it and budget it, with `enforce = false` if
     it misses;
   - N6: make P69's scenario meaningful;
   - N7: the wasm warning;
   - N8: docs dated 2026-10-25;
   - N9;
   - investigate one bench run that showed a 1.16 s gap between outline updates at step 16;
   - update docs (`control-protocol.md` job behaviour, `quick-selection-performance.md`,
     `ui-parity.md`) and regenerate `docs/scorecard.md`.
4. Squash into 2–3 commits that are each green (engine; UI; bench, budgets and docs). Rebase onto
   the newest upstream, run the full gates, and `git push --force-with-lease`.
5. **Review round 3**, the last allowed by LOOP.md §5: a fresh reviewer, given `reviews.md` for
   context. If the gates still fail after it, push and mark the row `blocked`.
6. Ship (LOOP.md §6): update the draft `../prs/00-quick-selection-live.md`, retake the screenshots
   if the UI changed, and set `ready-for-pr`.
