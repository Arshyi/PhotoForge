#!/bin/bash
# Runs cargo in the crate directory with this project's habits, from any directory:
#
#   * PATH gets ~/.cargo/bin, because cargo is often not on PATH in a non-interactive shell;
#   * PF_TARGET_DIR (optional) moves the build output out of the repository; the default is the
#     usual src-tauri/target;
#   * PF_BUILD_MODE=compact (default) uses line tables for application/dependency debug info
#     and disables incremental compilation. PF_BUILD_MODE=full uses ordinary Cargo profiles.
#     Features, optimisations, assertions, overflow checks and release settings stay intact.
#   * PF_CARGO optionally selects a cargo executable (also the hermetic regression boundary).
#
# Usage:  scripts/cargo-ext.sh <subcommand> <cargo arguments...> [-- test arguments]
#         scripts/cargo-ext.sh test --lib raw::
#         scripts/cargo-ext.sh clippy --all-targets -- -D warnings
# Do not use it for `cargo fmt` (that subcommand has no --config); run `cargo fmt` in src-tauri.
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
if [ -n "$PF_TARGET_DIR" ]; then export CARGO_TARGET_DIR="$PF_TARGET_DIR"; fi
export CARGO_TERM_COLOR=never
cd "$repo/src-tauri" || exit 1
if [ "$#" -eq 0 ]; then
  echo 'Usage: cargo-ext.sh <cargo subcommand> [arguments]' >&2
  exit 64
fi
profile_args=()
case "${PF_BUILD_MODE:-compact}" in
  compact)
    export CARGO_INCREMENTAL=0
    profile_args=(--config "$here/cargo-compact.toml")
    ;;
  full) ;;
  *) echo 'PF_BUILD_MODE must be compact or full.' >&2; exit 64 ;;
esac
sub="$1"; shift
"${PF_CARGO:-cargo}" "$sub" "${profile_args[@]}" "$@"
