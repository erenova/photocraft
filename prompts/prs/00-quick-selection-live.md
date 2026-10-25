<!-- DRAFT, not ready: task 00 is mid review round 2 (see tasks/00-quick-selection-live.md ›
Status at handoff). Update every TODO, the numbers and the commit list before setting the row to
ready-for-pr. Delete this comment then. -->

# Quick Selection: the selection grows while you drag, as in Photoshop (#9145)

## Summary

Quick Selection computed nothing until the mouse button went up: during the drag it drew only a
faint trail, so "holding the mouse and dragging doesn't keep selecting a bigger area like in
Photoshop" (#9145). Now the selection grows under the brush while you paint, off the UI thread,
and what you see when you let go is exactly what gets committed, as one `select.quick` history
step that replays to the same selection. The options bar gains Photoshop's New / Add / Subtract
buttons (#7266).

## Photoshop behaviour

Sources: Adobe Help, "Paint a selection with Quick Selection tool"
(https://helpx.adobe.com/photoshop/desktop/make-selections/automatic-color-based-selections/paint-a-selection-with-quick-selection-tool.html),
and Adobe's tutorial (https://www.adobe.com/learn/photoshop/web/use-quick-selection-tool).

- The selection grows as you paint and follows nearby edges; a click selects around the click;
  painting again nearby extends it.
- Options bar: New, Add, Subtract (no Intersect). New is the default and the tool switches itself
  to Add after the first stroke.
- Alt (Option) subtracts while held; Shift adds.
- One history step per stroke, "Quick Selection".

Inferred (not stated by Adobe): the outline may lag the pointer on large images and catch up;
Shift+Alt subtracts; Alt pressed mid-drag subtracts from that point of the stroke on.

## What changed

- **Engine:** `select.quick` accepts an optional `segments` param that records how a stroke was
  cut. A stroke is cut by pointer travel (max(1.25 × brush, 24) px), not by timing, and each
  segment also carries the previous segment's travel. So the same hand motion gives the same
  selection on any machine and in replay, and a short drag still takes a whole object as the
  Photoshop-fitted algorithm intends. Without `segments` the result is byte-identical to before;
  old journals replay unchanged. The live result is handed to the commit through
  `Session::offer_live_quick`, the existing `offer_live_*` pattern.
- **UI:** segments grow on a worker (regions in parallel, combined in order) and the worker also
  traces the outline, so the UI thread only swaps textures. On the web (no threads) segments grow
  inline, throttled on the frame clock.
- **After release:** while a stroke still grows it commits as a background job on its own
  document. Edits to that document get the usual "still running; wait or Esc" refusal, Esc
  cancels, other documents are unaffected. TODO: confirm against the final code.
- **Modes:** `quick_selection_mode` in `UiState` (New / Add / Subtract buttons, auto-switch to Add,
  modifiers read for each segment, so the Subtract button works with no key, which helps on Linux
  window managers that take Alt+drag); `ui.set {quickSelectionMode}` documented.
- **Limits:** hostile `segments` / point lists are capped and cancellable (`Err`, never a freeze).
- **Bench:** `quick_selection_bench` under `xtask perf` (P68–P73, TODO: final rows), and
  `docs/quick-selection-performance.md`, `docs/ui-parity.md` updated.

Builds on PR #7302 by its author (mode buttons on the Quick Selection bar, New replacing once),
which is conflicting with main. Unlike it: no Intersect, New doesn't keep replacing across strokes,
and the state has its own field. Its reviews couldn't be read from where this was written.

## Tests

- Engine (`quick_stroke/tests.rs`): one segment == the old one-shot at 8/16/32-bit; live ==
  commit == replay with a mid-stroke subtract; a short drag takes the square however the stroke is
  cut; the cut follows travel, not timing; replace-once; later Replace adds; mismatched offers are
  ignored; hostile params never panic; work caps. TODO: final list.
- UI (`quick_select_ui_tests.rs`): a harness drag on a two-colour image has a selection before
  release that grows mid-drag; release == last preview == replay, one undo step; New → Add; Alt
  before or during the drag; Subtract with no key; Shift adds; an edit mid-drag; nothing waits
  after release; a stroke always commits on its own document; automation authorization holds; the
  web throttle; hostile pointer input. TODO: final list.

## Numbers

24 MP (6000×4000), 4-core VM, release, `cargo run --release -p photocraft-ui-egui --example
quick_selection_bench`. TODO: replace with the final run.

| | Before | After (round 2 state) |
|---|---|---|
| Outline updates during a drag (30 px) | 0 | about 8.4/s |
| Longest UI-thread frame | 491–495 ms (at release) | ≤ 8.4 ms |
| Commit after release, normal drag | (the stall above) | 0.14–0.22 s |
| Commit after release, brisk drag | | TODO |

## Screenshots

`prompts/prs/00-quick-selection-live/mid-drag.png` (outline partway along the wave crest, New
selected) and `after-release.png` (larger, bar switched to Add). The Great Wave, public domain.
Offscreen snapshots don't show the OS brush cursor.

## Notes for reviewers

`cargo xtask perf --quick` exits 1 on main too: `layout_bench.rs:175` panics ("the layer is
already a smart object"), unrelated to this change.

Fixes #9145
Fixes #7266

🤖 Generated with [Claude Code](https://claude.com/claude-code)
