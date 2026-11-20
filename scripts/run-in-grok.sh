#!/bin/bash
# Builds the engine if needed, then starts an interactive Grok session with
# the engine connected as an MCP server, so you can prompt it:
#
#   scripts/run-in-grok.sh              # then type prompts
#   scripts/run-in-grok.sh "What version is Logix Engine?"
#
# The same as scripts/run-in-claude.sh, with Grok as the host. Arguments are
# passed to grok. Grok runs in a playground folder, ~/.logix/engine-dev/playground
# (or LOGIX_ENGINE_DEV_PLAYGROUND), where it can create designs, not in this repo. The
# engine uses the engine folder ~/.logix/engine-dev/engine-data (or
# LOGIX_ENGINE_DEV_DATA_FOLDER), and logs at LOGIX_ENGINE_DEV_LOG_LEVEL (info by default; debug
# also logs every program's output). The engine's tools are allowed without
# asking. Each run starts the freshly built engine, copied to
# ~/.logix/engine-dev/engine-program so the session doesn't see this repo's path.
#
# For a reproducible starting point, the session has Grok's memory off,
# loads none of Claude Code's files (such as ~/.claude/CLAUDE.md), and gets
# the rules in scripts/_test-session.sh: start only from what the engine
# provides, and don't read local repos or earlier notes. The rules ask the
# agent; they don't enforce.
#
# Grok has no option to hand it an MCP server for one session, so this
# script writes the engine into the playground's project config,
# <playground>/.grok/config.toml, on every run, and trusts the playground
# folder so Grok starts it. MCP servers in your own Grok config
# (~/.grok/config.toml) are loaded too.
#
# Unlike Claude Code, Grok doesn't accept --add-dir, so name work folders in
# the prompt instead; the engine's tools take any folder.
#
# Uses the grok command on the PATH, or else ~/.grok/bin/grok.
#
# Prerequisites: the same as scripts/run-in-claude.sh, with Grok instead of
# Claude Code, installed and signed in (run grok once by hand to sign in).

set -euo pipefail

# Grok 1.0.70 refuses --add-dir for every folder ("a session works only in
# its cwd"), so stop before building instead of opening a broken session.
for arg in "$@"; do
  if [[ "$arg" == --add-dir || "$arg" == --add-dir=* ]]; then
    echo "Error: Grok doesn't support --add-dir (a session works only in its" >&2
    echo "working directory). Name the folder in the prompt instead, such as" >&2
    echo "\"Build ~/work for the iCESugar v1.5\": the engine takes any folder." >&2
    exit 2
  fi
done

cd "$(dirname "$0")/.."
source scripts/_rust-env.sh
source scripts/_test-session.sh

repo="$(pwd)"
engine_folder="${LOGIX_ENGINE_DEV_DATA_FOLDER:-$HOME/.logix/engine-dev/engine-data}"
# The engine log level: error, warn, info (default) or debug, which also logs
# every program's output, such as a programmer's.
log_level="${LOGIX_ENGINE_DEV_LOG_LEVEL:-info}"
playground="${LOGIX_ENGINE_DEV_PLAYGROUND:-$HOME/.logix/engine-dev/playground}"

# Find grok.
grok_bin="$(command -v grok || true)"
if [[ -z "$grok_bin" ]]; then
  grok_bin="$HOME/.grok/bin/grok"
fi
if [[ ! -x "$grok_bin" ]]; then
  echo "Error: grok not found. Install Grok and run it once to sign in." >&2
  exit 1
fi

echo "Building the engine..." >&2
cargo build --release --locked
engine="$(copy_engine "$repo/target/release/logix-engine")"
echo "Engine $("$engine" --version | cut -d' ' -f2), engine folder $engine_folder, log level $log_level" >&2
echo "Grok: $grok_bin, in $playground" >&2

mkdir -p "$playground"
cd "$playground"

# Adds the engine to ./.grok/config.toml, or updates it there.
"$grok_bin" mcp add --scope project logix-engine -- \
  "$engine" --engine-folder "$engine_folder" --log-level "$log_level" > /dev/null

# A reproducible starting point: no Grok memory, none of Claude Code's
# files (CLAUDE.md, rules, skills, MCP servers), and the session rules.
export GROK_MEMORY=0
export GROK_CLAUDE_AGENTS_ENABLED=0 GROK_CLAUDE_RULES_ENABLED=0
export GROK_CLAUDE_SKILLS_ENABLED=0 GROK_CLAUDE_MCPS_ENABLED=0
export GROK_CLAUDE_HOOKS_ENABLED=0

# --trust: Grok starts a project's MCP servers only in a trusted folder.
exec "$grok_bin" \
  --trust \
  --rules "$SESSION_RULES" \
  --allow "MCPTool(logix-engine__*)" \
  "$@"
