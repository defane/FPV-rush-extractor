#!/usr/bin/env bash
# Exercise the real application-menu action in the packaged app, on the main thread.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$ROOT/tmp"
WORK="$(mktemp -d "$ROOT/tmp/macos-about.XXXXXX")"
mkdir -p "$WORK/mount" "$WORK/config" "$WORK/media" "$WORK/cache"
trap 'hdiutil detach "$WORK/mount" -quiet || true' EXIT
python3 - "$WORK" <<'PY'
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
(root / 'config/config.json').write_text(json.dumps({
    'mediaRoot': str(root / 'media'), 'doNotShowSplash': True,
}))
PY
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/fpv-rs/Cargo.toml" | head -1)"
hdiutil attach "$ROOT/dist/FPV-Rush-$VERSION-macos-universal.dmg" -readonly -nobrowse -mountpoint "$WORK/mount"
if ! FPV_CONFIG_DIR="$WORK/config" FPV_MEDIA_ROOT="$WORK/media" FPV_CACHE_DIR="$WORK/cache" \
    FPV_SMOKE_SECONDS=6 FPV_SMOKE_ABOUT_MENU=1 FPV_SMOKE_SCREENSHOT="$WORK/about.png" \
    "$WORK/mount/FPV Rush.app/Contents/MacOS/fpv-rush" > "$WORK/app.log" 2>&1; then
    cat "$WORK/app.log"
    exit 1
fi
if ! python3 -c 'import sys; assert "FPV_MACOS_ABOUT_MENU_SMOKE_OK" in open(sys.argv[1]).read()' "$WORK/app.log"; then
    cat "$WORK/app.log"
    echo "The packaged app did not dispatch its native About menu action" >&2
    exit 1
fi
test -s "$WORK/about.png"
echo "Packaged macOS About menu smoke passed; screenshot: $WORK/about.png"
