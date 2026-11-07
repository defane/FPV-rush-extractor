# Snow Split: implementation and evaluation

Original evaluation: 2026-11-03; frozen-static follow-up: 2026-11-04. Status at evaluation: experimental, with outstanding validation
under #35. No source footage or review images are distributed with this report.

## Frozen-static follow-up (`snow-split-2`)

The original temporal test retained even a repeated image of full-frame random
static. A failing regression now covers that case. Version 2 additionally requires
at least 90% of patches to pass the existing spatial noise tests in both images,
with absolute bandpass-signature correlation at least 0.98. The same three-sample
run confirmation applies. Stationary pictures, menus, black frames, dim grain and
regular textures still pass their preservation regressions.

This stricter coverage is provisional: EV300D's largest spatial-noise coverage
was 0.791667 over the whole recording and 0.625 in its annotated useful windows.
The filmed-TV sample reached only 0.020833; this follow-up does not address that
optically filtered, strongly banded input. It does not relax the spatial guard
that previously caused destructive terrain false cuts.

[Version 2 measurements](snow-split-2-results.tsv) repeat all five recordings
with the rebuilt bundled Linux FFmpeg. The table below remains the historical
version 1 comparison. Version 2's static-noise rule is not a semantic guarantee:
a highly random-looking scene can resemble frozen snow. New independent FPV
recordings and offline learned comparisons are still needed before closing #35.

## Method

`fpv-detect/src/snow_split.rs` implements a separate classifier, not retuned
Standard/Textured thresholds. At four samples/second, FFmpeg decodes greyscale
images at no more than 640×480, without upscaling ordinary low-resolution input.
Dimensions are rounded down to multiples of 40×30 for an exact 8×6 patch grid.
Very small inputs use a minimum 40×30 grid. Standard/Textured retain their
original analysis dimensions and calculations.

Each patch needs all of the following positive spatial evidence:

- Median absolute 2×2 Haar residual of at least 0.5 in 8-bit luma units.
- Centered vertical-difference variance of at least 16. Centering prevents a
  constant alternating pattern from passing solely on its large differences.
- Absolute fine vertical Pearson correlation below 0.85.
- Absolute vertical correlation below 0.5 within its 5×5 region of a 40×30
  downsample. This guards against structured, highly detailed terrain.

Temporal evidence compares 64 signed bandpass residuals per patch against the
preceding sampled image, requiring absolute Pearson correlation below 0.5.
Spatial and temporal evidence must agree within each patch. A second-scale
prototype was rejected because it cut usable rocky terrain. The first sample is confirmed using the
second; a lone sample has insufficient temporal evidence and is retained.
Only the preceding signatures are needed for this added temporal calculation;
the existing diagnostic API still retains per-sample metrics and small images.

A strict majority of the 48 patches must qualify. Persistent OSD regions may
therefore remain structured while the rest demonstrates static. This coverage
requirement intentionally keeps limited-area and uncertain interference.
These thresholds are provisional local calibration, not published RF thresholds.

Three consecutive snow samples confirm an excluded run, backdated to its first
sample. At 4 Hz this requires evidence at 0, 0.25 and 0.5 seconds; excluded bins
cover at least 0.75 seconds. Shorter runs remain. Recovery retains the first
non-snow sample immediately, including a one-sample menu or picture. The
remaining intervals become clips without duration/coherence filtering or merging
through confirmed snow. Black, Menu, arming and OSD timers do not independently
exclude footage. Clean EOF uses the pipeline's actual source-frame count.

Clip confidence currently reuses the legacy heuristic value (0.9); it is not a
calibrated probability of signal or snow.

The profile serializes as `snowSplit`; current detection runs record `snow-split-2`,
sampling/hysteresis settings and every Snow Split threshold/scale constant.
Legacy parameter maps are unchanged. No migration, model weights or runtime
dependency was added. Original recordings are unchanged.

## Research decisions

