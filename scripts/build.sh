#!/bin/bash
# Builds the release engine binary: target/release/logix-engine.
# Release builds in CI set LOGIX_ENGINE_VERSION to the release tag. Local
# builds use today's date as the version.

set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --locked
ls -l target/release/logix-engine
target/release/logix-engine --version
