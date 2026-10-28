# Player media fixtures

`h264-initial-headers.avi` contains three seconds of generated FFmpeg `testsrc2`
at 64 × 48, 25 fps. It contains no user footage or audio. H.264 parameter sets
appear only in the first video packet, reproducing the EV300D AVI header layout.
The file is 21 KB; runtime builds do not include it.

Generate the initial AVI with a development FFmpeg providing libx264:

```sh
ffmpeg -f lavfi -i testsrc2=size=64x48:rate=25:duration=3 \
  -c:v libx264 -preset ultrafast -g 25 -x264-params repeat-headers=1 \
  -flags -global_header -an h264-initial-headers.avi
```

Use `ffprobe -show_packets -select_streams v -show_entries packet=pos,size -of json`
to locate video packets. In every packet after the first, replace NAL header bytes
whose type (`byte & 31`) is 7 or 8 following an Annex B start code (`00 00 01`)
with 12 (filler). This keeps RIFF lengths and index offsets unchanged while
removing repeated SPS/PPS. The first packet retains the original parameter sets.

`h264_indexed_seek_recovers_headers_without_replaying_the_take` compares an indexed
seek against a decode from the beginning. It fails with the old minimal bundled
FFmpeg and passes when its internal `extract_extradata` bitstream filter is enabled.
No libx264 dependency is required to run this test.
