# 01 · Magic Wand selects like Photoshop's: clean edges, Sample Size, exact modifiers

- **Upstream issues:** storytold/photocraft#4902 (tolerance "doesn't fully grab an item … leaves
  a trace of the selected line behind", and the more you use it on the same item, the more you
  lose), #9145 ("Magic Wand is also hard to control"; Alt is taken by the Linux window manager),
  #5775 (clicking inside the selection doesn't deselect)
- **Branch:** `claude/fix-01-magic-wand-parity`
- **Suggested PR title:** `Magic Wand: in-tolerance pixels stay fully selected, and Sample Size, as in Photoshop (#4902)`
- **Time box:** 6 hours.

## The report

#4902 has a video: https://github.com/user-attachments/assets/0a7a998e-6e4b-43dc-978d-9f940e05e8ec.
Try `curl -L -A Photocraft-dev -o /tmp/4902.mp4 <url>` and look at frames
(`ffmpeg -i /tmp/4902.mp4 -vf fps=2 /tmp/4902-%03d.png`, then read the PNGs). If the download is
blocked, work from the text and the hypotheses below.

The fork's owner also expected dragging with the wand to grow the selection. **Photoshop doesn't
do that** (see the spec): the drag-to-grow tool is Quick Selection, task 00. Don't add
drag behaviour to the Magic Wand.

## Photoshop behaviour (the spec)

