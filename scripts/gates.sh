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
#
# Usage:  scripts/gates.sh [log-dir]        (default: ${PF_WORK:-<repo>/.work}/gates)
# Optional real-file inputs are read from the environment (see docs/development.md); with none set
# those tests report "skipped" and still pass.
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
logs="${1:-${PF_WORK:-$repo/.work}/gates}"
mkdir -p "$logs" || exit 1
progress="$logs/progress.txt"
echo "START $(date +%T)" > "$progress"

run() {
  name="$1"; shift
  echo "BEGIN $name $(date +%T)" >> "$progress"
  "$here/cargo-ext.sh" "$@" > "$logs/gate-$name.log" 2>&1
  echo "END $name rc=$? $(date +%T)" >> "$progress"
}

run all-clippy         clippy --locked --all-targets --all-features -- -D warnings
run all-test           test   --locked --all-targets --all-features --no-fail-fast
run nogpu-clippy       clippy --locked --all-targets --no-default-features --features custom-protocol,inference,plugins -- -D warnings
run nogpu-test         test   --locked --all-targets --no-default-features --features custom-protocol,inference,plugins --no-fail-fast
run noinference-clippy clippy --locked --all-targets --no-default-features --features custom-protocol,gpu,plugins -- -D warnings
run noinference-test   test   --locked --all-targets --no-default-features --features custom-protocol,gpu,plugins --no-fail-fast
run noplugins-clippy   clippy --locked --all-targets --no-default-features --features custom-protocol,gpu,inference -- -D warnings
run noplugins-test     test   --locked --all-targets --no-default-features --features custom-protocol,gpu,inference --no-fail-fast
echo "GATES-COMPLETE $(date +%T)" >> "$progress"

echo "== steps"
grep -E "^END " "$progress"
echo "== tests (passed / failed / test binaries)"
for set in all nogpu noinference noplugins; do
  printf '%-12s ' "$set"
  grep -E '^test result' "$logs/gate-$set-test.log" \
    | awk '{p += $4; f += $6} END {printf "%d passed, %d failed, %d binaries\n", p, f, NR}'
done
echo "== clippy diagnostics"
for set in all nogpu noinference noplugins; do
  printf '%-12s %s\n' "$set" "$(grep -c -E '^(warning|error)' "$logs/gate-$set-clippy.log")"
done
