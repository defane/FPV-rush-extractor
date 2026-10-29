# FPV Rush

A native Rust desktop application for organizing FPV recordings into sessions,
flights and tagged figures. Imports Fat Shark analog DVR and action-camera
takes, detects usable analog video between snow and menus, and exports lossless,
frame-exact cuts. The interface uses eframe/egui on macOS, Linux and Windows.

## Interface layout references

The native interface follows the layout of these earlier application captures:
compact navigation tabs and session summaries, thumbnail badges, tag pills and
pagination below the grid. The native player adds controls over the video and
adapts its size to the visible display.

| Sessions | Figures |
| --- | --- |
| ![Session layout](docs/screenshots/session.png) | ![Figure layout](docs/screenshots/figures.png) |

[Flight grid reference](docs/screenshots/flights.png) ·
[Player reference](docs/screenshots/player.png)

## Install

Distribution packages include FFmpeg and ffprobe. No Node.js, browser engine,
GStreamer, SDL or separate FFmpeg installation is needed.

- macOS: universal Intel/Apple Silicon DMG; drag FPV Rush into Applications.
  Requires macOS 11+. Builds use an ad-hoc signature unless a signing identity
  is supplied. Gatekeeper may require approving the app in Privacy & Security;
  ad-hoc signing is not Apple notarization.
- Windows: x64 per-user installer or portable ZIP. Extract the whole ZIP:
  keep resources/ beside fpv-rush.exe.
- Linux: AppImage, Debian package, or a tar.gz containing bin/ and lib/.
  Build on an old enough distribution for the glibc versions you support
  (CI uses Ubuntu 22.04). The desktop's graphics and ALSA libraries remain OS
  dependencies; the package is not a completely static executable.

There is one build for each OS, not one executable that runs on all of them.
GitHub Actions builds the installers on native Linux, macOS and Windows runners.
Artifacts are uploaded on pushes to main, version tags, pull requests and manual
runs. Certificate-based signing/notarization is not configured by this pipeline.

## Build and run

Install stable Rust through rustup and a C compiler. The product's UI is native
Rust with eframe/egui; there is no Tauri, webview, JavaScript build, or browser
runtime. SQLite and the separate FFmpeg tools are C dependencies.

Linux development dependencies on Debian/Ubuntu:

    sudo apt install build-essential pkg-config libasound2-dev libxkbcommon-dev libxkbcommon-x11-0 \
        libx11-dev libgl1-mesa-dev ffmpeg

macOS: Xcode command-line tools and Rust; Homebrew FFmpeg is useful for tests.
Windows: Rust's MSVC toolchain and Visual Studio C++ Build Tools; FFmpeg on PATH
is useful for tests. Building the bundled tools additionally needs MSYS2/MinGW
and make. NSIS produces the Windows installer.

    make app             # native release binary
    make media-tools     # build our LGPL-only FFmpeg/ffprobe from pinned sources
    make run             # native GUI; uses prepared tools, or tools on PATH
    make check           # clippy and all Rust tests
    make cli             # headless fpvlib CLI

For a native package:

    make macos           # on macOS; universal DMG by default
    make linux           # on Linux; deb + tar.gz + AppImage
    make macos-remote    # dispatch packaging.yml on GitHub from any host

Linux AppImage packaging downloads pinned, checksum-verified appimagetool/runtime
assets. APPIMAGETOOL can override the tool; FPV_SKIP_APPIMAGE=1 makes deb/tar only.
Windows packaging is run through:

    powershell -ExecutionPolicy Bypass -File packaging/windows/package.ps1

Run bash packaging/build-ffmpeg.sh native from a MINGW64 MSYS2 shell first.
For a single Mac architecture, use make macos TARGET=aarch64-apple-darwin or
TARGET=x86_64-apple-darwin. Rustup must have both Apple targets for a universal
build. Packaging scripts use the workspace's standard target directory.
Installers land in dist/.

Tests use generated H.264 fixtures and therefore require a development FFmpeg
with libx264. The redistributable tools deliberately omit libx264; the native
player's integration tests are also run against those exact bundled tools:

    cd fpv-rs
    cargo test --workspace --locked
    FPV_TEST_BUNDLED_TOOLS=../packaging/resources/bin cargo test -p fpv-app player::tests

