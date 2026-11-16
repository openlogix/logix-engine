#!/bin/bash
# Runs all checks and tests: format check, lint, then unit, schema,
# command-line and end-to-end MCP tests. Stops at the first failure.
# Run it before committing. CI runs the same script.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

echo "== Format check"
cargo fmt --check

echo "== Lint"
cargo clippy --all-targets --locked -- -D warnings

echo "== Tests"
cargo test --locked --no-fail-fast

# Delete old build files: Cargo never deletes them, so the build folder
# grows by gigabytes. First those unused for 7 days, then the oldest until
# the folder is under 8 GB, since a busy day's rebuilds filled the disk
# within 7 days (2026-11-16, over 20 GB per repo). The newest build, just
# made, stays. Needs cargo-sweep (cargo install cargo-sweep); skipped
# without it, as on CI.
echo "== Clean old build files"
if cargo sweep --version >/dev/null 2>&1; then
  cargo sweep --time 7
  cargo sweep --maxsize 8GB
else
  echo "Tip: cargo install cargo-sweep, to delete old build files."
fi

echo "== All checks and tests passed"
