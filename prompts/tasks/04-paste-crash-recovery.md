# 04 · Pasting many images doesn't crash, and recovery keeps the latest autosave

- **Upstream issues:** storytold/photocraft#7582 (`priority: critical`: the app crashed after
  six pastes of 1920×1080 images; after restarting, recovery had only the first paste)
- **Branch:** `claude/fix-04-paste-crash-recovery`
- **Suggested PR title:** `Paste: repeated large pastes stay stable and recovery holds the latest autosave (#7582)`
- **Time box:** 5 hours. A crash you can't reproduce on Linux is a valid outcome: report what you
  ruled out, keep any hardening you found, and mark the row `blocked` with the evidence.

## The report

Windows 11: open the app; copy a 1920×1080 image; File › New with the clipboard's size; paste six
times, copying a new 1920×1080 image into the clipboard before each paste. The app crashed.
After reopening, the recovered document held only the first pasted image, not the other five.
(The reporter suggests autosaving on every paste. That isn't Photoshop's behaviour: see the spec.)

## Photoshop behaviour (the spec)

Sources: Photoshop's Edit menu and Preferences › File Handling ("Automatically Save Recovery
Information Every: N minutes").

1. **Edit › Paste** (Ctrl/⌘+V) puts the clipboard image on a new layer above the active one, as
   one history step. Where it lands is outside this task: keep `edit.paste`'s placement.
2. Many pastes of large images never crash: Photoshop spills to its scratch disk.
3. **Recovery** saves documents with unsaved changes every N minutes (default 10). After a
   crash, Photoshop reopens the newest recovery save. It doesn't save on every action, so
   recovery can be up to N minutes old, but it must never be *older* than the last completed
   autosave.

## What PhotoCraft does today

- `edit.paste`: `crates/engine/src/edit_cmds.rs`. Clipboard reads go through the platform
  services in `apps/photocraft/src/services.rs` (decoding of image formats from the OS
  clipboard happens there or in `crates/codecs`).
- Autosave: `crates/format/src/autosave.rs` (`Autosaver::request`, `request_checked`,
  `take_completed`, `RecoveryStore`); triggered from `apps/photocraft/src/services.rs`
  (`fn autosave`); preferences `autosave`, `autosave_minutes` in `crates/engine/src/prefs.rs`
  (default 10 minutes, as in Photoshop).
- Neither cause is known yet.

## How to investigate

1. **The crash.** Reproduce headless: an engine-level test that pastes 6, then 30 different
   1920×1080 images (8 and 16-bit, with and without alpha) into a 1920×1080 document, then
   undoes and redoes all of them. Watch peak memory (`/usr/bin/time -v`). Then the clipboard
   decode path: feed the decoder the formats Windows puts on the clipboard (CF_DIB/CF_DIBV5
   bitmaps with and without alpha masks, top-down and bottom-up, PNG) as synthetic bytes,
   including truncated and lying headers. Any panic, unbounded allocation or quadratic growth
   is a finding: fix it with a regression test (AGENTS.md *Never crash*).
2. **The recovery.** Write a test with a fake clock or a short interval: paste, autosave
   completes, paste five more, let the interval pass, "crash" (drop the app without a clean
   shutdown), then list and recover. Expected: six pasted layers. Look for: a revision or dirty
   flag that stops later autosaves after the first one; a background save that never completes
   or whose completion is never collected (`take_completed`); a snapshot taken before the paste
   lands; a recovery entry chosen by the wrong timestamp.
3. Read `log/`-style crash output if the app writes any (`apps/photocraft/src/app_dirs.rs` shows
   where logs go) for hints about what a Windows crash would have logged.

## Acceptance criteria

- [ ] A test proving the recovery bug (or proving recovery is right, with the reason the
      reporter saw an old state, e.g. the interval hadn't passed).
- [ ] If recovery was wrong: fixed; the newest completed autosave is what recovery offers.
- [ ] Stress test for repeated pastes passes with bounded memory (numbers in the PR text).
- [ ] Every crash path found has a regression test that panicked before the fix.
- [ ] Bad clipboard data → a message the user can act on, never a panic.

## Gates

```sh
cargo test -p photocraft-engine -p photocraft-format -p photocraft-codecs -p photocraft
cargo clippy -p photocraft-engine -p photocraft-format -p photocraft-codecs -p photocraft --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo test -p photocraft-engine --test panic_hunt -- --ignored
cargo xtask test-corpus   # if codecs or format changed
```

## Out of scope

Autosaving on every edit (not Photoshop). Windows-only clipboard APIs you can't run here: if
the cause is there, describe the suspected code path in the PR text for a Windows tester.