The pinned Rust dependency graph is in fpv-rs/Cargo.lock. eframe handles the
window and drawing, rfd the system dialogs, image the posters, and cpal the
audio output. Video decode and playback coordination are local modules which
run the shipped FFmpeg programs. No libav libraries are linked to the app.

## Library and compatibility

Existing libraries open unchanged. The SQLite schema, migration ledger,
source-frame coordinates, IDs and config.json remain compatible with the
earlier desktop applications.

- Configuration and database: ~/.config/fpv_rush_extractor/ on Linux, the
  corresponding application-support directory on macOS and Windows.
- Recordings: ~/Videos/FPV Rush by default, adjustable in Settings.
- Disposable posters, hover frames and source previews: the OS cache directory.
  Old playback proxies can be cleared in Settings.

Back up the configuration directory and recordings. Browser preferences are
stored separately by eframe. Old Svelte browser preferences are not migrated.

## Workflow

Import a card shown in the sidebar or open Import Folder. Name the session and
supply the flying date for DVR footage. Copies preserve bytes and mtime; SHA-256
deduplicates repeated imports. GoPro chapters join into one flight. Optional
card cleanup removes only recordings successfully imported in that run.
Load destination registers previously copied recordings without copying again.
It skips recordings deleted with their flight unless their restoration is
explicitly selected. The source fork's cloud-drive behavior is retained:
Online Only files are marked, pictures never hydrate them implicitly, and
playing a flight first downloads its missing recordings with progress.
Session/flight menus offer Download and, on supported macOS cloud drives,
Free Up Space. Settings exposes separate preview/picture budgets and cache
cleanup by category. The import and session-edit dialogs use a calendar date
picker; the flying date remains optional for action-camera imports.
New import dialogs select today's local date. **No Sound** and its clickable
speaker icon apply to newly imported recordings only: their source bytes remain
intact, while playback and exports honor the persisted mute choice. Already known
recordings skipped during import keep their previous choice. Settings' global
**Mute videos** preference can also silence all playback and exports.

