# Learnings

Short, durable lessons for the next agent: things that cost time once and shouldn't again. Read
this before every task; add at most 3 lines after each one; keep it under 150 lines by merging and
deleting stale lines. Newest first within each section.

## Environment

- The cloud VM has about 4 cores, 16 GB of RAM and 30 GB of disk. One `target/` is about 10 GB:
  build in the session's own checkout only (worktrees get their own `target/`).
- Bash calls time out (2 minutes by default, 10 at most): run `cargo build`/`cargo test` in the
  background and wait for the notification.
- `gh pr …` and `gh issue …` use GraphQL, which the cloud proxy blocks (403). Use `gh api` (REST)
  for the fork, and unauthenticated `curl https://api.github.com/...` for upstream.
- No GPU: wgpu falls back to Mesa's lavapipe (installed by the environment's setup script). Tests
  that need a real adapter skip or fail on the VM; say so in the PR text rather than hiding it.

## Codebase

- Iterate with `cargo test -p <crate> --lib --tests` (skips examples and doctests); keep the full
  gates from AGENTS.md §5 for the end.
- Upstream commit subjects read `Area: what now happens (#issue)`.

## Reviews upstream

- Reviewers check behaviour against Photoshop first, then tests, then numbers. A PR that
  explains Photoshop's behaviour with a source and shows before/after numbers gets merged faster.
