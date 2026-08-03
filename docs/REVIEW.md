# Application review

Review date: 2026-08-02

## Executive assessment

Oneiroi is a functional four-deck VJ instrument with a credible media and
rendering foundation. Direct HAP upload, bounded FFmpeg fallback, independent
A/B composition, deck and master effects, audio/MIDI/OSC control, clean program
output and deterministic session recovery are implemented and covered by more
than 200 automated tests.

The main release risk is no longer missing mixer fundamentals. It is show-machine
certification, application packaging and concentrated application orchestration.
External stage integrations such as tempo sync and video sharing should follow
those gates rather than displace them.

## Current strengths

### Media and rendering

- HAP remains block-compressed through GPU upload; conventional media and stills
  use isolated FFmpeg fallback paths.
- Exact media timestamps, bounded workers, reusable frame leases and generation
  checks prevent unbounded backlog and obsolete-frame presentation.
- Four decks feed independent linear-light A/B composites with 35 blend modes,
  transforms, crop, Solo/Bypass and per-deck effect chains.
- The program render is shared by an operator preview and clean second window.
  Display targeting, fullscreen, calibration overlays and surface recovery are
  observable rather than implicit.
- Two bounded master slots support built-in and validated one/two-pass WGSL
  packages with last-known-good reload and one optional history texture.

### Performance workflow

- The 4 × 8 clip grid supports scene launches, quantization, folder import,
  missing-media relink, safe slot movement and explicit clip deletion.
- The selected deck has one primary, always-visible editor. Deck row labels and
  clip slots retarget it directly; secondary deck editors no longer bury the
  active controls.
- Show Mode locks setup and destructive/structural edits while retaining clip,
  scene, transport, mixer, Solo/Bypass and live deck-FX controls.
- MIDI supports multiple persisted controllers, learn/clear, relative modes,
  soft takeover and reconnect. OSC input/output shares the command gateway and
  supports bounded timetag scheduling.
- Audio RMS, bands and transient analysis plus beat/bar phase feed the deck and
  master modulation systems.

### Persistence and validation

- Project schema v5 migrates supported v1-v4 projects, validates values and
  persists graph, take identity and deterministic seeds.
- Structural edits and performance controls enter an origin-aware command log.
  Bounded JSONL journals, checkpoints, recovery branches, markers and exported
  take copies are implemented.
- The workspace passes full tests and strict Clippy. One extended decoder reopen
  soak remains intentionally opt-in for release candidates.

## Stage-critical gaps

### 1. Hardware certification

The clean output, audio capture and multi-controller MIDI paths need a recorded
show-machine matrix: sustained 1080p/UHD playback, display disconnect/reconnect,
sleep/wake, audio permission and device loss, MIDI reconnect and storage failure.
Use [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) for every candidate.

### 2. Packaging and distribution

The repository does not yet produce a signed/notarized macOS application bundle.
Camera and microphone usage strings, FFmpeg distribution strategy and license
notices must be settled before calling a build stage-ready.

### 3. Diagnostics depth

FPS, surface health, decoder drop/repeat/late totals and RGBA lease counters are
visible. GPU pass/upload timings, queue occupancy and per-deck decode latency are
still needed to explain a marginal show machine without attaching a profiler.

## Engineering risks

### Concentrated application orchestration

`oneiroi-app/src/main.rs` remains roughly 1,700 lines and still coordinates
windowing, media, projects, cameras, tempo, rendering and action dispatch. The UI
has begun splitting into `clips`, `deck`, `master_fx`, `midi`, `midi_manager` and
`theme`, but `ui.rs` is also still roughly 1,700 lines.

Next seams:

- `output.rs`: output window/surface lifecycle and display selection
- `media_session.rs`: per-deck decoder, scheduler and transport orchestration
- `actions.rs`: UI and keyboard action dispatch through the command gateway
- UI toolbar/setup/diagnostics modules to complete the panel split

These are behavior-preserving refactors and should land in small validated
checkpoints.

### Project migration discipline

Schema evolution reached v5 before golden project files were established. The
repository now checks in v1-v5 fixtures and proves migration, current-schema
save/reload and typed-graph compilation. Keep that fixture chain mandatory for
every future schema revision.

### External integration and licensing

- Ableton Link fits the tempo model, but `rusty_link` is GPL-2.0+ unless a
  proprietary Ableton license is obtained. Resolve distribution policy first.
- NDI requires its SDK and redistribution terms. Keep any integration in an
  optional crate/feature with a build that remains functional without the SDK.
- Syphon/Spout requires platform-specific native texture interop and should not
  be estimated as a thin Rust dependency addition.
- `wgpu` and `naga` must move together with the `egui-wgpu` compatibility line;
  do not introduce two incompatible `wgpu` versions.

## Recommended order

```text
repeatable release certification and golden project fixtures
    -> behavior-preserving app/UI module seams
    -> GPU/upload/decode timing diagnostics
    -> signed macOS release and FFmpeg licensing decision
    -> tempo sync (after Ableton Link licensing decision)
    -> feature-gated NDI output
    -> projection mapping and additional stage I/O
```

This ordering protects the working show path while turning the next external
integrations into optional, testable additions.
