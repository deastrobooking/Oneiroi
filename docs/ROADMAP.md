# Prioritized implementation plan

This plan was revised after reviewing the application against the original MVP
notes. The ordering is based on stage usability and dependency risk, not on
which feature is most visually interesting.

## Phase 0: establish a reliable baseline

The current working tree contains several completed feature slices beyond the
last commit. Before another large subsystem lands:

1. Preserve the current validated state as an intentional checkpoint.
2. Record release-mode performance for two and four simultaneous 1080p sources.
3. Add a repeatable fixture command for HAP, conventional video, still and
   synthetic camera playback.
4. Split application orchestration into focused output, media-session and
   persistence modules; keep `main.rs` as event-loop wiring.
5. Split the large operator UI into toolbar, clip-grid, deck, effects and
   diagnostics panels.
6. Extend the new version-two migration boundary with golden v1/v2 fixture
   files before adding audio settings.

Acceptance criteria:

- Full tests and strict Clippy remain clean.
- A release-build smoke test can be repeated from documented commands.
- No behavior changes are introduced by the module split.
- The repository has a recoverable checkpoint before dual-window work begins.

## Phase 1: dedicated program output

Status: implementation complete; hardware validation remains. The offscreen
program texture, clean second window, display targeting, enable/disable
control, borderless fullscreen, preset and custom resolutions, calibration
overlays and project persistence are implemented. Presentation preserves
composition aspect ratio with automatic letterbox or pillarbox bars. Surface
acquisition and display-topology health are observable in the operator UI.

Delivered:

1. Shared GPU instance/device/queue with independent surfaces
2. Single offscreen composition render
3. Operator preview and clean program presentation passes
4. Windowed, hidden and borderless fullscreen output
5. 720p, 1080p and UHD composition presets
6. Version-two project persistence and v1 migration
7. Aspect-preserving presentation
8. Connected-display enumeration, selection and refresh
9. Custom 320×180 through 7680×4320 composition sizing
10. GPU-rendered test card and output-identification overlay
11. Persistence for the selected display and calibration state
12. Exact surface acquisition status, recovery counters and operator diagnostics
13. Two-second display-topology polling and reconnect fallback

Remaining:

1. Harden display identity across topology changes with identical display models.
2. Complete show-machine display reconnect and long-run soak testing.

Acceptance criteria:

- The output window never contains operator UI.
- Output can move between displays and recover from surface loss.
- Closing or disabling output does not stop decoding or the operator window.
- Blackout, freeze and disable-output execute on the next rendered frame.
- Output resolution is independent from operator-window size. (implemented)
- Two 1080p60 HAP sources plus active effects sustain the target display rate
  on the show machine.

## Phase 2: correct layer composition

Status: implementation complete. Decks are composited into independent A and B
images in fixed order within each bus, then the completed bus images are
crossfaded. The layer-control model includes transforms, source modes, crop,
blend modes, Solo and Bypass.

Implementation sequence:

1. Define explicit Bus A and Bus B intermediate composites. (implemented)
2. Crossfade the two completed bus images instead of multiplying each deck's
   level by a bus gain. (implemented)
3. Add position, scale, rotation and horizontal/vertical flip. (implemented)
4. Add crop plus fit, fill and stretch source modes. (implemented)
5. Add linear-light Normal, Add, Screen, Multiply, Difference, Lighten, Darken
   and Overlay blend modes. (implemented and GPU tested)
6. Add per-deck solo and bypass. (implemented and GPU tested)
7. Persist and validate every composition field. (implemented)

Acceptance criteria:

- Bus results do not depend on the other bus's deck order. (GPU tested)
- Transform and crop behavior is consistent for HAP, RGBA, still and camera
  sources. (transform path implemented and GPU tested)
- Every blend mode has GPU readback coverage using known input colors.
- Neutral defaults render identically to the current mixer.
  (preserved through project compatibility defaults)

## Phase 3: audio-reactive modulation

After output and composition are stable, extend the existing source/destination
matrix instead of creating a separate audio-effect path.

Implementation sequence:

1. Add input-device enumeration and a bounded audio callback adapter in
   `oneiroi-io`.
2. Copy callback samples into a fixed-capacity queue; never perform FFT or UI
   work inside the callback.
3. Publish smoothed broadband RMS, bass, mid, high and transient signals from
   an analysis worker.
4. Add gain, noise floor, attack, release and normalization.
5. Generalize matrix source identifiers beyond LFO 1–3.
6. Add audio sources, beat phase and bar phase to the routing UI.
7. Persist device-independent analysis and routing settings.
8. Display input device, sample rate, queue overrun and signal-health status.

Acceptance criteria:

- The render thread reads a snapshot and never waits on audio work.
- Disconnecting an input device resolves sources safely to zero.
- Band separation, envelope timing and transient behavior have deterministic
  signal-fixture tests.
- Audio queue growth is bounded and overruns are observable.

## Phase 4: physical performance control

The device-neutral MIDI mapping model already exists. Connect it to hardware
and make it operable without editing project JSON.

Implementation sequence:

1. Add platform MIDI input and device enumeration.
2. Build learn/cancel/clear UI around exposed controls.
3. Support note, CC, pitch bend and common relative encoders.
4. Route clip, scene, transport, mixer, effect, LFO, matrix and emergency
   targets.
5. Expose pickup/soft-takeover and mapping activity.
6. Add controller reconnection and device-missing behavior.

Acceptance criteria:

- MIDI callbacks cannot block rendering.
- Reconnecting a known controller restores mappings.
- Emergency actions are not quantized.
- Soft takeover prevents parameter jumps after project load.

## Phase 5: clip readiness and media hardening

1. Preload and retain the first visible frame of every ready slot.
2. Add clip in/out points, restart/resume mode and BPM-relative duration.
3. Build keyframe indexes for conventional-codec seeks.
4. Add reusable CPU frame leases and steady-state allocation instrumentation.
5. Add folder import and a bounded preload policy.
6. Add missing-media browsing and explicit relink.
7. Add decoder failure injection and long-running soak tests.

Acceptance criteria:

- A preloaded launch becomes visible within two display frames.
- Seeking and looping cannot present obsolete generations.
- Memory remains bounded with all 32 slots populated.
- Missing files never prevent the rest of a project from opening.

## Phase 6: effect-chain architecture

1. Replace the monolithic fixed effect struct with three reorderable deck slots
   and two master slots.
2. Add common bypass, dry/wet, reset and preset behavior.
3. Implement separable blur.
4. Implement persistent feedback/trails textures.
5. Define effect manifests and validated parameter schemas.
6. Compile changed WGSL away from presentation and retain the last valid
   pipeline after errors.

Acceptance criteria:

- Invalid effect code cannot blank program output.
- Reordering and bypassing are generation-safe and allocation-bounded.
- Feedback history resets predictably on source replacement and project load.

## Phase 7: release hardening

1. Add GPU upload/render timing and per-deck decoder-health diagnostics.
2. Add test-card and failure-mode rehearsal documentation.
3. Create macOS bundle metadata with camera and microphone usage strings.
4. Package, sign and notarize the application.
5. Decide FFmpeg dynamic/static distribution and verify license obligations.
6. Run show-machine soak, suspend/resume, display reconnect and storage-failure
   tests.

## Deferred beyond the focused release

Projection warping, multiple simultaneous program outputs, edge blending, ISF
import, generative sources, NDI, Syphon/Spout, OSC, Ableton Link, DMX, Art-Net,
timelines and redundant show-machine operation remain outside the focused MVP.
