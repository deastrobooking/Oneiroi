# Prioritized implementation plan

This plan was revised after reviewing the application against the original MVP
notes. The ordering is based on stage usability and dependency risk, not on
which feature is most visually interesting.

## Current execution sequence

The August 2026 upgrade audit is being applied in this order:

1. **Certify the baseline.** Keep strict Clippy, workspace tests, v1-v5 golden
   projects and the release build green. Run the separate physical-hardware
   matrix on the target show machine and record the binary hash.
2. **Reduce app and UI coupling.** Route UI intent through one action boundary,
   move output lifecycle ownership out of `State`, then split the remaining
   toolbar, setup and diagnostics panels. The action boundary is implemented;
   output ownership is next.
3. **Improve live diagnostics.** Add frame-time history, decoder/upload timing,
   dropped-frame visibility and actionable output-recovery detail.
4. **Finish distribution readiness.** Add bundle metadata and permissions,
   settle FFmpeg distribution obligations, then sign and notarize the macOS
   build.
5. **Deepen performance control.** Add MIDI feedback, clock/sync and controller
   templates after the current input path passes hardware soak testing.
6. **Expand graph/session authoring.** Continue from the compatibility graph to
   general execution and live editing only after the show-critical seams above
   are stable.
7. **Add stage integrations last.** Treat NDI/Syphon/Spout, SMPTE/Link and
   capture/recording as post-certification work so they do not destabilize the
   core release path.

## Phase 0: establish a reliable baseline

The completed feature slices have been preserved as intentional checkpoints.
Before another large subsystem lands:

1. Preserve validated feature slices as intentional checkpoints. (implemented)
2. Record release-mode performance for two and four simultaneous 1080p60 HAP
   sources using the matrix in `RELEASE_CHECKLIST.md`.
3. Maintain a repeatable local fixture set for HAP, conventional video, still
   and live-camera playback. The checklist is implemented; redistributable
   fixture media still needs to be produced.
4. Split application orchestration into focused output, media-session and
   action modules; keep `main.rs` as event-loop wiring. (in progress: UI action
   dispatch extracted; output lifecycle ownership is next)
5. Complete the operator UI split. Clip grid, deck, master FX, MIDI and theme
   modules are implemented; toolbar/setup/diagnostics remain in `ui.rs`.
6. Add checked-in golden v1-v5 projects and migration/save-reload tests before
   introducing project schema v6. (implemented)

Acceptance criteria:

- Full tests and strict Clippy remain clean.
- A release-build smoke test can be repeated from documented commands.
- No behavior changes are introduced by the module split.
- Every release candidate has a dated show-machine checklist and binary hash.

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
   and two master slots. (implemented)
2. Add common bypass, dry/wet, reset and preset behavior. (implemented for
   deck slots with five factory presets)
3. Implement separable blur. (implemented in the master chain with fixed
   ping-pong targets)
4. Implement persistent feedback/trails textures. (implemented with bounded
   final-frame history and deterministic lifecycle resets)
5. Define effect manifests and validated parameter schemas. (implemented for
   the master shader package with version, identity, path, entry-point and
   control-range validation)
6. Compile changed WGSL away from presentation and retain the last valid
   pipeline after errors. (implemented with background polling/compilation,
   GPU error scopes and an atomic render-thread pipeline swap)
7. Register custom master effects, generate controls from their schemas and
   persist named values. (implemented with a 32-parameter `master-v1` ABI,
   neutral missing-package fallback and bundled Chromatic Split example)
8. Route custom controls through modulation and MIDI without positional
   identity. (implemented with stable package/parameter keys, three master
   LFOs, eight audio/beat/bar-capable routes and generated MIDI learn controls)
9. Add a bounded declarative multipass package graph. (implemented with one or
   two fragment passes, atomic pipeline-set reload, fixed scratch/ping reuse,
   pass index/count uniforms and bundled Spectral Echo reference)
10. Add a safely budgeted temporal package resource. (implemented as one fixed
    previous-slot-output history texture per master slot, with validity
    signaling, deterministic resets and bundled Temporal Melt reference)

Acceptance criteria:

- Invalid effect code cannot blank program output.
- Reordering and bypassing are generation-safe and allocation-bounded.
- Feedback history resets predictably on source replacement and project load.

## Phase 7: release hardening

1. Add GPU upload/render timing and per-deck decoder-health diagnostics.
2. Add test-card and failure-mode rehearsal documentation. (implemented in
   `OPERATOR_GUIDE.md` and `RELEASE_CHECKLIST.md`; execution remains per build)
3. Create macOS bundle metadata with camera and microphone usage strings.
4. Package, sign and notarize the application.
5. Decide FFmpeg dynamic/static distribution and verify license obligations.
6. Run show-machine soak, suspend/resume, display reconnect and storage-failure
   tests.

## Phase 8: typed graph and deterministic session runtime

Status: the compatibility runtime, GPU lowering, command routing, recovery,
takes and bounded OSC transport are implemented. General graph execution and
live graph editing remain.

