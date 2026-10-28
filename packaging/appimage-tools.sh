#!/usr/bin/env bash
# Fixed release assets and SHA-256 digests from the official GitHub releases.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="$(uname -m)"
DEST="$ROOT/fpv-rs/target/appimage-tools"
case "$ARCH" in
    x86_64)
        toolsha=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
        runtimesha=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
    aarch64)
        toolsha=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
        runtimesha=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
    *) echo "Unsupported AppImage architecture: $ARCH" >&2; exit 1 ;;
esac
mkdir -p "$DEST"
fetch() {
    local url="$1" name="$2" hash="$3"
    if [ ! -f "$DEST/$name" ]; then
        curl --fail --location --retry 3 --connect-timeout 30 "$url" -o "$DEST/$name"
    fi
    printf '%s  %s\n' "$hash" "$DEST/$name" | sha256sum -c -
    chmod +x "$DEST/$name"
}
fetch "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$ARCH.AppImage" "appimagetool-$ARCH.AppImage" "$toolsha"
fetch "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$ARCH" "runtime-$ARCH" "$runtimesha"
