# PhotoCraft fix loop

Prompts that let Claude Code cloud sessions fix PhotoCraft bugs on their own, on this fork
(`erenova/photocraft`): one branch per fix, tested and reviewed by subagents, pushed as they go.
You test each finished branch and open the pull request to upstream (`storytold/photocraft`).

| File | What it is |
|---|---|
| `LOOP.md` | the loop's instructions (the session reads it every iteration) |
| `STATUS.md` | the task board: what's todo, in progress, ready for a PR |
| `tasks/NN-*.md` | one task per file, with the Photoshop behaviour to match |
| `TASK_TEMPLATE.md` | the shape of a task, for you or the loop to add more |
| `LEARNINGS.md` | lessons the loop writes for its future runs |
| `prs/NN-*.md` | the PR title and description for each finished fix (made by the loop) |

Everything here lives on the branch **`claude/loop`**. Your `main` stays an exact copy of
upstream, so fix branches start clean and never carry this folder into an upstream PR.

## 1. Run PhotoCraft on your computer

```sh
cd ~/repos/photocraft
cargo run --release -p photocraft             # or the built binary: ./target/release/photocraft
```

Try a finished fix before you open its PR:

```sh
git fetch origin
git switch claude/fix-00-quick-selection-live
cargo run --release -p photocraft
git switch main                               # back to normal
```

## 2. One-time setup on claude.ai/code

1. Open https://claude.ai/code, connect GitHub, and give the Claude GitHub app access to
   `erenova/photocraft`.
2. Create an **environment**. This is a settings form (the environment picker when you start a
   session → add or edit an environment), **not** a chat message. Fill it in once, save it, and
   pick it every time you start a session; the chat only ever gets the start prompt (step 3).
   - **Network access:** Trusted (the default; it allows GitHub, crates.io and the Rust and
     Ubuntu package servers).
   - **Environment variables:**
     ```
     BASH_DEFAULT_TIMEOUT_MS=600000
     BASH_MAX_TIMEOUT_MS=1800000
     CARGO_INCREMENTAL=0
     ```
     The last one keeps the build folder small enough for the VM's 30 GB disk.
   - **Setup script:**
     ```bash
     #!/bin/bash
     set -e
     apt-get update -qq
     DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
       libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev \
       libgtk-3-dev mesa-vulkan-drivers xvfb ffmpeg
     if command -v rustup >/dev/null; then
       rustup update stable --no-self-update
       rustup default stable
       rustup target add wasm32-unknown-unknown
     fi
     ```
     PhotoCraft needs Rust 1.95 or newer; the script updates the toolchain. Mesa's
     `mesa-vulkan-drivers` gives the VM a software GPU for the snapshot tests.

## 3. Start the loop

Pick **one** way. Both use your Claude plan's usage (credits); a loop with subagents uses a lot,
so watch your usage page the first day.

### A. One long session (start with this)

New session → repository `erenova/photocraft` → the environment from step 2 → model
**Opus 5.5** → paste this and send:

```text
/loop Run the PhotoCraft fix loop.
Bootstrap: git fetch origin claude/loop, then git worktree add -B claude/loop ../pc-loop origin/claude/loop (if ../pc-loop already exists, run git -C ../pc-loop pull --rebase origin claude/loop instead).
Then read ../pc-loop/prompts/LOOP.md and follow it exactly. Do one task per iteration: launch the implementer and reviewer subagents with model "opus", put every fix on its own claude/fix-* branch, commit in small steps, push to origin as you go, and push the board (../pc-loop/prompts/STATUS.md) after every change.
Never ask me anything and never wait for me: decide by LOOP.md's rules and write each decision into the board's notes.
Stop the loop only when LOOP.md says the board is empty.
```

If the session says `/loop` isn't available, send the same text without `/loop` at the start.
After any pause, open the session and send `continue`: everything it needs is in the files.
To end a session cleanly, send `hand off`: it stops its subagents, saves unfinished work under
`prompts/notes/`, writes where it stopped into the task file and pushes the board (LOOP.md §8).
A new session started with the same prompt then resumes that task first.

### B. A routine (a fresh session every few hours, fully hands-off)