Delivered:

1. Device-neutral typed `ProjectGraph` and versioned node contracts.
2. Video, audio, control, event, async and external rate domains.
3. Port/type/required-input validation and explicit temporal cycle breaks.
4. Deterministic scheduling, rate-adapter insertion, resource lifetime reuse
   and hard GPU/texture budget checks.
5. Immutable `RenderPlan` with a compiled 11-node four-deck compatibility
   macro.
6. Shadow graph preparation that cannot mutate the active or last-known-good
   plan on failure.
7. Frame, beat, bar, exact-frame and timecode transaction scheduling.
8. Serializable `ShowCommand`, session state, checkpoints, replay, branches
   and named takes.
9. Runtime recording for graph activation, clip/scene launches, tempo changes
   and output enable changes.
10. Renderer lowering that validates the compatibility topology and turns the
    11 logical nodes into authoritative fused-composite, master-effect and
    program-output stages.
11. Graph-gated composition resizing and rollback to the previous plan when a
    candidate cannot be safely lowered.
12. A bounded background JSONL session journal with versioned headers,
    non-blocking render-thread enqueue, periodic fsync, atomic checkpoint
    replacement, torn-tail recovery and visible health counters.
13. An origin-aware application command gateway for every device-neutral MIDI
    target, keyboard emergency/mixer/scene control, semantic clip/transport/
    output commands and 208 built-in plus dynamic custom-effect UI values.
14. Record-before-apply structural snapshots for deck transforms, crop/source
    and blend modes, bus/solo/bypass, effect slots, LFO and modulation routing,
    master effects/modulation, plus accepted movie/folder/relink assignments.
15. An operator session-recovery catalog that excludes the active journal,
    validates checkpoint-plus-tail state, restores concrete controls and
    structures, and rotates to a fresh baseline journal.
16. Project schema v4 with stable project/take identities, bounded take
    metadata, linked journal headers, legacy compatibility and project-aware
    recovery filtering.
17. Project schema v5 with persisted typed graph and deterministic seed maps,
    compile-before-accept restoration, operator named takes and named recovery
    branches.
18. Full-history journal replay with checkpoint-aware timeline scrubbing plus
    safe project take metadata rename/removal that never deletes journal files.
19. First-class labeled timeline markers plus non-destructive take export and
    archive copies. Every copy receives a unique directory and retains both
    the journal and its atomic checkpoint when present.
20. Bounded OSC 1.0 UDP input with message/bundle decoding, visible malformed
    and overflow telemetry, and origin-aware command-gateway routes covering
    mixer, deck, clip, scene, tempo and output control.
21. Bounded asynchronous OSC state feedback with an initial control snapshot,
    plus NTP bundle-timetag conversion into a 1,024-event monotonic scheduler
    with a defensive 24-hour horizon and visible scheduling drops.

Next:

1. Add marker editing plus portable project/media manifests to exported takes.
2. Expand OSC effect/modulation routes and add route discovery.
3. Add shadow edit, preview, readiness and quantized commit UI.
4. Insert inferred color/resolution conversions and expose compiler
   diagnostics.
5. Add delay/rate-adapter executors and independent node passes where fusion
   is not valid.

Acceptance criteria:

- The current four-deck show renders entirely from the compiled plan without
  visual or performance regression. (implemented through bounded pass fusion)
- An invalid shadow graph cannot change program output.
- A recorded take restores the same state and deterministic seeds at any
  checkpoint.
- Every live mutation has a sequence number, origin and execution time.

## Deferred beyond the focused release

Projection warping, multiple simultaneous program outputs, edge blending, ISF
import, generative sources, NDI, Syphon/Spout, Ableton Link, MIDI clock, DMX,
Art-Net, the visual score, spatial engine and redundant render cluster remain
beyond the current graph-foundation slice. OSC transport is implemented;
effect/modulation route expansion and discovery remain in Phase 8.

## Phase 9: stage integration

Start only after the Phase 7 release gate is repeatable.

1. Decide Ableton Link licensing. If approved, add it behind an optional
   feature and adapt its tempo/beat phase through the existing clock boundary.
2. Add MIDI clock output and Song Position Pointer through the device-neutral
   control layer.
3. Expand OSC effect/modulation routes and publish route discovery metadata.
4. Prototype NDI output in an optional crate after accepting the NDI SDK and
   redistribution terms; preserve a default build without the SDK.
5. Prototype native Syphon/Spout texture sharing separately per platform.
6. Add a final bounded projection-warp pass before pursuing multi-output edge
   blending.

Dependency policy:

- Keep `wgpu` 29 with `egui-wgpu` 0.35. Upgrade `wgpu`, `naga` and the egui
  renderer compatibility line together.
- Evaluate `cpal` and `midir` upgrades independently with audio/MIDI hardware
  reconnect tests; do not combine them with a render-stack migration.
- External SDK and copyleft license decisions are release gates, not implicit
  implementation details.
