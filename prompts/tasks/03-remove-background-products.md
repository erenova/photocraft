# 03 · Remove Background cuts out product shots cleanly, in one click

- **Upstream issues:** #9145 ("Select Subject only selects the face and ignores the body"),
  #2788 (Select Subject / Remove Background with a local model), #7534
- **Branch:** `claude/fix-03-remove-background-products`
- **Suggested PR title:** `Remove Background: clean product cutouts on plain backgrounds, as in Photoshop`
- **Time box:** 8 hours.

## Why this task exists

The fork owner removes backgrounds from AI-generated product PNGs: one object on a plain, light
or softly graded background. Photoshop's one-click **Remove Background** handles that case
almost perfectly, and so must PhotoCraft's, without any AI model download, since the default
build ships none.

## Photoshop behaviour (the spec)

1. **Where:** Properties panel › Quick Actions › *Remove Background* on a pixel layer (also the
   Contextual Task Bar's *Remove background*). PhotoCraft has the Properties button already
   (`crates/ui-egui/src/props_layout.rs`).
2. **What it does:** if the layer is *Background*, it first becomes a normal layer; then it
   finds the subject (Select Subject) and adds a **layer mask** that hides everything else. The
   pixels are untouched: painting the mask fixes mistakes, and Layer › Layer Mask › Apply bakes
   it in.
3. Running it again replaces the mask; undo removes it in one step (inferred: check the History
   panel names PhotoCraft already uses).
4. **Expected quality on product shots:** the whole object, including thin parts (handles,
   straps, caps), holes through which the background shows are cut out, edges anti-aliased
   without a light halo, the drop shadow on the background removed (inferred: Photoshop's
   result on such images).

## What PhotoCraft does today

- `layer.removeBackground`: `crates/engine/src/cutout_cmds.rs`. It finds the subject with
  `select.subject`'s heuristic (`crates/algo/src/segment/subject.rs` and around), refines it
  (`REFINE` params) and writes a layer mask. Tests in the same file already cover depths,
  Background conversion, cancellation and bad params.
- Bench: `cargo run --release -p photocraft-algo --example bench_smartselect` reports Select
  Subject IoU 0.9998 on a synthetic disc on noise (727 ms on a 6016×6016 image, 12-core desktop,
  2026-10-24). A disc is far easier than a product shot: build harder cases.

## Upstream work to check first

- **PR #7918** (BiRefNet background removal as a cancellable job) and **PR #8231** (optional
  local models: neural Select Subject, Remove Subject, Preferences › AI Models): open as of
  2026-10-24. They add optional AI models. This task improves the **default, model-free** path
  for plain-background product shots, which stays the fallback either way. Read both and keep
  your change compatible with them (same entry points; don't rename what they hook into).
- PR #2995 (BiRefNet-lite via tract): old; read only for context.

## How to do it

1. **A product test set, made in code** (clean-room): bottles, boxes, a mug with a handle, a
   bag with straps, a shoe; matte and glossy shading; on near-white, light grey, a flat colour
   and a two-tone sweep; with soft contact shadows; with and without light JPEG-like noise; one
   white product on a white background (hard). You know the true masks.
2. **Score the current path** (IoU, edge-band alpha error, halo, holes not cut out, time at
   2048² and 24 MP on the VM) and keep the numbers.
3. **Improve it** for this class of images. One idea to evaluate, not a prescription: when the
   image border is dominated by one smooth background (low variance, slow gradient), model that
   background from the border (colour plus a low-order gradient fit), grow it inward with an
   adaptive tolerance, reject shadows as background-coloured-but-darker, cut holes where enclosed
   regions match the background model, then refine the edge with the existing matting code and
   decontaminate the edge colour. Fall back to the current heuristic when the border isn't a
   clean background. Whatever you build must not make people or busy scenes worse: score those
   too, with the existing tests and bench.
4. Every pixel loop works per tile in parallel and is depth-agnostic (8/16/32-bit, AGENTS.md
   rules 2 and 8).

## Acceptance criteria

- [ ] On the product test set: mean IoU ≥ 0.99, holes cut out, no light halo (thresholds
      justified in the PR text), with before/after numbers per image.
- [ ] No regression on the existing Select Subject / Remove Background tests and bench cases.
- [ ] Time at 24 MP on the VM, before and after; within 2× of before, or justified.
- [ ] Works at 8, 16 and 32-bit and on a CMYK document.
- [ ] Bad input (empty layer, 1×1 layer, a layer off the canvas, cancellation) → no panic.
- [ ] Before/after snapshot PNGs of three product cutouts on a checkerboard.

## Gates

```sh
cargo test -p photocraft-algo -p photocraft-engine
cargo clippy -p photocraft-algo -p photocraft-engine --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo test -p photocraft-engine --test panic_hunt -- --ignored
cargo xtask perf --quick
```

## Out of scope

Shipping or downloading AI models. People and portraits (beyond not regressing them).
