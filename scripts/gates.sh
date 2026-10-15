#!/bin/bash
# The Rust gate matrix, one step after another, each logged: clippy (warnings are errors) and the
# whole test suite under the four feature sets the project is held to
#
#   all           every default feature (gpu, inference, plugins)
#   nogpu         without gpu
#   noinference   without inference
#   noplugins     without plugins
#
# Run it on a quiet machine (about 25 minutes warm, much longer cold). Progress is appended to
# <log-dir>/progress.txt as it goes, and a summary is printed at the end.
# Every configuration runs even after a failure. Any failed step or missing
# test summary makes the final exit status nonzero.
# Storage checks (Node.js 20+) precede/follow every command: default 20 GiB free,
# at most 64 GiB retained logical output. Refusals fail the gate; nothing is deleted.
#
# Usage:  scripts/gates.sh [log-dir]        (default: ${PF_WORK:-<repo>/.work}/gates)
# Optional real-file inputs are read from the environment (see docs/development.md); with none set
# those tests report "skipped" and still pass.
set -u
set -o pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" || exit 1
repo="$(cd "$here/.." && pwd)"
logs="${1:-${PF_WORK:-$repo/.work}/gates}"
mkdir -p "$logs" || exit 1
progress="$logs/progress.txt"
echo "START $(date +%T)" > "$progress" || exit 1
failed=0
export PF_TARGET_DIR="${PF_TARGET_DIR:-${CARGO_TARGET_DIR:-$repo/src-tauri/target}}"
export CARGO_TARGET_DIR="$PF_TARGET_DIR"
# Pin intermediate and final output to the same budgeted root, even if a user's
# global Cargo configuration otherwise separates them (Cargo 1.91+).
export CARGO_BUILD_BUILD_DIR="$PF_TARGET_DIR"

storage() {
  node "$here/build-cache.mjs" check --target-dir "$PF_TARGET_DIR" --base-dir "$repo/src-tauri" > "$logs/storage-$1.json" 2>&1
}

run() {
  local name="$1" rc=0
  shift
  echo "BEGIN $name $(date +%T)" >> "$progress" || exit 1
  if storage "$name-before"; then
    bash "$here/cargo-ext.sh" "$@" > "$logs/gate-$name.log" 2>&1 || rc=$?
    if ! storage "$name-after"; then
      echo "Storage check failed after $name; inspect storage-$name-after.json" >> "$logs/gate-$name.log"
      if [ "$rc" -eq 0 ]; then rc=86; fi
    fi
  else
    echo "Storage check refused $name; inspect storage-$name-before.json" > "$logs/gate-$name.log"
    rc=86
  fi
  echo "END $name rc=$rc $(date +%T)" >> "$progress" || exit 1
  if [ "$rc" -ne 0 ]; then failed=1; fi
}

run all-clippy         clippy --locked --all-targets --all-features -- -D warnings
run all-test           test   --locked --all-targets --all-features --no-fail-fast
run nogpu-clippy       clippy --locked --all-targets --no-default-features --features custom-protocol,inference,plugins -- -D warnings
run nogpu-test         test   --locked --all-targets --no-default-features --features custom-protocol,inference,plugins --no-fail-fast
run noinference-clippy clippy --locked --all-targets --no-default-features --features custom-protocol,gpu,plugins -- -D warnings
run noinference-test   test   --locked --all-targets --no-default-features --features custom-protocol,gpu,plugins --no-fail-fast
run noplugins-clippy   clippy --locked --all-targets --no-default-features --features custom-protocol,gpu,inference -- -D warnings
run noplugins-test     test   --locked --all-targets --no-default-features --features custom-protocol,gpu,inference --no-fail-fast
echo "GATES-COMPLETE $(date +%T)" >> "$progress" || exit 1

echo "== steps"
grep -E "^END " "$progress" || failed=1
echo "== tests (passed / failed / test binaries)"
for set in all nogpu noinference noplugins; do
  printf '%-12s ' "$set"
  awk '/^test result:/ {p += $4; f += $6; runs++}
       END {printf "%d passed, %d failed, %d binaries\n", p, f, runs;
            if (runs == 0 || f != 0) exit 1}' "$logs/gate-$set-test.log" || failed=1
done
echo "== clippy diagnostics"
for set in all nogpu noinference noplugins; do
  printf '%-12s %s\n' "$set" "$(grep -c -E '^(warning|error)' "$logs/gate-$set-clippy.log")"
done
exit "$failed"
