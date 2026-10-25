# 06 · Convert to Smart Object acts on the layer it's given, and `xtask perf --quick` passes again

- **Upstream issues:** none filed (found by the loop on 2026-10-24 while running task 00's gates)
- **Branch:** `claude/fix-06-convert-smart-object-target`
- **Suggested PR title:** `Smart objects: Convert to Smart Object checks the layer it converts, not the active one`
- **Time box:** 2 hours. Past it, push what you have, write down what you learned, and mark the
  row `blocked`.

## The report

`cargo xtask perf --quick` exits 1 on upstream/main (seen at 14a4ab9b and b4a297fe, 2026-10-24/25):
`crates/ui-egui/examples/layout_bench.rs:175` panics with
`convert: Disabled("layer.smartObjects.convertToSmartObject", "the layer is already a smart object")`.
Every other quick bench runs; the regression check can't be trusted while one bench panics. Two
review rounds of task 00 reproduced it on a clean upstream worktree.

## Photoshop behaviour (the spec)

Layer › Smart Objects › Convert to Smart Object converts the selected layer(s). It is unavailable
only when the selection can't be converted; a layer that is already a smart object can be wrapped
again (Photoshop nests it). Source: Adobe Help, "Create smart objects"
(https://helpx.adobe.com/photoshop/using/create-smart-objects.html). Whether PhotoCraft should
nest instead of refusing is out of scope here (inferred; leave the refusal as it is).

## What PhotoCraft does today

- `crates/engine/src/smart_cmds.rs:1385`: the command's check is
  `if active_smart(s).is_ok() { Err("the layer is already a smart object".into()) }`. It looks at
  the **active** layer, even when the caller names another one with `{"layer": id}`.
- `layout_bench.rs:168-176` converts several layers in a row, each by id. After the first
  conversion the active layer is the new smart object, so the second call is refused although the
  layer it names is a plain layer. **Hypothesis to confirm first**: reproduce with an engine test
  (two raster layers; convert the first by id, then the second by id → today `Err`).
- Second hypothesis, if the first is wrong: the bench's comment says "PSD export writes engine-made
  smart objects as pixels (no SoLd yet)". If PSD export now writes real smart objects, the layers
  come back as smart objects and the bench's conversion is redundant. Then fix the bench (skip
  layers that already are smart objects) and say so.
- Check other `enabled` closures in `smart_cmds.rs` that read the active layer while accepting a
  `layer` param: the same bug may repeat.

## Upstream work to check first

`git log upstream/main --oneline -i --grep 'convertToSmartObject' --grep 'smart object' | head`,
and `git log -L1380,1390:crates/engine/src/smart_cmds.rs upstream/main | head -40` for when the
check was written. The API is blocked, so open PRs can't be searched; note that in the PR text.

## Acceptance criteria

- [ ] An engine test that fails before the fix: converting a named non-active plain layer works
      while the active layer is a smart object; converting a named smart object is still refused.
- [ ] `cargo run --release -p photocraft-ui-egui --example layout_bench` runs to the end.
- [ ] `cargo xtask perf --quick` exits 0 (or every remaining failure is named and unrelated).
- [ ] Bad params (`layer` missing, unknown, wrong type) → `Err`, never a panic; panic_hunt green.

## Gates

```sh
cargo test -p photocraft-engine --lib --tests
cargo clippy -p photocraft-engine -p photocraft-ui-egui --all-targets -- -D warnings
cargo test -p photocraft-engine --test panic_hunt -- --ignored
cargo xtask perf --quick
cargo fmt --all --check
```

## Out of scope

Nesting a smart object inside another on conversion; PSD SoLd export.
