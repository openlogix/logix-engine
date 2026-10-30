#!/bin/bash
# Deletes the build output (target/). Doesn't touch any engine folder.

set -euo pipefail
cd "$(dirname "$0")/.."

cargo clean
