#!/bin/bash
# Builds the engine if needed, then starts an interactive Claude Code
# session with the engine connected as an MCP server, so you can prompt it:
#
#   scripts/run.sh                      # then type prompts
#   scripts/run.sh "What version is the Logix engine?"
#
# Arguments are passed to claude. Claude runs in a playground folder,
# ~/.logix-dev/playground (or LOGIX_PLAYGROUND), where it can create
# designs, not in this repo. The engine uses the engine folder
# ~/.logix-dev/engine (or LOGIX_ENGINE_FOLDER). Only the engine's MCP server
# is loaded, and its tools are allowed without asking. Each run starts the
# freshly built engine.
#
# Uses the claude command on the PATH, or else the one bundled with the
# Claude Code VS Code extension.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

repo="$(pwd)"
engine_folder="${LOGIX_ENGINE_FOLDER:-$HOME/.logix-dev/engine}"
playground="${LOGIX_PLAYGROUND:-$HOME/.logix-dev/playground}"

# Find claude.
claude_bin="$(command -v claude || true)"
if [[ -z "$claude_bin" ]]; then
  # The newest extension version.
  claude_bin="$(ls -d "$HOME"/.vscode/extensions/anthropic.claude-code-*/resources/native-binary/claude \
    2> /dev/null | sort -V | tail -n 1 || true)"
fi
if [[ -z "$claude_bin" || ! -x "$claude_bin" ]]; then
  echo "Error: claude not found. Install Claude Code: https://claude.com/claude-code" >&2
  exit 1
fi

echo "Building the engine..." >&2
cargo build --release --locked --quiet
engine="$repo/target/release/logix-engine"
echo "Engine $("$engine" --version | cut -d' ' -f2), engine folder $engine_folder" >&2
echo "Claude Code: $claude_bin, in $playground" >&2

mkdir -p "$playground"
cd "$playground"

mcp_config=$(cat <<JSON
{"mcpServers": {"logix-engine": {"command": "$engine", "args": ["--engine-folder", "$engine_folder"]}}}
JSON
)

exec "$claude_bin" \
  --strict-mcp-config \
  --mcp-config "$mcp_config" \
  --allowedTools "mcp__logix-engine" \
  "$@"
