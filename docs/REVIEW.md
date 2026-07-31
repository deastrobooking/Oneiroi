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
- The fixed deck effects are now organized as three reorderable persisted
  groups with shared bypass/dry-wet/reset behavior and factory presets.
- Two persisted master slots now route through a bounded post-composite graph.
  Separable blur reuses fixed horizontal scratch and ping textures, while an
  inactive chain retains the direct composition path.
- Persistent feedback samples the previous final program frame and owns
  explicit reset behavior for source/project/resolution/blackout/disable
  transitions. Master freeze now holds both final output and history.
- The master shader now has a versioned, path-safe package manifest with
  validated parameter ranges and WGSL entry points. Candidate pipelines compile
  on a watcher worker and replace the render pipeline only after validation;
  failures preserve the last-known-good output and remain visible to the
  operator.
- Custom one-pass master effects are registry-discovered by stable ID, receive
  schema-generated controls and persist named parameter values in project v3.
  Missing packages fall back to a neutral copy. Chromatic Split is bundled as
  an executable package/ABI example.
- Three master LFOs and eight routes address custom controls through stable
  package/parameter keys, with audio, transient, beat and bar sources. The same
  identity drives generated MIDI learning and readable persisted mappings.
- Custom packages may declare one or two fragment passes. Complete pipeline
  sets compile atomically, reuse the fixed scratch/ping targets and retain the
  previous set if either pass fails. Spectral Echo exercises the two-pass path.
- Temporal packages may request one fixed previous-slot-output history texture.
  Validity is explicit, resets follow source/project/blackout/disable lifecycle,
  and Temporal Melt verifies clean seeding and subsequent-frame sampling.
- Scene launch, quantization, tempo, transport, effects, LFOs and modulation
  routes form a coherent playable instrument.
- Save, autosave, recovery and asynchronous restoration are already integrated.
- The first deterministic-runtime spine is now present. Versioned typed node
  contracts compile the current four-deck topology into an immutable,
  budget-checked 11-node plan. Illegal implicit feedback is rejected, explicit
  delay nodes break temporal cycles, cross-rate edges receive adapter records,
  and non-overlapping transient texture lifetimes can share slots.
- Shadow graph preparation cannot replace the active or last-known-good plan
  unless the complete candidate validates and compiles. Ready transactions can
  commit on a frame, beat, bar or timecode boundary.
- The renderer now lowers the 11-node compatibility plan into three
  authoritative executable stages. It verifies four independent source/effect
  branches, linear color and matching extents before reusing the tested
  compositor, master processor and presenters. Unsupported lowering restores
  the previous graph plan.
- Primary launch, tempo and output-enable actions now enter a serializable
  command log with periodic checkpoints. Session state can replay to a target
  time, and takes can branch without rewriting recorded commands.
- The live take is also persisted through a bounded background writer. Its
  versioned JSONL stream and atomically replaced checkpoint recover after a
  torn final write without putting file I/O on the render thread. Queue
  overruns and worker errors are visible in the operator UI.
- MIDI and keyboard performance mutations now pass through one origin-aware
  command gateway before concrete application. Continuous UI controls are
  snapshotted, reverted and reapplied through that gateway, covering 192 fixed
  mixer/transport/effect/LFO/matrix targets plus dynamic custom-effect values.
  Launch, clear, eject, seek, tempo and output operations use typed semantic
  commands for deterministic replay.
- Structural UI edits are captured before/after each editor frame, restored,
  journaled as deterministic field commands, and only then accepted. This
  covers deck transforms/crop/source/blend, effect-slot and LFO/modulation
  structure, master effects/modulation, and successful media assignments.
- The operator can scan prior session journals, inspect recovery metadata and
  restore checkpoint-plus-tail state into concrete mixer/output/effect state.
  Recovery excludes the active writer and continues in a new baseline journal.

### Compatibility and validation

- Project values are validated before application.
- Version-one through version-four projects migrate to version five with
  stable identity, take metadata, deterministic seeds and the active graph.
- The workspace currently passes 189 tests and strict Clippy, with one extended
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

### Arbitrary package texture declarations are not implemented

Deck and master chains now provide ordered slots, common controls, bounded blur
and deterministic feedback history. Custom master shaders now register through
validated manifests, generate controls dynamically, receive stable-ID
LFO/audio/tempo modulation plus MIDI, and may use one or two atomic passes. The
current ABI deliberately grants 32 scalar parameters, the existing sampled
textures and at most one fixed previous-output history per slot; packages
cannot request arbitrary auxiliary texture counts, formats or resolutions.

Resolution: define a small, memory-budgeted resource declaration model without
allowing runtime allocation or unbounded shader resources.

### Project schema discipline

Custom effect instances established project version 3 and upgrade version-one
and version-two files on load. The next persistence work should extract
explicit migration steps and add golden fixture files for every supported
version before a fourth schema is introduced.

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