Sources: Adobe Help, "Select areas by color with the Magic Wand tool"
(https://helpx.adobe.com/photoshop/desktop/make-selections/automatic-color-based-selections/select-areas-by-color-with-the-magic-wand-tool.html);
Adobe Help, "Select an object with the Magic Wand tool"
(https://helpx.adobe.com/photoshop/using/tool-techniques/magic-wand-tool.html). The pages may be
unreachable from the VM; this is what they describe.

1. **Click-based.** A click selects; a drag adds nothing beyond the click.
2. **Options bar, left to right:** the four mode buttons (New, Add to, Subtract from, Intersect
   with selection); **Sample Size** (Point Sample, 3 by 3 Average, 5 by 5, 11 by 11, 31 by 31,
   51 by 51, 101 by 101 Average); **Tolerance** 0–255, default 32, with a scrubby label (drag the
   word left or right; PhotoCraft has this already); **Anti-alias** (on); **Contiguous** (on);
   **Sample All Layers** (off); *Select Subject*; *Select and Mask…*.
3. **Sample Size** sets the colour the wand matches: the clicked pixel, or the average of the
   square around it. Users report it is the same setting as the Eyedropper's Sample Size
   (inferred; PhotoCraft's is `tool_options.eyedropper_size`, see `state.rs`, and
   `photocraft_engine::sample_cmds::SAMPLE_SIZES`). If you can't confirm the link, give the wand
   its own setting with the same values.
4. **Tolerance:** a low value selects colours very close to the sampled one; a higher value, a
   broader range. Contiguous on: only connected pixels; off: every matching pixel.
5. **Modifiers** override the mode buttons: Shift-click adds, Alt-click (Option) subtracts,
   Shift+Alt-click intersects.
6. **Changing Tolerance doesn't change the current selection**; it applies to the next click
   (Adobe/Peachpit: you change Tolerance *between* clicks).
7. **Select › Grow and Select › Similar** use the Magic Wand's current Tolerance.
8. **A New-mode click inside an existing selection makes a new selection** from the clicked
   pixel; it doesn't deselect (only a marquee click without a drag deselects). This is the
   current PhotoCraft behaviour and the upstream triage of #5775 says the same; keep it.
9. **Anti-alias** smooths the edge of the selection. Inferred, from how users rely on it: every
   pixel within tolerance stays **fully** selected, and smoothing only adds partial coverage to
   pixels just outside. So Delete after a wand click on a flat-colour shape leaves nothing of
   the shape behind. Say "inferred" in the PR text.

## What PhotoCraft does today (upstream `main` at b07b7fab5)

- UI: `crates/ui-egui/src/canvas.rs`, `Tool::MagicWand` in the pointer-down handler: runs
  `select.magicWand` with the options-bar values and `selection_mode(app, mods)`, on press.
  Options bar: `crates/ui-egui/src/panels.rs`, `Tool::MagicWand =>` (no Sample Size).
- Engine: `crates/engine/src/selection_cmds.rs`, `magic_wand` (`select.magicWand`). It samples
  `shown_gray` (visible alpha, Quick Mask or layer mask when that's what's shown) or
  `sample_rgba8` (active layer or composite, 8-bit).
- Algorithm: `crates/algo/src/selection.rs`, `wand_region` (the 8-bit span fill used by the
  command), `magic_wand` (the f32 reference), and the anti-alias rule in `antialias`.

### Hypotheses for #4902, in order (prove or rule out each with a failing test first)

- **H1, anti-alias eats the inside edge.** The rule sets a selected edge pixel to
  `0.5 + 0.5 × (selected neighbours / 9)`: an in-tolerance pixel on the boundary ends up only
  50–94 % selected. Delete then leaves 6–50 % of it: a faint ring of the object, at *any*
  tolerance, which is the report. Doing it again on what's left eats further in. If that's the
  cause: keep in-tolerance pixels at 100 % and put the smoothing in the outside fringe only,
  in both `antialias` and `segment::antialias_u8` (one rule, one place: see PR #8582). The
  Paint Bucket and the Background Eraser share this code, so check what they need (PR #6242
  fixes the bucket's halo in a similar way).
- **H2, alpha is compared like a colour.** `similar` compares alpha with the same tolerance, so
  on a transparent layer the soft edge of an anti-aliased line drops out ("a trace of the
  line"). Find out what the reporter's layer looked like before changing this; if you can't
  establish Photoshop's behaviour, leave it and note it as open.
- **H3, the sampled picture changes after the first click** (the selection overlay, Quick Mask
  or a mask view leaking into `shown_gray`), which would explain "the more times you use it,
  the more you lose". Rule it out with a test that clicks the same pixel five times in New mode
  and asserts identical selections, and five times in Add mode.

## Upstream work to check first

- **PR #8582** (draft, depends on #8554): moves the wand's anti-alias onto
  `segment::antialias_u8`. Same output, faster. If it has merged, change the rule in that one
  place. If not, change both copies the same way and mention #8582 in the PR text.
- **PR #5851** makes a New-mode click inside the selection deselect. That contradicts spec
  point 8: don't take it. Say why in the PR text.
- **PR #3919** re-runs the last click when Tolerance changes. Not Photoshop (spec point 6): don't
  take it.
- **PR #2995** bundles a Magic Wand Sample Size with an AI model. Take nothing from it except, if
  useful, the shape of the Sample Size option.

## Acceptance criteria

- [ ] A failing-before test for the cause you confirmed (H1 at least): wand click with
      Anti-alias on a flat-colour shape, then `edit.clear`, leaves zero ink of that colour.
- [ ] Repeated clicks on the same pixel give identical selections (New and Add modes).
- [ ] Sample Size in the options bar and a `sampleSize` param on `select.magicWand` (default
      Point Sample, so existing scripts behave the same); tests for 1, 3 and 101 on a noisy
      image; values outside the list → `Err`.
- [ ] Shift / Alt / Shift+Alt override the mode buttons; the Subtract button works with no key
      (Linux window managers take Alt+drag, #9145).
- [ ] `select.grow` and `select.similar` use the wand's Tolerance (check; fix if not).
- [ ] Tests at 8, 16 and 32-bit and in a CMYK document (AGENTS.md rule 2). If CMYK matching
      differs from Photoshop's per-ink comparison and isn't a small fix, write it into
      `docs/gaps.md` instead.
- [ ] Bad params (NaN, huge coordinates, negative tolerance) → `Err`; `panic_hunt` green.
- [ ] Before/after snapshot PNGs: wand + Delete on an anti-aliased shape.
- [ ] `docs/ui-parity.md` updated, with the date.

## Gates

```sh
cargo test -p photocraft-algo -p photocraft-engine -p photocraft-ui-egui
cargo clippy -p photocraft-algo -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo test -p photocraft-engine --test panic_hunt -- --ignored
cargo xtask parity      # if a command or param was added
```

## Out of scope

Drag behaviour (Photoshop has none). Quick Selection (task 00). Select Subject quality.
