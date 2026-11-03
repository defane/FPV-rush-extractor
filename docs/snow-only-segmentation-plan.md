# Snow Split — segmentation design (#35)

Status: implemented as an experimental selectable profile; broader validation is pending.
The user-facing algorithm name is **Snow Split**.

The original proposal below records the requirements and candidate approaches.
The implemented method and measured comparisons are documented in
[the benchmark report](benchmarks/snow-split.md). Its limitations mean #35 must
remain open for broader independent FPV evaluation and offline learned comparisons.

## Required behavior

Add a separate `Snow Split` choice to import and session detection controls.
Only positively identified analog RF static can create excluded intervals.
Black frames, goggle menus, low temporal coherence, short clips, disarming and
OSD timers must not independently cause a cut or discard footage. A recording
without confirmed snow retains its complete source-frame range; an all-snow
recording produces no usable clip. Standard and Textured retain their current
behavior and thresholds.

## Research basis and limits

- [Chen, Zhu and Heng, ICCV 2015](https://openaccess.thecvf.com/content_iccv_2015/html/Chen_An_Efficient_Statistical_ICCV_2015_paper.html)
  estimates noise through patch covariance statistics under an additive Gaussian
  model. This is a candidate statistical baseline, not an established model of
  compressed, banded analog static.
- [RAPIQUE, 2021](https://arxiv.org/abs/2101.10955) uses spatial and temporal
  bandpass statistics alongside learned semantic features for general video
  quality prediction. Its statistical features are a useful research direction;
  reusing ideas does not reproduce the full published model.
- [FAST-VQA, ECCV 2022](https://arxiv.org/abs/2207.02595) studies efficient fragment
  sampling for learned quality assessment.
- [KVQ, CVPR 2025](https://arxiv.org/abs/2503.10259) studies local distortion
  perception and saliency. It is a possible offline comparison, not a binary
  RF-loss detector.

These sources do not establish state-of-the-art FPV snow segmentation. Generic
quality, Gaussian noise estimation and weather-snow removal are different tasks.
Any learned comparison needs appropriate weights, licensing review and evaluation
on the same FPV labels. Do not add a neural inference dependency merely to claim
use of a recent paper. Record unavailable comparisons and domain-transfer limits.

## Original implementation proposal

1. Establish a snow-only baseline using the existing spatial measurements but
   separate its semantics from Black/Menu and whole-clip rejection. Measure its
   errors before proposing additional features.
2. Compare a dedicated patch-level noise score combining high-frequency energy,
   spatial coverage/uniformity and temporal predictability. Require positive
   noise energy so black or uniformly blank images cannot become snow merely
   because their correlation is low. Account for persistent OSD structure and
   horizontal bands with robust patch aggregation.
3. Temporal incoherence must never be sufficient by itself: project measurements
   already show that per-frame coherence rejection damages genuine flights.
   Compare joint evidence against fast motion, highly textured terrain, dim
   scenes and compressed/interlaced static. If adaptive calibration is tested,
   a recording without snow must not have its noisiest region forced into a
   snow class.
4. Choose thresholds and temporal confirmation from labelled comparisons.
   Existing hysteresis/gap settings are reference baselines, not validation of
   new thresholds. Keep short usable intervals and extend only genuinely clean
   EOF intervals to the true last source frame.
5. Add the profile's serialization, import/session selection and detection-run
   version/parameters. Keep source files unchanged, retain per-source clocks,
   and preserve reviewed/reduced-source protections. Implement runtime processing
   in Rust with the existing FFmpeg reader unless measurements justify otherwise.

## Validation and acceptance gates

The checkout contains three parity frames and the private Indoor, EACHINE and
EV300D recordings. No labelled CSV corpus was found outside temporary analysis
files. Earlier detector-generated clip boundaries are predictions, not ground
truth. The reported Indoor snow timestamps and EACHINE tail provide targeted
regressions but cannot establish full-file accuracy or cross-camera generalization.

Before tuning, annotate representative usable/snow intervals and reserve held-out
recordings or sessions. Do not split exports of the same recording between tuning
and evaluation. If data are insufficient, explicitly report a local regression
result rather than a held-out accuracy claim.

Report no-signal precision/recall, excluded useful seconds, retained snow seconds,
boundary error and false cuts, with annotated/unknown regions distinguished.
Compare both existing profiles and the snow-only baseline. Add synthetic cases
for OSD overlays, black/menu frames, short valid clips, static scenes, textured
motion, all-snow input and snow at each recording boundary. Report Linux ARM wall
time and peak memory on the same inputs and document which offline comparisons
were actually run.

Changes must pass failing-then-passing regressions, unchanged Standard/Textured
parity tests, workspace tests, strict Clippy, formatting and native UI checks.
Do not close #35 while required benchmark evidence remains missing. No macOS build
is part of this proposal; first validate the implementation under Linux.
