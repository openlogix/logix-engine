#!/bin/bash
# Regenerates schemas/tools.json, the published copy of the tool schemas,
# and schemas/logix-debug-v1.json, the debug file's (debug.toml), from the code. Review the diff: once released, changes may only add tools
# or optional fields. (The board schema belongs to Logix Catalog.)

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

UPDATE_SCHEMAS=1 cargo test --quiet --test schemas
UPDATE_SCHEMA=1 cargo test --quiet --lib schema_file
git --no-pager diff --stat -- schemas/
