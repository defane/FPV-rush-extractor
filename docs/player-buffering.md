# Native player buffering

The minimal FFmpeg build must include the internal `extract_extradata` bitstream
filter. `avformat_find_stream_info` uses it to retain H.264/HEVC parameter sets
before an indexed seek. Without it, an EV300D H.264 AVI that stores SPS/PPS only
at the beginning cannot decode after seeking. Replaying the file from zero avoids
that failure but makes startup progressively slower for later flight segments.

The player now seeks at the input for video and audio. If a non-intra video input
emits no frame, it retries by decoding from the beginning. Its first usable frame
starts the playback clock and ends the buffering state; the video queue remains
limited to four frames and audio to one second. Debug logs include the full FFmpeg
command, first-frame latency and whether the fallback was used.

The bundled decoder also includes `adpcm_ima_wav`, used by the supplied EV300D AVI.
These are FFmpeg's own LGPL components and add no external runtime library.

## Measurements

Measured on Linux aarch64 on 2026-10-26, using `debug/EV300D_0002.AVI`: H.264
Baseline, 720 × 576 at 30 fps, 340.5 seconds, 193,578,202 bytes. Each value is the
median of three warm-file runs from spawning FFmpeg to reading one complete RGB24
frame through the player's `fps=30,scale=720:576` filter. Processes were stopped
after the frame. The before version uses the previous output-side seek; the after
version uses input-side seeking with the rebuilt bundled FFmpeg 8.1.3.

| Source time | Before | After |
| --- | ---: | ---: |
| 0 s | 16.6 ms | 16.1 ms |
| 10 s | 142.3 ms | 21.0 ms |
| 180 s | 2462.5 ms | 19.7 ms |
| 330 s | 5098.6 ms | 19.1 ms |

SHA-256 hashes of the first decoded RGB frame match before/after at every time.
These measurements cover decoding startup, not worker queue time, cloud downloads
or display presentation. Cold-storage performance and other codecs depend on the
recording and hardware.

## Regression and recovery

The generated 21 KB `h264-initial-headers.avi` fixture reproduces the header layout
without user footage. The player test compares indexed and sequential seek frames
and checks buffering, pause and queue bounds. CI runs it with both system and
bundled FFmpeg on Linux; application tests also run on macOS and Windows.

Keep the output-side fallback when changing seek behavior. If indexed decoding
regresses, its logs identify the failing invocation, and the previous package
can be reinstalled without a database migration. Source recordings are unchanged.
