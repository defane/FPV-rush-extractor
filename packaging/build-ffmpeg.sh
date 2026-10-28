#!/usr/bin/env bash
# Build our own LGPL-only ffmpeg/ffprobe; no third-party binaries or GPL codecs.
# Usage: bash packaging/build-ffmpeg.sh [native|aarch64|x86_64|universal]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION=8.1.3
SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
ARCH="${1:-native}"
WORK="${FPV_FFMPEG_WORK:-$ROOT/fpv-rs/target/ffmpeg-build}"
# FFmpeg's configure refuses an out-of-tree build from a source path holding
# whitespace, and the pilot's checkout is ".../FPV Rush extractor": the build
# then goes to the system's temporary folder. A chosen FPV_FFMPEG_WORK is not
# second-guessed.
case "$WORK" in
    *[[:space:]]*)
        [ -z "${FPV_FFMPEG_WORK:-}" ] || { echo "FPV_FFMPEG_WORK must not contain spaces: $WORK" >&2; exit 1; }
        WORK="${TMPDIR:-/tmp}"
        WORK="${WORK%/}/fpv-rush-ffmpeg-build" ;;
esac
case "$WORK" in
    *[[:space:]]*) echo "No folder without spaces to build FFmpeg in; set FPV_FFMPEG_WORK" >&2; exit 1 ;;
esac
DEST="$ROOT/packaging/resources"
mkdir -p "$WORK" "$DEST/bin" "$DEST/licenses/ffmpeg"
ARCHIVE="$WORK/ffmpeg-$VERSION.tar.xz"
if [ ! -f "$ARCHIVE" ]; then
    curl --fail --location --retry 3 --retry-all-errors --connect-timeout 30 \
        "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz" -o "$ARCHIVE.download"
    mv "$ARCHIVE.download" "$ARCHIVE"
fi
if command -v sha256sum >/dev/null; then
    printf '%s  %s\n' "$SHA256" "$ARCHIVE" | sha256sum -c -
else
    printf '%s  %s\n' "$SHA256" "$ARCHIVE" | shasum -a 256 -c -
fi
if [ ! -d "$WORK/ffmpeg-$VERSION" ]; then tar -xJf "$ARCHIVE" -C "$WORK"; fi
SOURCE="$WORK/ffmpeg-$VERSION"
JOBS="${JOBS:-4}"

build() {
    local arch="$1" build="$WORK/build-$1" prefix="$WORK/install-$1"
    mkdir -p "$build"
    local platform=()
    case "$(uname -s)" in
        Darwin)
            local cpu="$arch"
            [ "$cpu" != aarch64 ] || cpu=arm64
            platform=(--target-os=darwin --arch="$cpu" --cc=clang --enable-cross-compile
                --sysroot="$(xcrun --sdk macosx --show-sdk-path)"
                --extra-cflags="-arch $cpu -mmacosx-version-min=11.0"
                --extra-ldflags="-arch $cpu -mmacosx-version-min=11.0"
                # Apple's own decoder, a system framework: the player needs it
                # for 4K HEVC (78 s for 30 s in software on the pilot's Mac).
                --enable-videotoolbox --enable-hwaccel=h264_videotoolbox,hevc_videotoolbox) ;;
        MINGW*|MSYS*) platform=(--target-os=mingw32 --arch=x86_64 --cc=gcc --extra-ldflags=-static) ;;
        Linux) ;;
        *) echo "Unsupported build host: $(uname -s)" >&2; exit 1 ;;
    esac
    (
        cd "$build"
        # Disabling autodetection prevents a developer's installed libx264,
        # TLS or other optional libraries from silently entering a release.
        "$SOURCE/configure" --prefix="$prefix" \
            --disable-autodetect --disable-gpl --disable-nonfree --disable-version3 \
            --disable-shared --enable-static --disable-doc --disable-debug \
            --disable-network --disable-ffplay --disable-x86asm --disable-everything \
            --enable-ffmpeg --enable-ffprobe --enable-avcodec --enable-avformat \
            --enable-avfilter --enable-avdevice --enable-swscale --enable-swresample \
            --enable-protocol=file,pipe,concat \
            --enable-demuxer=avi,mov,concat,matroska,image2,image2pipe,wav,rawvideo,pcm_f32le \
            --enable-muxer=avi,mov,mp4,matroska,image2,image2pipe,rawvideo,pcm_f32le,pcm_s16le,wav,framemd5,md5,null,adts \
            --enable-parser=mjpeg,h264,hevc,mpeg4video,aac,mpegaudio \
            --enable-bsf=extract_extradata \
            --enable-decoder=mjpeg,h264,hevc,mpeg4,aac,mp3,flac,opus,vorbis,adpcm_ima_wav,png,rawvideo,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,wrapped_avframe \
            --enable-encoder=mjpeg,mpeg4,aac,png,rawvideo,pcm_s16le,pcm_s24le,pcm_f32le,wrapped_avframe \
            --enable-filter=scale,fps,format,aresample,anull,null,trim,atrim,setpts,asetpts,concat,testsrc,testsrc2,smptebars,sine,anoisesrc,color,anullsrc \
            --enable-indev=lavfi "${platform[@]}"
        make -j"$JOBS"
        make install
    )
}

if [ "$ARCH" = universal ]; then
    [ "$(uname -s)" = Darwin ] || { echo "Universal builds need macOS and lipo" >&2; exit 1; }
    build aarch64
    build x86_64
    for bin in ffmpeg ffprobe; do
        lipo -create "$WORK/install-aarch64/bin/$bin" "$WORK/install-x86_64/bin/$bin" -output "$DEST/bin/$bin"
    done
else
    if [ "$ARCH" = native ] && [ "$(uname -s)" = Darwin ]; then
        case "$(uname -m)" in arm64) ARCH=aarch64 ;; *) ARCH=x86_64 ;; esac
    fi
    build "$ARCH"
    suffix=""
    case "$(uname -s)" in MINGW*|MSYS*) suffix=.exe ;; esac
    for bin in ffmpeg ffprobe; do cp "$WORK/install-$ARCH/bin/$bin$suffix" "$DEST/bin/$bin$suffix"; done
fi

# Corresponding, unmodified source and exact build recipe travel in EVERY
# archive/installer. Downstream distributors can mirror the source tarball.
cp "$ARCHIVE" "$DEST/licenses/ffmpeg/"
cp "$SOURCE/COPYING.LGPLv2.1" "$SOURCE/LICENSE.md" "$DEST/licenses/ffmpeg/"
cp "$0" "$DEST/licenses/ffmpeg/build-ffmpeg.sh"
printf 'FFmpeg %s\nSource: https://ffmpeg.org/releases/ffmpeg-%s.tar.xz\nSHA-256: %s\nNo modifications. Built with build-ffmpeg.sh.\n' \
    "$VERSION" "$VERSION" "$SHA256" > "$DEST/licenses/ffmpeg/BUILD.txt"
for binary in "$DEST/bin/ffmpeg"*; do
    "$binary" -L | tee "$DEST/licenses/ffmpeg/LICENSE-BUILD.txt"
    if "$binary" -buildconf 2>&1 | grep -Eq -- '--enable-(gpl|nonfree|libx264|libx265)'; then
        echo "Refusing a GPL/nonfree FFmpeg build" >&2; exit 1
    fi
    if [ "$(uname -s)" = Darwin ] && ! "$binary" -hide_banner -hwaccels | grep -qx videotoolbox; then
        echo "Refusing a Mac FFmpeg build without VideoToolbox" >&2; exit 1
    fi
done
echo "FFmpeg tools and corresponding sources prepared in $DEST"
