# FPV Rush

Empty your FPV memory cards, spend less time sorting footage, and track your
training sessions.

FPV Rush copies your recordings into an organized library, finds flights in
analog DVR footage, and helps you review manoeuvres and keep useful clips.

- **Free your cards:** import recordings, then optionally remove the copies on
  the card once their transfer has been verified.
- **Save review time:** browse detected flights instead of scrubbing through
  entire DVR recordings and stretches of snow.
- **Track your training:** organize sessions by date and location, tag figures,
  mark favourites, and return to the moments you want to work on.
- **Keep and share useful footage:** adjust clip boundaries and export selected
  flights, with audio or muted.

Available for **macOS, Linux and Windows**. Video tools are included in the
installation packages.

## Your flight library

- Group recordings into sessions with a date, location, notes and tags.
- Detect flights in any imported recording, separating usable video from snow,
  black screens and goggle menus, regardless of codec or camera name.
- Join a flight that continues across consecutive DVR files or GoPro chapters.
- Browse Sessions, All flights and Figures with thumbnails, filters, sorting
  and pagination.
- Mark favourites, name flights and tag manoeuvres at specific moments.
- Review and adjust clip boundaries before exporting.
- Export individual flights or a selection, with optional audio muting.
- Follow live import counts in the import dialog or bottom panel, without opening Tasks.

| Sessions | Figures |
| --- | --- |
| ![Sessions](docs/screenshots/session.png) | ![Figures](docs/screenshots/figures.png) |

[Flights](docs/screenshots/flights.png) · [Player](docs/screenshots/player.png)

## Install

Download the package for your system from the repository's
[releases](https://github.com/defane/FPV-rush-extractor/releases) or the artifacts
of a completed [Packaging run](https://github.com/defane/FPV-rush-extractor/actions/workflows/packaging.yml).

- **macOS 11 or later:** open the universal DMG and drag FPV Rush into
  Applications. Supports Intel and Apple Silicon. If macOS blocks the unsigned
  download, approve it in Privacy & Security.
- **Windows x64:** use the installer or extract the complete portable ZIP.
  Keep the `resources` folder beside `fpv-rush.exe`.
- **Linux:** use the AppImage, Debian package or compressed archive. For the
  archive, keep its `bin` and `lib` folders together.

## Import recordings

Select a detected card in the sidebar or choose **Import Folder**. Give the
session a name and choose the flying date; new imports default to today.

Recordings are copied to your library without changing their contents. Repeated
imports skip recordings already known to the library. **Segment videos after
import** is enabled by default: after copying, FPV Rush detects usable clips in
the new recordings, including H.264, HEVC and MJPEG. Follow copying and analysis
in the import dialog or bottom panel. Existing reviewed flights keep their edits.
Uncheck the option to copy first and run **Segment** later.

**No Sound** on import keeps the original audio intact but silences playback and
exports of newly imported recordings. It does not change recordings skipped as
already imported. **Mute videos** in Settings applies the default playback mute
and silences exported videos.

Use **Load from destination folder** to register recordings already copied to
your library folder without copying them again. Optional card cleanup removes
only recordings successfully imported during that operation.

## Detect and organize flights

Use **Segment** or **Re-segment** on a session to run detection again. Detection follows the
picture signal rather than the armed state or OSD timer. Review the results:
noise, interference and very textured scenes can still produce incorrect cuts.
The **Textured terrain** profile helps when the Standard profile splits good
footage too aggressively.

Re-segmenting replaces flights from all unreduced recordings, including manually
adjusted boundaries, flight names, favourites and flight tags. The confirmation
describes what will be lost. Figure tags attached to source recordings and
already reduced clips are retained.

Use session and flight tags to organize your library. Figure tags identify a
moment or a range inside a recording and let you jump back to a manoeuvre.

## Watch and inspect

Open a thumbnail to watch a flight. The player starts in a compact window;
**Maximize**, **Restore** and fullscreen controls let you change its size.
Tags and figures are shown beside the video.

| Control | Action |
| --- | --- |
| Click the video or press **Space** | Play / pause |
| **Left / Right** | Seek five seconds |
| Progress bar over the video | Seek to a position |
| Speaker icon | Mute / unmute playback |
| Vertical slider at the right | Adjust volume |
| **Escape** in fullscreen | Leave fullscreen |
| **I** | Mark the start of a figure |
| **Enter** in the figure-name field | Save a point or complete the marked range |

Playback mute is remembered for each flight, including after restarting the
application. It does not change export audio settings. Import's **No Sound**
recordings remain silent.

Choose **… → Info** on a thumbnail to view the original file name, original and
library paths, video/audio properties and clip start/end times. Paths can be
copied. Flights spanning several files show each recording separately, with its
own frame rate. End times and frame indexes are exclusive.

## Adjust boundaries and export

**Edit boundaries** shows the full original recording as an image strip below
the preview. Hover or scrub the strip to inspect the source at that position,
including outside the selected clip. Move the left handle for the start and the
right handle for the end, or use frame and time-step controls for fine adjustment. For multipart flights, select
the source clip to edit. **Save this clip** applies the range; closing without
saving discards the draft. Editing boundaries does not change source files.

Choose **Export** from a flight's menu or select several flights for batch
export. Exports continue in the bottom **Tasks** panel after the form closes.
Existing destination files are never overwritten; failed exports report their
errors in Tasks.

Analog MJPEG footage can be cut losslessly at exact frames. Action-camera
Complete H.264/HEVC recordings can be stream-copied. Precise cuts inside these
recordings require an explicit re-encode, which the export panel offers. When exporting
to MP4, audio that the container cannot carry (such as DVR ADPCM) is converted
to AAC while the video remains unchanged. Flights whose
recordings have incompatible frame rates can be exported as separate lossless
parts or explicitly re-encoded into one file.

## Storage and background work

Set your recording destination in **Settings**; the default is
`~/Videos/FPV Rush`. Keep backups of both your recordings and the application's
configuration/database directory. Cached thumbnails and previews can be cleared
in Settings without deleting recordings.

The collapsible **Tasks** panel shows operation progress, completed work and
errors. Imports and exports continue while you browse the library.

On supported macOS cloud drives, online-only recordings are marked without
being downloaded for thumbnails. Playing them downloads the missing files with
progress. Session and flight menus provide **Download** and **Free Up Space**
where available.

**Reduce recordings** is a separate, confirmed operation: it keeps detected
flights with five-second margins and moves the original DVR recordings to the
trash. Reduced footage cannot be re-segmented. Removing a session can keep its
recordings; deleting a flight only trashes recordings used exclusively by it.
