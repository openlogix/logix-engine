#!/bin/bash
# Sends one prompt to Claude Code with the engine connected, prints the
# answer as plain text and exits, so it stays in the terminal's normal
# scrollback. Built on scripts/run.sh (same engine build, engine folder and
# playground; see its comments for the prerequisites).
#
# Usage: scripts/engine-cmd.sh [options] PROMPT...
#
#   -c, --continue     Continue the previous conversation, instead of
#                      starting a new one.
#   --add-dir DIR      Let Claude read files in DIR, such as a project
#                      folder. Can be repeated.
#   --allow-edits      Also let Claude create and edit files, such as to
#                      write a new design.
#   -h, --help         Show this help.
#
# Examples:
#
#   scripts/engine-cmd.sh --add-dir ~/work "Build ~/work for the Alhambra II with the build folder _build/default."
#   scripts/engine-cmd.sh -c "Program the board."
#
# Claude can't stop to ask for permission here. The engine's tools and
# reading files are allowed; editing files only with --allow-edits.

set -euo pipefail

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

claude_args=()
tools=(Read Glob Grep)
while [[ $# -gt 0 ]]; do
  case "$1" in
    -c | --continue)
      claude_args+=(--continue)
      shift
      ;;
    --add-dir)
      [[ $# -ge 2 ]] || { echo "Error: --add-dir needs a folder" >&2; exit 2; }
      claude_args+=(--add-dir "$2")
      shift 2
      ;;
    --allow-edits)
      tools+=(Write Edit)
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "Error: unknown option $1. See --help." >&2
      exit 2
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -eq 0 ]]; then
  echo "Error: no prompt. See --help." >&2
  exit 2
fi
prompt="$*"

exec "$(dirname "$0")/run.sh" ${claude_args[@]+"${claude_args[@]}"} --allowedTools "${tools[@]}" -p "$prompt"