Long-running work appears in a collapsible Tasks panel along the bottom of the
main window, where progress, notifications, and task logs stay available
without floating notification windows. Settings includes Mute videos, which
silences player playback and exported video, and the splash-screen preference.
The About window displays the Letz Kwad logo, project link, and developer
credits.
It includes **Coded by Claude, Codex, Driven by Defane, Thanat0s** and a link to
[LetzKwad](https://letzkwad.lu/). On macOS the system About panel includes the same
branding and links through bundled rich-text credits. The splash dismisses
automatically with a 2.5× zoom and fade; it has no Close button.

Segment analyzes DVR recordings only. Picture statistics determine boundaries,
independently of OSD or armed state. It joins flights continuing into the next
recording. The per-session Textured terrain profile is available for a camera
chain or terrain the Standard thresholds split too aggressively. Re-segmenting
replaces DVR flights and their manual boundaries, titles, likes and flight tags;
figure tags on source recordings and action-cam takes survive.

Sessions, All flights and Figures support filtering, sorting and pagination.
Cards show the recording source and format, likes, tags and reduced status.
Hover a card to load cached filmstrip frames. Session menus expose details,
profile, folders, relocation, labels export, reduction and removal.

The native player decodes the original files, including MJPEG-in-AVI and HEVC.
Each part of a multi-file flight keeps its own rate and source times. Playback
scales to at most 1280x720 and 30 display frames/s; it never changes source frames
or export accuracy. Audio drives the playback clock when an output device is
available; silent playback remains possible without one. The decoder is
software-based in this migration, so very large HEVC takes may buffer more than
the previous hardware-backed WebView player.
Indexed seeking avoids decoding from the beginning; a sequential fallback handles
inputs whose indexes still fail. See [buffering measurements](docs/player-buffering.md).

In the player, playback and video are on the left while flight tags and figures
are on the right. Videos open in a compact window; Maximize fills the available
application area and Restore returns to the previous window size. Reopening a
video starts compact again. The player cannot be collapsed; its controls include a
fullscreen toggle, Escape to exit fullscreen, and an X close button. Mute videos
also silences player audio, in which case the volume slider is replaced by a
muted indicator.
The progress bar is overlaid transparently on the video, with a vertical volume
slider at its right edge. Click the video or press Space to play/pause; the
Play/Pause, Replay and clickable mute speaker buttons remain below the image.
Temporary speaker mute preserves the previous audible gain and does not change
export preferences. The
window stays inside the visible viewport, including when the viewport shrinks.

Space plays/pauses; Left/Right seek five seconds; I marks the start of a figure.
Enter in the figure-name field records a point or completes the marked range.
Click a figure to seek. Flight tags are separate from figure markers/ranges.

A thumbnail's **… → Info** shows each original recording's name, original and
library paths (selectable and copyable), stored video/audio properties and
source-relative clip times. End times and frame indexes are exclusive. Each
part keeps its own frame rate; missing or cloud-only sources still show their
stored metadata without downloading media.

Exports continue in **Tasks** after the export panel closes. A failed export
leaves no empty final file; completed outputs are checked for readable video
packets before publication. Existing destinations are never replaced.

Edit source boundaries steps through a recording's own frames, with source
previews and frame/quarter-second/second controls. Exports always read the
recordings. MJPEG cuts are lossless; long-GOP material is stream-copied only
from frame zero. Incompatible file rates require separate lossless parts or
an explicit re-encode. Multi-flight export uses unique filenames and reports
failures while continuing.

Reduce is an explicitly confirmed destructive operation: keep detected DVR
flights with five-second margins, move originals to the trash, preserve the
metadata inside kept footage. Reduced pieces cannot be re-segmented.
Removing a session can keep its recordings; deleting a flight only trashes
recordings that held nothing but that flight.

## Architecture

The workspace is under fpv-rs/. Core/application crates remain independent of
the GUI:

- fpv-core: model, source frame rates and flight timelines.
- fpv-media: FFmpeg process boundary, probing and source frames.
- fpv-detect: signal metrics and segmentation.
- fpv-store: SQLite and immutable schema migrations.
- fpv-import: discovery, copying, hashing and chapter grouping.
- fpv-pipeline: segmentation and cross-recording flight stitching.
- fpv-export: lossless cuts and explicit rate reconciliation.
- fpv-proxy: optional CLI MPEG-4/AAC derivatives and cache; the GUI reads sources.
- fpv-library: application operations and views.
- fpv-app: native egui window, background worker, bounded video/audio pipes.
- fpvlib: headless CLI.

CLAUDE.md records detector measurements and settled domain decisions. Detector
threshold changes still require the external hand-labelled tuning harness.

## Redistribution

The app's Rust code is MIT OR Apache-2.0. Bundled FFmpeg is built independently
from the pinned official archive, with GPL/nonfree components and optional
third-party autodetection disabled. Each package includes its LGPLv2.1 license,
exact unmodified source archive, SHA-256 and build recipe under
resources/licenses/ffmpeg (lib/fpv-rush/licenses/ffmpeg in Linux archives).
Distributors should retain these files and make the corresponding source
available alongside their download. See https://ffmpeg.org/legal.html.

FFmpeg 8.1.3 source: https://ffmpeg.org/releases/ffmpeg-8.1.3.tar.xz

The startup logo approaches continuously from a small distant size and fades as it zooms forward, without a pause. Disable it with Do not show splash in Settings.

Edit boundaries opens a source-video preview with a trim range: drag its left
handle to move the start, and its right handle to move the exclusive end. The
preview follows the edited source frame; numeric fields allow fine adjustment.
Select each source clip for multipart flights, then Save this clip to persist its
range. Closing without saving discards the draft and never changes source files.

On macOS, the application-menu About command opens the same branded window as
Settings About, including the grouped contributor credits and LetzKwad link.
The startup splash uses the transparent logo directly, without a white frame.

The fullscreen player covers the library completely. Play/Pause is hidden only
in fullscreen: click the video or press Space instead; windowed playback buttons
remain available. Resizing and Maximize grow the video to the remaining content
area while preserving aspect ratio, so only aspect-ratio letterboxing remains.
