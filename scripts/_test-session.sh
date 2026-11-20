# Shared by run-in-claude.sh and run-in-grok.sh, for a reproducible starting
# point in hand-test sessions: the agent should know only what the engine
# tells it.

# Rules added to the agent's system prompt. They ask; they don't enforce.
SESSION_RULES="This is a test session of the Logix Engine. Start only from what the \
logix-engine MCP server provides (its instructions, its tool descriptions and the docs they \
link to) and from the folders the user names. Don't read the engine's source repo or other \
local clones, other projects' files, or notes and memories from earlier sessions. If \
something you need is missing, say so instead of looking for it elsewhere."

# Copies the built engine to ~/.logix/engine-dev/engine-program (or LOGIX_ENGINE_DEV_PROGRAM_FOLDER), so the
# agent's MCP config doesn't show the repo's path, and prints the copy's
# path. Copies to a temporary name and renames, so a session already running
# the old copy keeps working.
copy_engine() {
  local built="$1"
  local bin="${LOGIX_ENGINE_DEV_PROGRAM_FOLDER:-$HOME/.logix/engine-dev/engine-program}"
  mkdir -p "$bin"
  cp "$built" "$bin/.logix-engine.tmp"
  mv -f "$bin/.logix-engine.tmp" "$bin/logix-engine"
  echo "$bin/logix-engine"
}
