# Unreleased

- Enable signal segmentation for every imported video codec, including H.264/HEVC; retain source labels independently, preserve clean EOF frames, warn before replacing all affected flights, and offer explicit re-encoding for precise long-GOP cuts (#28).

- Keep export image-integrity checks compatible with modern FFmpeg and the older Linux tools used by CI.

- Fix EV300D H.264/ADPCM AVI exports to MP4: preserve video packets and convert only incompatible audio to AAC, avoiding the reproduced zero-byte muxer failure (official #21).

- Add a full-original-video filmstrip to Edit boundaries, independent hover/scrub preview, selected-range shading and draggable start/end handles; load images incrementally (official #24).

- Support validated export publication on filesystems without hard links, including exFAT, while preserving no-overwrite and failed-copy cleanup (official #21).

- Remember each flight's playback mute across navigation and restarts, independently of import/export audio policies (official #25).

- Add thumbnail Info with copyable original/library paths, stored media metadata, availability and source-relative start/exclusive-end times for every recording (official #23).

- Close submitted export panels immediately, keep progress/errors in Tasks and reopen frame-rate recovery without interrupting another dialog (official #22).

- Validate exported video packets before publishing files; failed exports leave no empty or header-only destination and never replace an existing file (official #21).

- Moved notifications into the collapsible bottom task panel (#1).
- Decode long-GOP poster frames from the beginning when AVI indexes cannot seek into H.264 streams (#2).
- Seek through long-GOP footage by decoding from the input start, and keep the seek slider interactive (#3).
- Add a persisted, opt-in Mute videos setting for player playback and exports (#4, #14).
- Show a dismissible Letz Kwad splash screen with a persisted opt-out and animated zoom/fade exit (#5).
- Enlarge the splash to 2.5 times its initial size while fading out and remove its Close button (#5).
- Add an About window with the Letz Kwad logo, project URL, and developer credits (#7).
- Use calendar date pickers for recording imports and session edits (#6).
- Keep the Letz Kwad logo and credits visible in About and remove duplicate splash title text (#7).
- Restore the charcoal/orange palette, DVR/film metadata icons, and numbered orange pagination (#8, #10, #12).
- Place player tags and figures beside the video, with non-collapsible fullscreen and close controls (#11, #13).
- Add a speaker icon beside the player volume slider (#15).
- Update the README for the current native Rust/egui workflow and controls (#9).
- Reduce player startup/seek buffering with indexed decoding, retain sequential recovery, and enable FFmpeg's internal codec-header extraction and EV300D ADPCM decoder (#16).
- Include the LetzKwad logo, exact role credits and website in both the native macOS and in-app About views (#7).
- Restore reference navigation tabs/counts, session summaries, clean cards, figure badges and tag pills (#8).
- Keep the player inside the visible viewport, including after shrinking the app window (#17).
- Move pagination and its Showing counter below the thumbnail grid (#18).
- Select today's local date by default in new import dialogs (#19).
- Add import's No Sound checkbox and speaker toggle, persisting playback/export muting per recording without altering source files (#20).
- Overlay seek, playback and audio controls on the video (#21).
- Open videos in a compact player, add Maximize/Restore, and reset oversized saved geometry on reopening (#22).
- Group in-app About contributors into bulleted Coded by and Driven by sections (#24).
- Animate the splash continuously from a distant logo to a larger approach zoom, without a stationary hold (#23).
- Draw favourite hearts without a surrounding button box while preserving their click target (#29).
- Toggle temporary playback mute by clicking the speaker and restore the previous audible volume; forced no-audio policies remain authoritative (#25).
- Place the audible player volume slider vertically at the right edge of the video (#26).
- Remove the dark controls backdrop so seek and volume controls sit directly over the video (#27).
- Toggle playback by clicking the video or pressing Space while retaining Play/Pause, Replay and speaker buttons beneath the video (#31).
- Add a large native source preview and draggable left/start and right/end trim handles, with source-frame cadence, numeric fine adjustment and non-destructive per-clip saving (#28).
- Remove the white card and outer backdrop from the startup splash, preserving the transparent logo and continuous zoom/fade.
- Route the macOS About menu to the same Rust About window as Settings, keeping logo, grouped credits and links identical (#30).
- Give the fullscreen player an opaque viewport-sized backing, including gaps around controls, without adding a slider backdrop (#32).
- Hide Play/Pause only in fullscreen; retain windowed playback buttons, video-click/Space controls, audio and Replay (#33).
- Let video fill the remaining player area when resizing or maximizing, without the 480-point cap and while preserving source proportions (#34).
- Keep fullscreen text and controls readable on the opaque black surface when macOS or Linux uses a light theme (#32).
