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

# Delete build files unused for 7 days: Cargo never deletes old ones, so
# the build folder otherwise grows by gigabytes. Needs cargo-sweep
# (cargo install cargo-sweep); skipped without it, as on CI.
echo "== Clean old build files"
if cargo sweep --version >/dev/null 2>&1; then
  cargo sweep --time 7
else
  echo "Tip: cargo install cargo-sweep, to delete old build files."
fi

echo "== All checks and tests passed"
