#!/usr/bin/env bash
# Isolated library: never open the developer's real recordings during smoke tests.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/fpv-rs/target/release/fpv-rush}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export FPV_CONFIG_DIR="$WORK/config" FPV_CACHE_DIR="$WORK/cache" FPV_MEDIA_ROOT="$WORK/recordings"
export FPV_SMOKE_SECONDS=4
export PATH="$ROOT/packaging/resources/bin:$PATH"
timeout 25 xvfb-run -a "$BIN"
test -f "$WORK/config/library.sqlite"
echo "Native window opened and closed with an isolated library."
