#!/bin/bash
# Builds the engine if needed, then starts an interactive Claude Code
# session with the engine connected as an MCP server, so you can prompt it:
#
#   scripts/run-in-claude.sh            # then type prompts
#   scripts/run-in-claude.sh "What version is Logix Engine?"
#
# Arguments are passed to claude. Claude runs in a playground folder,
# ~/.logix/engine-dev/playground (or LOGIX_ENGINE_DEV_PLAYGROUND), where it can create
# designs, not in this repo. The engine uses the engine folder
# ~/.logix/engine-dev/engine-data (or LOGIX_ENGINE_DEV_DATA_FOLDER), and logs at LOGIX_ENGINE_DEV_LOG_LEVEL
# (info by default; debug also logs every program's output). Only the
# engine's MCP server is loaded, and its tools are allowed without asking.
# Each run starts the freshly built engine, copied to ~/.logix/engine-dev/engine-program so
# the session doesn't see this repo's path.
#
# For a reproducible starting point, the session loads none of your user
# settings (~/.claude/settings.json), no CLAUDE.md files (your own
# ~/.claude/CLAUDE.md included) and no auto memory, doesn't connect to an
# IDE, and gets the rules in scripts/_test-session.sh: start only from
# what the engine provides, and don't read local repos or earlier notes.
# The rules ask the agent; they don't enforce.
#
# Uses the claude command on the PATH, or else the one bundled with the
# Claude Code VS Code extension.
#
# Prerequisites:
#
#   - macOS on Apple Silicon (the only platform in Milestone 1).
#   - This repo, cloned locally:
#       git clone https://github.com/openlogix/logix-engine.git
#   - Rust, installed with rustup. On macOS: brew install rustup
#     (see https://rustup.rs for other ways). The first build installs the
#     Rust version pinned in rust-toolchain.toml.
#   - Claude Code, signed in to a Claude account: either the claude command
#     (https://claude.com/claude-code) or the Claude Code extension for
#     VS Code. Run claude once by hand to sign in.
#
# Registering the engine instead (optional):
#
#   This script hands the engine to Claude Code for one session only, with
#   these claude options:
#
#     --strict-mcp-config --mcp-config '{"mcpServers": {"logix-engine":
#       {"command": "<repo>/target/release/logix-engine",
#        "args": ["--engine-folder", "<home>/.logix/engine-dev/engine-data"]}}}'
#     --allowedTools mcp__logix-engine
#
#   To have the engine in every claude session instead, register it once.
#   scripts/serve.sh builds the engine if needed and runs it as an MCP
#   server, so the registered engine is always the latest build:
#
#     claude mcp add --scope user logix-engine -- <repo>/scripts/serve.sh
#
#   Then start claude in any folder. Check with `claude mcp list`, and undo
#   with `claude mcp remove --scope user logix-engine`. Start a new claude
#   session after a code change to get the new build.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh
source scripts/_test-session.sh

repo="$(pwd)"
engine_folder="${LOGIX_ENGINE_DEV_DATA_FOLDER:-$HOME/.logix/engine-dev/engine-data}"
# The engine log level: error, warn, info (default) or debug, which also logs
# every program's output, such as a programmer's.
log_level="${LOGIX_ENGINE_DEV_LOG_LEVEL:-info}"
playground="${LOGIX_ENGINE_DEV_PLAYGROUND:-$HOME/.logix/engine-dev/playground}"

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
cargo build --release --locked
engine="$(copy_engine "$repo/target/release/logix-engine")"
echo "Engine $("$engine" --version | cut -d' ' -f2), engine folder $engine_folder, log level $log_level" >&2
echo "Claude Code: $claude_bin, in $playground" >&2

mkdir -p "$playground"
cd "$playground"

mcp_config=$(cat <<JSON
{"mcpServers": {"logix-engine": {"command": "$engine", "args": ["--engine-folder", "$engine_folder", "--log-level", "$log_level"]}}}
JSON
)

# A reproducible starting point: none of your user settings (they may add
# working folders, such as local repos), no CLAUDE.md files (yours
# included), no auto memory, no IDE connection, and the session rules.
# Slash commands stay on.
claude_settings=$(cat <<JSON
{"claudeMdExcludes": ["$HOME/.claude/CLAUDE.md", "**/CLAUDE.md", "**/CLAUDE.local.md"]}
JSON
)
CLAUDE_CODE_DISABLE_AUTO_MEMORY=1 CLAUDE_CODE_AUTO_CONNECT_IDE=false exec "$claude_bin" \
  --setting-sources project,local \
  --settings "$claude_settings" \
  --append-system-prompt "$SESSION_RULES" \
  --strict-mcp-config \
  --mcp-config "$mcp_config" \
  --allowedTools "mcp__logix-engine" \
  "$@"
