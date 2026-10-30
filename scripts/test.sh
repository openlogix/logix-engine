#!/bin/bash
# Runs all checks and tests: format check, lint, then unit, schema,
# command-line and end-to-end MCP tests. Stops at the first failure.
# Run it before committing. CI runs the same script.

set -euo pipefail
cd "$(dirname "$0")/.."

echo "== Format check"
cargo fmt --check

echo "== Lint"
cargo clippy --all-targets --locked -- -D warnings

echo "== Tests"
cargo test --locked

echo "== All checks and tests passed"
