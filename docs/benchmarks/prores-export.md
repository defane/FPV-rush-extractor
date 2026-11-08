# ProRes export validation

Linux ARM development measurements, 2026-11-07. Bundled FFmpeg 8.1.3,
`prores_ks`, profile 3 (422 HQ), `yuv422p10le`, without audio. Two-second
windows starting at source time 10 seconds; no scaling or rate conversion.
These are small development samples, not a broad perceptual quality benchmark.
The original private footage and generated output files are not committed.

| Local sample | Encoding time (s) | Output size (bytes) | PSNR average (dB) |
|---|---:|---:|---:|
| EACHINE — Flight 12.mp4 | 1.293 | 17877659 | 59.647139 |
| Indoor — Flight 13.mp4 | 1.275 | 16727881 | 62.386292 |
| EV300D_0002.AVI | 1.273 | 17667386 | 54.881185 |

PSNR compares decoded source and output after the same 10-bit 4:2:2 conversion
and timestamp reset, using the system FFmpeg PSNR filter. It measures the
additional encoding error in that representation, not chroma/bit-depth
conversion loss or perceived quality. Encoding time includes opening and
decoding the source before the requested window. ProRes HQ is lossy and can
produce substantially larger files than the original compressed recording.

Automated real-tool coverage checks exact interior H.264 cuts, single and
concatenated flights, separate parts, batch names, codec/profile/pixel format,
dimensions/rate/duration, metadata and global mute with system and bundled tools.
Missing-encoder and invalid-container/geometry tests preserve staged publication
and source files. The build keeps GPL/nonfree/autodetection disabled and enables
only FFmpeg's built-in ProRes encoder/decoder; no external codec library is added.

## Editor validation

Blackmagic's [Resolve 21.1 codec matrix](https://documents.blackmagicdesign.com/SupportNotes/DaVinci_Resolve_21_Supported_Codec_List.pdf)
lists ProRes MOV decode on macOS, Windows and Rocky Linux. H.264 decode on Linux
is Studio restricted. This motivated ProRes MOV rather than a universal H.264
MP4 promise.

On 2026-11-07 the user confirmed that exported output works in DaVinci Resolve,
accepting the export workflow for #39. The OS, Resolve version/edition and detailed
scrubbing/audio checks were not specified; this confirms the reported workflow,
not every combination in the vendor's compatibility matrix.

The native macOS universal build at `b990621` passed workspace tests, Clippy and
the installed-app smoke check in [GitHub Actions](https://github.com/defane/FPV-rush-extractor/actions/runs/42403890763).
A native Windows build for this change has not been run.
