#!/bin/bash
# Regenerates schemas/tools.json, the published copy of the tool schemas,
# from the code. Review the diff: once released, changes may only add tools
# or optional fields.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

UPDATE_SCHEMAS=1 cargo test --quiet --test schemas
git --no-pager diff --stat -- schemas/tools.json