| Primary reference | Relevance and decision |
| --- | --- |
| [Chen et al., ICCV 2015](https://openaccess.thecvf.com/content_iccv_2015/html/Chen_An_Efficient_Statistical_ICCV_2015_paper.html) | Noise estimation from patch covariance under additive Gaussian assumptions. Compressed RF static need not obey those assumptions. Snow Split uses a robust residual statistic; it does not implement the paper's covariance/eigenvalue estimator. |
| [RAPIQUE, 2021](https://arxiv.org/abs/2101.10955) | Spatial/temporal bandpass statistics motivate examining local residuals at multiple scales. The full quality model includes learned semantic features and is not reproduced here. |
| [FAST-VQA, ECCV 2022](https://arxiv.org/abs/2207.02595) | Efficient fragment sampling informs the bounded local-analysis approach. Its learned video-quality score is not a binary RF-loss label. No inference runtime is shipped. |
| [KVQ, CVPR 2025](https://arxiv.org/abs/2503.10259) | Saliency and local texture matter to perceptual quality. A quality score alone cannot distinguish a clean black/menu image from lost RF signal under Snow Split's preservation policy. |
| [Spatiotemporal Blind-Spot Network, AAAI 2025](https://ojs.aaai.org/index.php/AAAI/article/view/32242) | Noise-sensitive temporal alignment is relevant, but denoising and RF-loss detection require different targets. No denoising model was implemented or evaluated. |
| [CP-LLM, ICIP 2026, revised May 2026](https://arxiv.org/abs/2505.16025v4) | A newer context/pixel-aware learned quality model. Its quality scores and descriptions do not establish a calibrated RF-loss decision rule. No learned comparison was run. |

The engineering choice is a portable, inspectable statistical baseline with no
new dependencies. It is not a reproduction of these methods or a state-of-the-art
accuracy claim. An offline learned comparison still needs suitable weights,
resources and FPV labels to calibrate a binary decision without test leakage.
That part of #35 remains outstanding; no superiority over neural methods is claimed.

## Corpus and leakage controls

[snow-split-labels.tsv](snow-split-labels.tsv) contains visual annotations, not
detector predictions. Metrics only score fully annotated sample bins. Unlabelled
regions are counted separately; whole-file accuracy cannot be inferred from them.

- EACHINE Flight 12: 40.767 seconds, H.264, 720×576, 30 fps. Local calibration:
  usable through 22.25 seconds, then snow. It includes a dark static tail.
- Indoor Flight 13: 600.067 seconds, H.264, 720×576, 30 fps. Visually reviewed
  development windows include pure static starting at 111 and 207.5 seconds.
  Mixed, recognizable interference at 128–132.75 and 214.25–242 is deliberately
  unlabelled. Earlier exploratory labels treated those mixed intervals as snow;
  that interpretation was withdrawn before this final run, not used as evidence
  of improved accuracy. The final scored duration is only 47.5 seconds.
- EV300D_0002: 340.5 seconds. Two 16-second windows contain usable rocky terrain.
  A rejected prototype without the coarse spatial guard lost 19/32 useful seconds
  and made seven false cuts. This recording informed model selection and is
  development validation, not held out.
- External optical TV static: [Analog TV noise](https://commons.wikimedia.org/wiki/File:Analog_TV_noise.ogv),
  JussiClone, CC BY-SA 3.0. This is a camera filming a CRT with its bezel visible,
  **not a headset DVR**, and cannot validate generalization across FPV receivers.
- External clean motion: [OpenCV vtest.avi](https://github.com/opencv/opencv/blob/4.x/samples/data/vtest.avi),
  79.5 seconds, 768×576, 10 fps, people walking in a plaza. This is a negative
  control outside FPV; it does not supply independent positive RF-loss labels.

The two external inputs were visually reviewed before their first detector run.
The first external run evaluated the subsequently rejected two-scale variant;
the final single-scale variant was then rerun. The decision to reject the second
scale came from EV300D development footage, not external outcomes, and no thresholds
were fitted on external results. These are exploratory external checks, not a
preregistered independent test of the final model. Both require decoders
outside the bundled FFmpeg subset, so system FFmpeg converted them to MJPEG AVI
with `-an -c:v mjpeg -q:v 2`, preserving dimensions and nominal frame rates.
Optical TV's resulting video duration is 10.937333 seconds. Derivative SHA-256:
`analog-tv-noise.avi`: `40d6516cb26c5639b8610fe6e81a1e12bb049b4abeefc2cfbb7c2f3a0efdb496`;
`opencv-vtest.avi`: `3bd9df3a2f6f04d41071543f22d07f19dd665b71e6c3c29766da116cd0f5ad1c`. Only numeric labels
and results are committed; original and converted media remain local.

Original SHA-256 checksums:

```text
Analog_TV_noise.ogv 99fdbb31314f54d94120d3235b043373c99e6616b9a6eec3dd77c9097118be6b
vtest.avi          45cddc9490be69345cbdab64ca583be65987e864ca408038e648db99e10516cf
```

OpenCV sample Git blob: `965ab12bdb5e498297cefb476e4efb8204c6a29a`.
Before the first external inference, the **rejected two-scale** source SHA-256
checksums were frozen (retained for traceability, not hashes of the final source):

```text
snow_split.rs    90be1fa22b6f3a9e3f0805e7a48943b2864b4ecbc7a6ab62054536dd175bc324
file_detector.rs 916cf9b1afe05c161fa20cb757bc1969ce03391d82bcc3cd3aac8b35905bdcc6
```

The final single-scale `snow_split.rs` SHA-256 is
`0217f6835b3a23a95557a4c1d043c01f6f883b9b4ebe1136df3a71432ccf1965`;
`file_detector.rs` is unchanged from the hash above.

## Measurements and reproduction

Run the release example using the same FFmpeg tools for all profiles:

```sh
cd fpv-rs
cargo build -p fpv-detect --example snow_split_benchmark --release
PATH="../packaging/resources/bin:$PATH" target/release/examples/snow_split_benchmark \
  snowSplit '/path/to/EACHINE — Flight 12.mp4' ../docs/benchmarks/snow-split-labels.tsv
```

Repeat for `standard` and `textured`. Precision/recall treat excluded snow as the
positive class, duration weighted. `raw_noise_*` scores the classifier before
interval confirmation. `false_cuts` counts SignalLoss clip ends strictly inside
annotated usable intervals. Boundary error is distance to the nearest predicted
boundary of the matching kind; missing means none exists. It is a sample-grid
measurement, not a frame-accurate transition annotation. A denominator with no
examples is `n/a`, not perfect accuracy.

The results TSV records full-recording decode/analyze wall time on Linux aarch64,
release profile, single runs on a shared machine. Timing is descriptive rather
than a controlled performance comparison. Peak RSS is Linux `ru_maxrss` from a
fresh process wrapper: the maximum individual child-process RSS, not summed
simultaneous process-tree memory. FFmpeg decode and its internal threading are
included; annotation parsing/scoring is outside Rust's decode/analyze timer.

## Results

[Full numeric results](snow-split-results.tsv) include classifier scores,
unknown duration and resource measurements. Kept/lost seconds concern annotated
regions only; runtimes decode the entire recording.

| Input | Profile | Useful kept / lost (s) | Snow retained (s) | False cuts | Snow precision / recall | Wall (s) | Peak process RSS (MiB) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| EACHINE — Flight 12.mp4 | snowSplit | 22.25 / 0.00 | 0.00 | 0 | 1.000000 / 1.000000 | 1.717 | 31.0 |
| EACHINE — Flight 12.mp4 | standard | 22.25 / 0.00 | 0.00 | 0 | 1.000000 / 1.000000 | 1.059 | 24.8 |
| EACHINE — Flight 12.mp4 | textured | 22.25 / 0.00 | 0.00 | 0 | 1.000000 / 1.000000 | 1.047 | 25.5 |
| EV300D_0002.AVI | snowSplit | 32.00 / 0.00 | 0.00 | 0 | n/a / n/a | 11.934 | 27.9 |
| EV300D_0002.AVI | standard | 13.00 / 19.00 | 0.00 | 1 | 0.000000 / n/a | 12.632 | 30.2 |
| EV300D_0002.AVI | textured | 32.00 / 0.00 | 0.00 | 0 | n/a / n/a | 13.731 | 29.7 |
| Indoor — Flight 13.mp4 | snowSplit | 23.75 / 0.00 | 0.00 | 0 | 1.000000 / 1.000000 | 22.827 | 35.7 |
| Indoor — Flight 13.mp4 | standard | 23.75 / 0.00 | 0.00 | 0 | 1.000000 / 1.000000 | 14.271 | 31.6 |
| Indoor — Flight 13.mp4 | textured | 23.75 / 0.00 | 0.00 | 0 | 1.000000 / 1.000000 | 22.116 | 31.6 |
| analog-tv-noise.avi | snowSplit | 0.00 / 0.00 | 10.94 | 0 | n/a / 0.000000 | 0.602 | 9.7 |
| analog-tv-noise.avi | standard | 0.00 / 0.00 | 10.94 | 0 | n/a / 0.000000 | 0.647 | 13.8 |
| analog-tv-noise.avi | textured | 0.00 / 0.00 | 10.94 | 0 | n/a / 0.000000 | 0.702 | 13.8 |
| opencv-vtest.avi | snowSplit | 79.50 / 0.00 | 0.00 | 0 | n/a / n/a | 2.140 | 16.5 |
| opencv-vtest.avi | standard | 79.50 / 0.00 | 0.00 | 0 | n/a / n/a | 1.448 | 19.6 |
| opencv-vtest.avi | textured | 79.50 / 0.00 | 0.00 | 0 | n/a / n/a | 1.903 | 22.4 |

Snow Split matches both baselines on the annotated EACHINE and Indoor snow
intervals, with zero measured loss-boundary error at the three annotated events.
It keeps both EV300D usable windows, matching Textured while Standard loses 19
seconds and makes one false cut. These are development results, not proof of
improved accuracy on unseen FPV recordings. All three retain the entire external
optical-TV static clip: **zero snow recall on that input**. All retain the clean
OpenCV video. At that evaluation the profile remained experimental. It has since been accepted
by the user as the Snow Split product option; these accuracy limitations remain.
Its demonstrated distinction is its snow-only preservation policy, not an
established general accuracy improvement.

## Rejected variants

The [two-scale trial](snow-split-rejected-two-scale.tsv) adds half-resolution
bandpass votes. It detects synthetic enlarged grain but loses 14.5 of 32 labelled
usable EV300D seconds with six false cuts. It is not shipped. Removing the coarse
spatial guard entirely was worse (19/32 seconds and seven false cuts). These
failures justify retaining conservative spatial structure checks rather than
using temporal randomness or large grain as sufficient evidence.

## Known limitations and outstanding acceptance

- Calibration and development share only three private recordings. There is no
  independent held-out FPV positive corpus. Do not advertise cross-camera gains.
- Frozen static needs the stricter version 2 evidence described above. Brief snow,
  limited-area snow and low-energy/strongly correlated static can remain. A
  single-sample recording cannot demonstrate temporal unpredictability.
- Heavy interference with a recognizable picture has no agreed keep/cut policy;
  it is not silently counted as a successful snow classification.
- Motion and texture can still resemble changing noise. Synthetic negatives and
  the annotated windows do not cover all terrain, lighting or codec chains.
- Sampling cannot preserve every transition frame; boundaries lie on a 0.25-second
  grid before conversion to each source's own frame rate.
- Broader FPV evaluation, frozen/banded-static coverage and offline learned
  comparisons remain required before claiming broad independent accuracy validation.
- No macOS build was dispatched for this change. Portable Rust and existing
  FFmpeg APIs are retained; native macOS/Windows execution remains unverified.

## Implementation checks

The final implementation passed 435 Linux ARM workspace tests, strict workspace
Clippy (`--all-targets -- -D warnings`) and the required app formatting check.
Coverage includes legacy parity, snow/OSD and spatial-negative regressions,
short recovered takes, true EOF frame counts, profile persistence and the real
import-dialog worker with all three profiles. The new profile initially failed
the selectable-profile regression; the import fixture also exposed the noise
correlation introduced by upscaling, corrected by native-size analysis.
A native Linux binary was built and checked with an isolated Xvfb startup.
No macOS packaging was requested or dispatched for this change.
