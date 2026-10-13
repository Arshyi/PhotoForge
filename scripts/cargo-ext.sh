#!/bin/bash
# Runs cargo in the crate directory with this project's habits, from any directory:
#
#   * PATH gets ~/.cargo/bin, because cargo is often not on PATH in a non-interactive shell;
#   * PF_TARGET_DIR (optional) moves the build output out of the repository; the default is the
#     usual src-tauri/target;
#   * dependency debug info is reduced for the dev profile, per invocation with --config (no file is
#     changed), which keeps the debug target several times smaller.
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
sub="$1"; shift
cargo "$sub" --config 'profile.dev.package."*".debug="line-tables-only"' "$@"
