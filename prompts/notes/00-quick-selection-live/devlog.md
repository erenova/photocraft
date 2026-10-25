## 2026-10-24 · Quick Selection grows while dragging (#9145, #7266) — branch claude/fix-00-quick-selection-live

- Engine `quick_stroke.rs`: `select.quick` takes `segments` (point counts, optional per-segment mode); each
  segment grows on its own from the previous one's last point and is combined into the selection in order;
  a stroke replaces once. No `segments` = one segment = old behaviour (test pins it). `Session::offer_live_quick`
  lets the commit take the live result (`fromLive`).
- UI `quick_select_ui.rs`: live stroke state on `PhotocraftApp::quick_live`; segments grow on a std thread
  inside a dedicated rayon pool (n-1 threads) so the UI thread's own rayon work never queues behind cuts;
  inline when `background_jobs` is off (tests) and on wasm (throttled 100 ms). Canvas outlines `shown()`;
  release commits once caught up (`tick`), `flush` commits before any other command (`run_command`) or new press.
- Modes: `UiState::quick_selection_mode` (0 New, 1 Add, 2 Subtract), own buttons before the brush chip; New→Add
  after a stroke; ⌥/⇧ read per segment (mode change closes the open segment). `ui.set {quickSelectionMode}`.
- Still open: Photoshop's exact Shift+Alt behaviour on Quick Selection unknown (we subtract); the outline is
  traced on the UI thread (measured in the bench); Object Selection's click fallback still one-shot.

## 2026-10-25 · Quick Selection live strokes, review round 1 (#9145) — branch claude/fix-00-quick-selection-live

- Cut: deterministic by pointer travel (`quick_stroke::Cutter`: press alone, then every
  `max(1.25·size, 24)` px, mode change closes a segment, ≤ 4096 segments); each segment is led by the
  run's last segment of same-class travel, so a short drag takes the 300 px square however it is cut
  (test). A later Replace segment adds; a cap + `ctx.check` per segment on replay.
- UI: one worker thread per stroke (Reset/Grow channel) in the n-1 rayon pool; it also traces the
  outline (`canvas::outline_key` cache key) so the UI thread only swaps textures. A revision change
  mid-drag rebases the stroke on the worker (generation counter). After release nothing waits:
  document-editing commands are deferred behind the pending commit (`jobs::independent_of_document`
  ones run now), a new press chains, `ui.pointer` replies once committed. Worker failure ends the
  stroke with an error (no sync fallback). Web: `Throttle` on the frame clock (interval ≥ 2× last grow).
- Bench `quick_selection_bench` registered (perf/budgets.toml P68–P73). 2000×1500, 30 px: ~8 updates/s
  (median gap 118 ms; P69 100 ms not enforced), longest UI frame ≤ 6 ms in every scenario, commit
  0.32–0.37 s after release (was a 455 ms UI stall + zero live updates).
- Still open: update rate CPU-bound by segment cost with the lead overlap (67 ms/segment without it);
  Deselect's own 200 ms full refresh; layout_bench panic in `xtask perf --quick` is preexisting on main.
  The brush circle is an OS cursor bitmap, so offscreen snapshots don't show it.

## 2026-10-25 · Quick Selection, review round 2 (WIP, session ended) — branch claude/fix-00-quick-selection-live

- Deferral queue removed. A released stroke is committed by `select.quick` started at once on the
  stroke's document (`on_document` activates it for the dispatch): with segments still growing the
  engine runs it as a background job "Quick Selection" whose work is the live worker's `Finish`
  continuation (`QuickCommit::growing`); the doc is locked by the standard job_conflict, Esc /
  `jobs.cancel` cancel it, `on_job_event` (hooked in `jobs_ui::on_event`) reports it and lets the
  next stroke go; no modal progress dialog for it (`quick_select_ui::commits`). No `{pending}`
  replies anywhere (`defer`, `run_after`, `committing` gone).
- A press while a stroke's job runs: the new stroke waits (src None) until the doc has no job and
  no earlier stroke on it, then starts from the committed result (chained, no UI stall).
- `jobs::independent_of_document`: `view.` prefix and `gradient.presets.apply` no longer count as
  document-independent (guides, slices, ruler origin, gradient fill layers are history steps).
- Worker (`quick_select_ui/worker.rs`): regions of queued segments grow in parallel on the n-1
  pool (`pool.spawn` per region), combined in order; pixel epoch + in-flight dedupe; regions cached
  across a rebase that kept the pixels (⌘D); speculation 1 when segments wait behind others.
- Engine: `QuickStroke::plan`/`apply`, `grow_regions`/`grow_segments` (parallel chunks, ctx.check),
  `QuickSource::on/reads/same_pixels`, `max_segments(size)` work cap (2^28 px² of windows) and
  `MAX_POINTS` 250k, Cutter capped by `max_segments`. Algo: `quick_select_speculating` (same region).
- 24 MP, 30 px, 4-core VM, commit after release (before → after): step 8 102–119 → 136–220 ms;
  step 16 1580–1830 → 340–520 ms; step 32 4927–5815 → 2130–4250 ms; ⌘D at step 32 8841 →
  2140–4680 ms. Longest UI frame ≤ 8.4 ms. Parallel regions only reach ~1.6× on 3 threads
  (experiment: 61 segments 6.7 s sequential → 4.1–4.6 s parallel), so 0.5 s at step 32 is not
  reachable deterministically here.
- Still open: squash into 2–3 commits; docs (control-protocol, quick-selection-performance,
  ui-parity, module docs checked), scorecard regen, wasm check, panic_hunt, automation `it`,
  perf --quick full; run 3 of the bench showed only 4 outline updates at step 16 (one 1.16 s gap):
  investigate (dedupe/speculation or VM noise).
