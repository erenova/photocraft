# 05 · Smart filters reopen, edit and delete again (regression since 0.13)

- **Upstream issues:** storytold/photocraft#9055 (since 0.13, smart filters can't be reopened,
  deleted or have their sliders changed; blend mode and opacity still work), related #8388
  (Layer › Smart Filter › Blending Options), #6291, #5689, #8764
- **Branch:** `claude/fix-05-smart-filters-reopen`
- **Suggested PR title:** `Smart Filters: double-clicking a filter reopens its dialog again (#9055)`
- **Time box:** 5 hours.

## The report

#9055, reporter on Linux, version 0.15.1: "Just convert a layer to smart object and add a smart
filter (either normal or filter gallery): they won't reopen. They cannot be opened, deleted or
have their sliders interacted with. But I can change the blend mode and opacity." It worked
before 0.13, around when smart filters gained blending options.

## Photoshop behaviour (the spec)

Source: Adobe Help, "Apply Smart Filters"
(https://helpx.adobe.com/photoshop/using/applying-smart-filters.html); `docs/ui-parity.md` and
`docs/context-menu-parity.md` in this repo.

1. Under a smart object in the Layers panel: a *Smart Filters* row (with the filter mask
   thumbnail), then one row per filter, newest on top, each with an eye.
2. **Double-click a filter's name** → its dialog opens with the stored settings; OK re-renders
   the smart object, Cancel changes nothing. A Filter Gallery filter reopens the Filter Gallery
   with its stack.
3. **Double-click the blending icon** (the sliders icon at the right end of the filter row) →
   *Blending Options (filter name)*: Mode and Opacity, with a preview.
4. **Right-click a filter row** → Edit Smart Filter Blending Options…, Edit Smart Filter…,
   Disable Smart Filter, Delete Smart Filter, and the filter-mask items. **Delete** also works by
   dragging the row onto the panel's trash can.
5. The same actions are in Layer › Smart Filter.
6. Each edit is one history step.

## What PhotoCraft does today

- Layers panel rows: `crates/ui-egui/src/layer_row_ui.rs` (the smart filter rows are drawn
  there). Engine commands: `layer.smartFilter.*` (`blendingOptions`, `setParams`, `delete`,
  `setVisible`, `move`, …), find them with
  `grep -rn '"layer.smartFilter' crates/engine/src`.
- Tests that cover double-click for special filters exist and pass:
  `liquify_ui.rs` `smart_filter_double_click_edits_the_recorded_liquify`,
  `camera_raw_ui.rs` `smart_filter_double_click_edits_the_stored_camera_raw_filter_in_its_own_dialog`,
  `gallery_ui.rs` `smart_filter_double_click_edits_the_stored_gallery_stack`. So the break is
  probably in how the real panel routes the click (hit areas, the new blending icon, an
  `egui` id clash, a drag sensor swallowing the second click), which those tests bypass.
- Releases to bisect between: tags `v0.12.0` (works, per the report) and `v0.13.0` (broken).
  `git fetch upstream --tags`, then `git log --oneline v0.12.0..v0.13.0 -- crates/ui-egui/src/layer_row_ui.rs crates/ui-egui/src/layers*.rs crates/engine/src/smart*`.

## Upstream work to check first

- Open PRs touching smart filters: #7153 (Liquify double-click), #6187 (re-editing keeps
  settings), #3297 (paint the filter mask). Check each for an overlap with this fix; none of them
  claims #9055 as of 2026-10-24.
- #8388 (Blending Options doesn't work) is still open: if your fix also fixes it, say so.

## Acceptance criteria

- [ ] Reproduce first with a ui-egui harness test that drives **the real Layers panel** (real
      pointer events on the row, not a direct call to the edit function): convert a layer to a
      smart object, apply Gaussian Blur as a smart filter, double-click its row. Fails before the
      fix, passes after.
- [ ] The same test for: a Filter Gallery filter; deleting through the context menu and by
      dragging to the trash can; changing a dialog slider then OK (the image re-renders) and
      Cancel (nothing changes); the blending icon still opening Blending Options.
- [ ] The commit that broke it is named in the PR text, with why.
- [ ] Undo/redo of each edit is one step.
- [ ] No panic on stale ids (a filter index past the end, a layer that's gone): `Err`.
- [ ] Snapshot PNGs of the reopened dialog over the panel.

## Gates

```sh
cargo test -p photocraft-engine -p photocraft-ui-egui
cargo clippy -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo test -p photocraft-engine --test panic_hunt -- --ignored   # if engine commands changed
```

## Out of scope

New smart filter types. PSD import of filters that don't load (storytold/photocraft#9162 covers Displace).
