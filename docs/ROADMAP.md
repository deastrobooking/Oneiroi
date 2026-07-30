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

Status: implementation complete; hardware validation remains. Native input
capture, bounded callback handoff, worker-thread RMS/FFT analysis, adaptive
normalization, live diagnostics, five audio sources and beat/bar phase sources
are implemented.

Implementation sequence:

1. Add input-device enumeration and a bounded audio callback adapter in
   `oneiroi-io`. (implemented)
2. Copy callback samples into a fixed-capacity queue; never perform FFT or UI
   work inside the callback. (implemented)
3. Publish smoothed broadband RMS, bass, mid, high and transient signals from
   an analysis worker. (implemented)
4. Add gain, noise floor, attack, release and normalization. (implemented)
5. Generalize matrix source identifiers beyond LFO 1–3. (implemented)
6. Add audio sources, beat phase and bar phase to the routing UI. (implemented)
7. Persist device-independent analysis and routing settings. (implemented)
8. Display input device, sample rate, queue overrun and signal-health status.
   (implemented)

Acceptance criteria:

- The render thread reads a snapshot and never waits on audio work.
- Disconnecting an input device resolves sources safely to zero.
- Band separation, envelope timing and transient behavior have deterministic
  signal-fixture tests. (implemented, including normalization convergence)
- Audio queue growth is bounded and overruns are observable.

## Phase 4: physical performance control

Implemented. The device-neutral mapper is now connected to native hardware and
is operable without editing project JSON.

Implementation sequence:

1. Platform MIDI input and device enumeration. (implemented)
2. Learn/cancel/clear/remove UI around exposed controls. (implemented)
3. Note, CC, pitch bend, binary-offset and two's-complement encoders.
   (implemented)
4. Clip, scene, transport, mixer, effect, LFO, matrix and emergency targets.
   (implemented)
5. Pickup/soft-takeover, editable ranges and mapping activity. (implemented)
6. Controller reconnection and device-missing behavior. (implemented)

Acceptance criteria:

- MIDI callbacks use bounded `try_send` and cannot block rendering.
- Reconnecting a known controller restores its persisted mappings.
- Emergency blackout/freeze actions bypass launch quantization.
- Soft takeover prevents parameter jumps after project load.

Remaining MIDI work is output feedback, MIDI clock/sync, controller templates
and physical-device soak testing.

## Phase 5: clip readiness and media hardening

1. Preload and retain a bounded first-frame launch preview for every ready
   slot. (implemented at 640×360 maximum)
2. Add clip in/out points, restart/resume mode and BPM-relative duration.
   (implemented)
3. Build keyframe indexes for conventional-codec seeks. (implemented with a
   65,536-entry per-clip cap)
4. Add reusable CPU frame leases and steady-state allocation instrumentation.
   (implemented for conventional RGBA decode)
5. Add folder import. The fixed-size preload policy is implemented.
   (implemented with bounded recursive scanning and 32-slot assignment)
6. Add missing-media browsing and explicit relink. (implemented with a native
   per-slot picker, preserved playback settings and stale-result rejection)
7. Add decoder failure injection and long-running soak tests. (implemented
   with one-shot generation-scoped faults, accelerated allocation/generation/
   reopen coverage and an opt-in 10,000-reopen decoder soak)

Acceptance criteria:

- A preloaded launch is uploaded in the launch frame, before full decode
  produces its first scheduled frame. (implemented)
- Seeking and looping cannot present obsolete generations.
- The 32-slot first-frame cache is bounded to 29,491,200 bytes plus 160×90 UI
  thumbnails. (implemented)
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