https://claude.ai/code/routines → New routine → repository `erenova/photocraft`, the same
environment, model Opus 5.5, schedule **every 3 hours** → prompt:

```text
Run one cycle of the PhotoCraft fix loop.
Bootstrap: git fetch origin claude/loop, then git worktree add -B claude/loop ../pc-loop origin/claude/loop.
Then read ../pc-loop/prompts/LOOP.md and follow it for exactly one task (§0 to §7), with the implementer and reviewer subagents launched with model "opus". Commit in small steps and push to origin as you go. Never ask questions; decide by LOOP.md's rules and note decisions on the board. When the task is shipped or blocked and the board is pushed, stop.
```

Routines push only to `claude/…` branches by default, which is exactly what the loop uses.
Turn a routine off with its switch on the routines page. Don't run A and B at the same time at
first: the board's lock keeps them off each other's task, but you'd pay twice.

### Messages you can ignore

- **"Uncommitted changes" / the stop hook asking to commit.** Cloud sessions remind Claude to
  commit whenever it pauses. While a subagent is mid-edit, the loop correctly leaves those
  changes for the subagent to commit. Nothing to fix.
- **"GitHub API is blocked (403)".** Expected on the cloud VM; see section 5, step 4.

## 4. Watch it

- The board: https://github.com/erenova/photocraft/blob/claude/loop/prompts/STATUS.md
- The fix branches: https://github.com/erenova/photocraft/branches
- The session itself, on claude.ai/code.

## 5. Send a finished fix upstream (your part)

When a row on the board says `ready-for-pr`:

1. **Test it** on your computer (section 1). Does it behave like Photoshop?
2. **Open the PR.** Go to (put the row's branch name at the end):
   `https://github.com/storytold/photocraft/compare/main...erenova:claude/fix-00-quick-selection-live?expand=1`
   The page shows "base repository: storytold/photocraft, base: main ← head repository:
   erenova/photocraft, compare: your branch". That's right.
3. **Fill it in.** Copy the title and description from `prs/NN-*.md` on the `claude/loop`
   branch. Drag the PNG screenshots from `prs/NN-*/` into the description box. Press
   **Create pull request**.
4. **Tell the board.** The cloud VM can't read GitHub's API, so the loop can't see your PR.
   Edit the row's state to `pr-open #N` (N = the PR number), see section 6.
5. **Wait for the checks.** CI builds and tests it (a green tick when it passes). Maintainers
   and review bots comment.
6. **Changes requested?** Set the row to `changes-requested` and paste what the reviewer asked
   into its *Notes*. The loop picks it up first, fixes the branch and pushes; the PR updates by
   itself. Reply to the reviewer when it's done, e.g. "Addressed in the latest push, thanks!"
7. **Merged?** Set the row to `merged`. You can delete the branch on GitHub (the loop never
   deletes branches).

Same thing from the terminal, if you prefer:

```sh
gh pr create -R storytold/photocraft --head erenova:claude/fix-00-quick-selection-live \
  --title "…" --body-file prs/00-quick-selection-live.md
```

Words you'll see: a **fork** is your copy of someone's repository; a **branch** is a line of
work inside it; a **pull request** (PR) asks the original repository to take a branch's
changes; **CI** is the automatic build and test run on every PR; **rebase** replays a branch on
top of the newest `main`.

Keep your fork's `main` fresh: on https://github.com/erenova/photocraft press **Sync fork →
Update branch** now and then. Your `main` has no commits of its own, so this is always clean.

Optional: to be credited by name in PhotoCraft's About window (not only `@erenova`), add your
own entry to `contributors/people.toml` in one of your PRs (see `docs/contributors.md`).

## 6. Change what it works on

Edit files on the `claude/loop` branch (on GitHub: open the file, switch the branch picker to
`claude/loop`, press the pencil, commit). Add a task: copy `TASK_TEMPLATE.md` to
`tasks/NN-name.md`, fill it in, and add a `todo` row to `STATUS.md`. Skip one: set its state to
`dropped`. Move one up: reorder the rows. When the list runs out, the loop finds new work in
upstream's issues on its own.
