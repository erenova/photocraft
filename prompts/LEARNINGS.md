# Learnings

Short, durable lessons for the next agent: things that cost time once and shouldn't again. Read
this before every task; add at most 3 lines after each one; keep it under 150 lines by merging and
deleting stale lines. Newest first within each section.

## Environment

- Disk is the tightest resource (about 4 cores, 16 GB RAM, about 30 GB of disk). Build only in the
  session's own checkout, always with `CARGO_INCREMENTAL=0` (incremental data reached 11 GB).
  `target/debug/examples` grows to about 9 GB once the full `cargo test -p photocraft-ui-egui`
  builds the examples; `rm -rf target/debug/examples` whenever free space is under 6 GB (no cargo
  running). Never a full `cargo clean`: a cold build of algo+engine+ui-egui tests takes about 30
  minutes, a ui-egui lib test build about 6, a release ui-egui example about 10.
- The environment's setup script and variables only apply if the session was started with that
  environment. If `dpkg -s libgtk-3-dev` fails or `rustup target list --installed` lacks wasm32,
  run the README's setup script by hand once (it works as root) and note it on the board.
- `api.github.com` answers 403 from the VM, with or without `gh`, so upstream issues, PRs and
  reviews can't be read. `git fetch upstream` works, and so does fetching a PR's commits:
  `git fetch upstream pull/<N>/head:refs/remotes/upstream/pr-<N>`.
- Bash calls time out (2 minutes by default, 10 at most): run `cargo build`/`cargo test` in the
  background and wait for the notification. Never `pkill -f` a pattern that matches your own
  command line (it kills your shell); kill by PID.
- Release builds can fail on stale build-script outputs that point at another worktree;
  `cargo clean -p <crate> --release` fixes it. A `git worktree` of upstream/main can reuse the
  build cache with `CARGO_TARGET_DIR=/home/user/photocraft/target`.
- No GPU: wgpu falls back to Mesa's lavapipe/llvmpipe. Tests that need a real adapter skip or fail
  on the VM; say so in the PR text rather than hiding it. Offscreen snapshots don't show OS cursor
  bitmaps (the brush circle, `tool_cursor.rs`).

## Loop mechanics

- `log/devlog.md` and the session scratchpad die with the container. Before a session stops, copy
  what the next one needs (review proofs, dev log entries, WIP patches) to `prompts/notes/NN-slug/`
  and write the task file's *Status at handoff*.
- An implementer's own gate run isn't enough: a fresh reviewer found 2 blockers and 4 majors on a
  branch reported "all gates green" (fmt not run on every commit; a wasm clock that is a constant
  0). Always run the review round, and have reviewers measure at 24 MP themselves.
- While an implementer edits the checkout, the stop hook nags about uncommitted changes every turn:
  expected, say so in one line. Subagents save PNGs under `prompts/prs/NN-slug/`; a retake of a
  committed PNG blocks `git pull --rebase` on `../pc-loop` until you commit it.

## Codebase

- Background or live work must reuse the existing patterns: the engine's `offer_live_*` to commit
  exactly what a preview showed, and `jobs.rs`'s per-document job registration with the
  `job_conflict` refusal while it runs. A new dispatch queue that defers commands was rejected in
  review (wrong document, authorization bypass, fake `{"pending"}` replies).
- `gpu_canvas::now_ms()` returns a constant 0.0 on wasm32. Use the frame clock
  (`ctx.input(|i| i.time)` / `app.last_frame_time`) for anything that must advance on the web.
- `cargo xtask perf --quick` exits 1 on upstream/main itself: `layout_bench.rs:175` panics with
  "the layer is already a smart object" (seen at 14a4ab9b and b4a297fe). Judge perf runs by the
  other benches until task 06 lands.
- Quick Selection takes a flat region of up to about 240 px whole with a 20–30 px brush, so tests
  that check growth need large regions (the UI tests use 1600×600). `algo/src/segment/quick.rs` is
  fitted to measured Photoshop behaviour; never change results its tests pin.
- Run heavy parallel UI-side work in a dedicated rayon pool so it doesn't fill the global pool the
  UI thread relies on (#1554).
- Iterate with `cargo test -p <crate> --lib --tests` (skips examples and doctests); keep the full
  gates from AGENTS.md §5 for the end, and run `cargo fmt --all --check` on every commit of a
  branch, not just HEAD.
- Upstream commit subjects read `Area: what now happens (#issue)`. To reformat every commit of your
  own branch without conflicts, rebuild each with `git commit-tree` on a rustfmt'd tree.

## Reviews upstream

- Reviewers check behaviour against Photoshop first, then tests, then numbers. A PR that
  explains Photoshop's behaviour with a source and shows before/after numbers gets merged faster.
- Expect a maintainer to want a squashed history (2–3 commits, each green) rather than a log of
  every design that was tried.
