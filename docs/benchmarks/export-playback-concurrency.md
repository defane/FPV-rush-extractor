# Export / playback scheduling validation

Linux ARM development validation for #42, 2026-11-08, using the bundled LGPL
FFmpeg 8.1.3. One synthetic four-second 640x480 MPEG-4 recording at 25 fps was
imported; single As is, single ProRes HQ and separate-part ProRes batch exports
ran while native playback descriptors and decoded source frames were requested.

| Export | Playback preparation (ms) | First decoded frame (ms) | Export result observed (ms) |
|---|---:|---:|---:|
| As is, single | 0 | 31 | 32 |
| ProRes HQ, single | 0 | 48 | 507 |
| ProRes HQ, batch | 0 | 10 | 484 |

Times are observed on this development machine, not service guarantees. A zero
millisecond result means below the timer's reporting precision. Export-result
observation includes event consumption and player cleanup, not just encoding.
The test harness completed in 1.167 s with 2.711 child user-CPU seconds and 0.086
child system-CPU seconds. Linux RUSAGE_CHILDREN reported 34,728 KiB largest-child
peak RSS; this is not aggregate simultaneous process-tree memory or a production
memory budget. This tiny synthetic case does not characterize 4K or long flights.

A separate deterministic worker test holds one export open and verifies that a
playback reply arrives before it is released, while a second export remains
queued. Thus the scheduling regression does not depend on machine speed or on
making an encoding test artificially slow. Only one export executor runs; no
additional codec or asynchronous runtime dependency is introduced.

Library regressions verify immutable range/metadata capture and reference-counted
source protection, including shared-flight sources and cloned export plans.
Deletion, reduction, relocation, cloud eviction and re-segmentation reject
conflicting requests with a reason while a source is leased. Playback reads stay
available. Source protection is released when the last prepared plan is dropped,
including completion/error unwinding. UI tests retain existing playback during a
new open request, preserve it after failure and reject late replies from older
selections. Existing export suites retain metadata, audio, exact-cut, no-overwrite
and failed-output cleanup coverage.
