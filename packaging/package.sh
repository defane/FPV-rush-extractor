#!/usr/bin/env bash
# Native installers. Build on the destination OS; no JavaScript tooling.
# Usage: bash packaging/package.sh linux|macos [native|aarch64|x86_64|universal]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLATFORM="${1:?Expected linux or macos}"
ARCH="${2:-native}"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/fpv-rs/Cargo.toml" | head -1)"
DIST="$ROOT/dist"
RESOURCES="$ROOT/packaging/resources"
mkdir -p "$DIST"
for tool in ffmpeg ffprobe; do
    [ -x "$RESOURCES/bin/$tool" ] || { echo "Run make media-tools first (missing $tool)" >&2; exit 1; }
done
[ -f "$RESOURCES/licenses/ffmpeg/ffmpeg-8.1.3.tar.xz" ] || { echo "Missing corresponding FFmpeg source archive" >&2; exit 1; }

case "$PLATFORM:$(uname -s)" in linux:Linux|macos:Darwin) ;; *) echo "$PLATFORM packaging requires its native OS" >&2; exit 1 ;; esac
if [ "$ARCH" = native ]; then
    case "$(uname -m)" in arm64|aarch64) ARCH=aarch64 ;; x86_64) ARCH=x86_64 ;; *) echo "Unsupported architecture" >&2; exit 1 ;; esac
fi
WORK="$(mktemp -d "$ROOT/fpv-rs/target/package-$PLATFORM.XXXXXX")"
# Only our unique generated staging directory is removed on completion.
trap 'rm -rf "$WORK"' EXIT

if [ "$PLATFORM" = macos ]; then
    export MACOSX_DEPLOYMENT_TARGET=11.0
    case "$ARCH" in
        universal) targets=(aarch64-apple-darwin x86_64-apple-darwin) ;;
        aarch64|x86_64) targets=("$ARCH-apple-darwin") ;;
        *) echo "Unsupported Mac architecture: $ARCH" >&2; exit 1 ;;
    esac
    for target in "${targets[@]}"; do
        (cd "$ROOT/fpv-rs" && cargo build --release --locked -p fpv-app --target "$target")
    done
    APP="$WORK/FPV Rush.app"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    inputs=()
    for target in "${targets[@]}"; do inputs+=("$ROOT/fpv-rs/target/$target/release/fpv-rush"); done
    lipo -create "${inputs[@]}" -output "$APP/Contents/MacOS/fpv-rush"
    cp -R "$RESOURCES/." "$APP/Contents/Resources/"
    cp "$ROOT/fpv-rs/crates/fpv-app/icons/icon.icns" "$APP/Contents/Resources/icon.icns"
    cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$APP/Contents/Resources/licenses/"
    sed "s/@VERSION@/$VERSION/g" "$ROOT/packaging/macos/Info.plist.in" > "$APP/Contents/Info.plist"
    plutil -lint "$APP/Contents/Info.plist"
    for bin in ffmpeg ffprobe; do
        for target in "${targets[@]}"; do
            cpu="${target%%-*}"
            [ "$cpu" != aarch64 ] || cpu=arm64
            lipo "$APP/Contents/Resources/bin/$bin" -verify_arch "$cpu"
        done
        codesign --force --sign "${MACOS_SIGN_IDENTITY:--}" "$APP/Contents/Resources/bin/$bin"
    done
    codesign --force --sign "${MACOS_SIGN_IDENTITY:--}" "$APP/Contents/MacOS/fpv-rush"
    codesign --force --sign "${MACOS_SIGN_IDENTITY:--}" "$APP"
    codesign --verify --deep --strict "$APP"
    ln -s /Applications "$WORK/Applications"
    # Put just the app and Applications link into the image.
    hdiutil create -volname "FPV Rush" -srcfolder "$WORK" -format UDZO -ov "$DIST/FPV-Rush-$VERSION-macos-$ARCH.dmg"
else
    (cd "$ROOT/fpv-rs" && cargo build --release --locked -p fpv-app)
    STAGE="$WORK/fpv-rush"
    mkdir -p "$STAGE/usr/bin" "$STAGE/usr/lib/fpv-rush" "$STAGE/usr/share/applications" \
        "$STAGE/usr/share/icons/hicolor/128x128/apps"
    install -m755 "$ROOT/fpv-rs/target/release/fpv-rush" "$STAGE/usr/bin/fpv-rush"
    cp -R "$RESOURCES/." "$STAGE/usr/lib/fpv-rush/"
    cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$STAGE/usr/lib/fpv-rush/licenses/"
    cp "$ROOT/packaging/linux/re.ring0.fpv-rush.desktop" "$STAGE/usr/share/applications/"
    cp "$ROOT/fpv-rs/crates/fpv-app/icons/128x128.png" "$STAGE/usr/share/icons/hicolor/128x128/apps/fpv-rush.png"
    tar -czf "$DIST/FPV-Rush-$VERSION-linux-$ARCH.tar.gz" -C "$STAGE/usr" .
    case "$ARCH" in aarch64) debarch=arm64 ;; x86_64) debarch=amd64 ;; *) exit 1 ;; esac
    mkdir -p "$STAGE/DEBIAN"
    # dpkg-shlibdeps records linked runtime requirements from this build host.
    # Drivers, display server and audio devices remain supplied by the OS.
    glibc="$(getconf GNU_LIBC_VERSION | cut -d' ' -f2)"
    deps="libc6 (>= $glibc), libasound2, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libegl1, libgl1, xdg-utils"
    if command -v dpkg-shlibdeps >/dev/null; then
        detected="$(cd "$WORK" && dpkg-shlibdeps --ignore-missing-info -O \
            -e"$STAGE/usr/bin/fpv-rush" -e"$STAGE/usr/lib/fpv-rush/bin/ffmpeg" \
            -e"$STAGE/usr/lib/fpv-rush/bin/ffprobe" 2>/dev/null | sed -n 's/^shlibs:Depends=//p' || true)"
        [ -z "$detected" ] || deps="$detected, libasound2, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libegl1, libgl1, xdg-utils"
    fi
    printf 'Package: fpv-rush\nVersion: %s\nArchitecture: %s\nMaintainer: FPV Rush contributors\nSection: video\nPriority: optional\nDepends: %s\nDescription: Native FPV DVR flight library with bundled FFmpeg\n' \
        "$VERSION" "$debarch" "$deps" > "$STAGE/DEBIAN/control"
    dpkg-deb --build --root-owner-group "$STAGE" "$DIST/FPV-Rush-$VERSION-linux-$ARCH.deb"
    if [ "${FPV_SKIP_APPIMAGE:-0}" != 1 ]; then
        bash "$ROOT/packaging/appimage-tools.sh"
        TOOL="${APPIMAGETOOL:-$ROOT/fpv-rs/target/appimage-tools/appimagetool-$ARCH.AppImage}"
        RUNTIME="$ROOT/fpv-rs/target/appimage-tools/runtime-$ARCH"
        rm -r "$STAGE/DEBIAN"
        cp "$ROOT/packaging/linux/re.ring0.fpv-rush.desktop" "$STAGE/"
        cp "$ROOT/fpv-rs/crates/fpv-app/icons/128x128.png" "$STAGE/fpv-rush.png"
        cp "$ROOT/packaging/linux/AppRun" "$STAGE/AppRun"
        chmod +x "$STAGE/AppRun"
        ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --runtime-file "$RUNTIME" "$STAGE" "$DIST/FPV-Rush-$VERSION-linux-$ARCH.AppImage"
    fi
fi
echo "Installers: $DIST"
