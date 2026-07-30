# Application review

Review date: 2026-07-30

## Executive assessment

Oneiroi has moved beyond a proof of concept. Its strongest area is the media
engine: direct HAP playback, conventional-codec fallback, exact timestamp
scheduling, bounded workers, stale-generation rejection, four active decks,
camera inputs and project recovery are meaningful foundations for a real VJ
instrument.

It is not yet stage-ready as a standalone mixer. The first output slice now
separates an offscreen program render from the operator preview and clean second
window with aspect-preserving presentation. Display selection, custom sizing
test patterns and output diagnostics are implemented; show-machine validation
remains before the milestone is stage-certified.

## What is strong

### Media and timing

- HAP remains block-compressed through GPU upload.
- FFmpeg fallback and still-image paths are isolated from the direct HAP path.
- Media uses exact rational timestamps.
- Decoder queues and schedulers are bounded.
- Generations prevent obsolete seek/restart frames from flashing.
- Camera capture drops backlog rather than accumulating latency.

### Rendering

- Composition and effects run on the GPU.
- Stable-resolution textures are reused.
- Linear/sRGB handling is deliberate and tested.
- GPU readback tests cover compressed upload, composition and effect changes.

### Performance workflow

- Four decks and 32 persistent slots are functional.
- Every successfully preloaded slot retains a bounded first-frame launch
  preview, avoiding a blank deck while full decoding starts.
- Per-slot trim, restart/resume launch mode and musical beat duration now feed
  the same generation-safe transport and seek path.
- Conventional clips build capped keyframe indexes and reopen from a preceding
  anchor before exact-target frame discard.
- Conventional RGBA frames now use per-deck reusable leases with observable
  steady-state allocation/reuse behavior.
- Bounded recursive folder import deterministically fills available slots and
  reuses the independent probe/preload pipeline.
- Native per-slot relinking preserves playback settings, records the selected
  path immediately and rejects superseded or cross-project probe results.
- Generation-scoped decoder failure injection proves mid-stream error
  reporting and recovery. Accelerated lease, seek-generation and real FFmpeg
  reopen soaks enforce bounded allocations and stale-frame rejection; an
  opt-in 10,000-reopen target is available for release candidates.
- Scene launch, quantization, tempo, transport, effects, LFOs and modulation
  routes form a coherent playable instrument.
- Save, autosave, recovery and asynchronous restoration are already integrated.

### Compatibility and validation

- Project values are validated before application.
- Version-one projects migrate to the current version-two schema.
- The workspace currently passes 128 tests and strict Clippy, with one extended
  decoder soak available as an opt-in ignored test.

## Stage-critical gaps

### 1. Program output requires hardware certification

The shared GPU now owns independent operator/output surfaces, and the compositor
renders once into an offscreen program texture. The clean window can be hidden
or made borderless fullscreen at 720p, 1080p or UHD.

Connected displays can now be selected and refreshed, preset or custom
composition sizes are supported, and GPU test-card/identification overlays are
available. Exact surface errors, recovery and topology changes are now visible
in the operator UI. Remaining work: stronger identity across topology changes
and show-machine soak testing.

### 2. Physical I/O still needs show-machine validation

MIDI and audio now both have native input capture, bounded queues, diagnostics
and safe missing-device behavior. MIDI adds automatic known-controller
reconnection and a complete learn/mapping editor. Both paths still require
permission, disconnect/reconnect and long-duration testing with physical
interfaces on the target show machine. MIDI output feedback and clock are not
implemented.

### 3. Operational diagnostics are too shallow

FPS, aggregate dropped/repeated/late counts, audio status, MIDI
received/dropped/parse counts and output-display health are visible. GPU
timing, upload timing, explicit queue occupancy and detailed per-deck decoder
timing remain absent. RGBA lease allocation/reuse/live/discard counters are
now visible.

## Engineering risks

### Concentrated application modules

`oneiroi-app/src/main.rs` is over 1,000 lines and `ui.rs` is over 900 lines.
They mix orchestration for media, projects, cameras, tempo, rendering and
operator controls. Dual-window output and audio device lifecycle will make
these files harder to reason about unless seams are introduced first.

Recommended boundaries:

- `output.rs`: window/surface lifecycle and display selection
- `session.rs`: per-deck decoder/scheduler/transport orchestration
- `actions.rs`: UI and keyboard action dispatch
- UI panels split by toolbar, clips, deck/effects and diagnostics

### Monolithic compositor and effect shader

The compositor Rust module and mixer WGSL now contain source interpretation,
UV effects, color effects, edge sampling, modulation structures and
composition. Adding transforms, blend modes, blur and feedback directly to this
shader would make validation and performance harder.

Resolution: finish bus composition first, then introduce explicit effect-pass
boundaries before multipass effects.

### Project schema discipline

The output milestone established project version 2 and upgrades version-one
files on load. The next persistence work should extract explicit migration
steps and add golden fixture files for every supported version before a third
schema is introduced.

### Uncommitted integration surface

The working tree contains the camera, clip-grid, persistence, thumbnail,
effects, modulation, tempo and documentation slices beyond the last commit.
This is recoverable locally but is too large a delta to begin dual-window
refactoring without first preserving an intentional checkpoint.

## Recommended decision

Proceed with the roadmap in this order:

```text
checkpoint and module seams
    -> dedicated program output
    -> true bus composition and layer transforms
    -> audio-reactive matrix sources
    -> physical MIDI/audio soak validation
    -> clip/media hardening
    -> effect-chain generalization
    -> packaging and soak testing
```

This order delivers the largest increase in real show usability while
protecting the media engine that is already working.
