#!/bin/bash
# Builds the engine if needed and runs it as an MCP server on stdin/stdout,
# as a host would. For example, to connect Claude Code to it:
#
#   claude mcp add logix-engine -- /path/to/logix-engine/scripts/run.sh
#
# Uses the engine folder ~/.logix-dev/engine unless LOGIX_ENGINE_FOLDER is
# set. Extra arguments are passed to the engine, e.g. --log-level debug.
# Cargo's output goes to stderr, so stdout stays clean for MCP.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

cargo build --release --locked --quiet 1>&2
exec target/release/logix-engine \
  --engine-folder "${LOGIX_ENGINE_FOLDER:-$HOME/.logix-dev/engine}" "$@"
