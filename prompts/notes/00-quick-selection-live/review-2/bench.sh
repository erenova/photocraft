#!/usr/bin/env bash
# Runs quick_selection_bench at 24 MP with several pointer speeds and brushes (review round 2).
set -u
OUT=${OUT:-/tmp/qs-bench}; mkdir -p "$OUT"
BIN=/home/user/photocraft/target/release/examples/quick_selection_bench
cd /home/user/photocraft || exit 1
run() {
  local name=$1; shift
  echo "=== $name: $* (load: $(cut -d' ' -f1-3 /proc/loadavg))"
  "$BIN" "$@" --json "$OUT/bench-$name.json" 2>&1
  echo "=== exit $?"
}
run default --size 6000x4000
run default2 --size 6000x4000
run fast16 --size 6000x4000 --step 16
run fast32 --size 6000x4000 --step 32
run slow4 --size 6000x4000 --step 4
run brush100 --size 6000x4000 --brush 100
