# NN · <What will be true when this is done>

- **Upstream issues:** storytold/photocraft#<n> (and related)
- **Branch:** `claude/fix-NN-slug`
- **Suggested PR title:** `Area: what now happens (#n)`
- **Time box:** <hours>. Past it, push what you have, write down what you learned, and mark the
  row `blocked`.

## The report

<What users see, in their words, with links to screenshots or videos. Facts only.>

## Photoshop behaviour (the spec)

<Exactly what Photoshop 2026 does, step by step: tool, modifiers, options bar, menus, history
step name, edge cases. Name the public source for each point (Adobe help page, tutorial,
`docs/*-parity.md`). Mark anything not documented as "inferred".>

## What PhotoCraft does today

<Code pointers (file, function, command id) and the cause, if known. Measured numbers, with the
command that measured them.>

## Upstream work to check first

<Open or closed PRs touching this, and what to do about each.>

## Acceptance criteria

- [ ] <observable behaviour 1, matching the spec>
- [ ] <tests that would have failed before the fix>
- [ ] <graceful failure on bad input: Err, never a panic>
- [ ] <numbers, if performance is involved: before/after on a 24 MP image>
- [ ] <visual check, if UI is involved: snapshot PNGs saved for the PR>

## Gates

<The AGENTS.md §5 commands that apply to the crates this touches.>

## Out of scope

<Neighbouring work that belongs in another task.>
