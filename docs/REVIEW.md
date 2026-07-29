# Application review

Review date: 2026-07-29

## Executive assessment

Oneiroi has moved beyond a proof of concept. Its strongest area is the media
engine: direct HAP playback, conventional-codec fallback, exact timestamp
scheduling, bounded workers, stale-generation rejection, four active decks,
camera inputs and project recovery are meaningful foundations for a real VJ
instrument.

It is not yet stage-ready as a standalone mixer. The first output slice now
separates an offscreen program render from the operator preview and clean second
window with aspect-preserving presentation. Display selection, custom sizing
and test patterns are implemented; output diagnostics and show-machine
validation remain before that milestone is complete.

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
- Scene launch, quantization, tempo, transport, effects, LFOs and modulation
  routes form a coherent playable instrument.
- Save, autosave, recovery and asynchronous restoration are already integrated.

### Compatibility and validation

- Project values are validated before application.
- Version-one projects migrate to the current version-two schema.
- The workspace currently passes 78 tests and strict Clippy.

## Stage-critical gaps

### 1. Program output is only partially complete

The shared GPU now owns independent operator/output surfaces, and the compositor
renders once into an offscreen program texture. The clean window can be hidden
or made borderless fullscreen at 720p, 1080p or UHD.

Connected displays can now be selected and refreshed, preset or custom
composition sizes are supported, and GPU test-card/identification overlays are
available. Remaining work: output-health diagnostics, stronger identity across
topology changes and show-machine soak testing.

### 2. A/B routing is gain-based, not a true two-bus composite

Each deck receives an A/B crossfade gain and is then composited in fixed deck
order. This works for the current Normal-style mix but is not the right semantic
base for per-layer blend modes or predictable bus behavior.

Resolution: composite Bus A and Bus B independently, then crossfade the two
results.

### 3. Layer composition controls are incomplete

The application lacks position, scale, rotation, crop, fit/fill/stretch and the
specified blend modes. Mirror and fractal operate as effects, not as a complete
layer-transform model.

### 4. Control and audio models are not connected to hardware

MIDI mapping state is device-neutral only. Audio has no capture or analysis
adapter. The modulation matrix is architecturally ready for more source types,
but only LFO sources exist.

### 5. Operational diagnostics are too shallow

FPS and aggregate dropped/repeated/late counts are visible, but there is no GPU
timing, upload timing, queue occupancy, per-deck decoder health, audio status,
MIDI status or output-display status.

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
    -> physical MIDI
    -> clip/media hardening
    -> effect-chain generalization
    -> packaging and soak testing
```

This order delivers the largest increase in real show usability while
protecting the media engine that is already working.
