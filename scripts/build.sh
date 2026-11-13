#!/bin/bash
# Builds the release engine binary: target/release/logix-engine (.exe on
# Windows).
# Release builds in CI set LOGIX_ENGINE_BUILD_VERSION to the release tag
# (yyyy-mm-dd); the engine reports it as yyyy.mm.dd. Local builds use today's
# date.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

cargo build --release --locked
# logix-engine.exe on Windows (Git's bash sets OS=Windows_NT).
exe="target/release/logix-engine"
[[ "${OS:-}" == Windows_NT ]] && exe="$exe.exe"
ls -l "$exe"
"$exe" --version
