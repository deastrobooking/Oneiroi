# Application review

Review date: 2026-09-25

Event follow-up: see [the recorded review](EVENT_REVIEW_2026-09-25.md) for the
local build, fixes, automated checks and remaining physical rehearsal gates.

September follow-up: project writes now run on a bounded background worker;
active camera reads support cancellation/deadlines; camera recordings retain
capture timing across dropped frames; and golden fixtures cover v1-v6. These
changes have automated coverage. Physical camera/display/storage failure
certification and macOS packaging remain open. See the reliability entry in
[release notes](RELEASE_NOTES.md) for behavior and limits.

## Executive assessment

VIRTUAL is a functional four-deck VJ instrument with a credible media and
rendering foundation. Direct HAP upload, bounded FFmpeg fallback, independent
A/B composition, deck and master effects, audio/MIDI/OSC control, clean program
output and deterministic session recovery are implemented and covered by more
than 200 automated tests.

The main release risk is no longer missing mixer fundamentals. It is show-machine
certification, application packaging and measured expansion of the shader path.
Per-deck package execution, selection and persistence are now implemented for
one stateless pass per deck. Stable deck-package modulation, MIDI and OSC
identities, alpha/culling diagnostics and non-blocking per-deck GPU pass timing
are implemented; show-machine certification remains. External stage
integrations such as tempo sync and video sharing should follow the release
gates rather than displace them.

## Current strengths

### Media and rendering

- HAP remains block-compressed through GPU upload; conventional media and stills
  use isolated FFmpeg fallback paths.
- Exact media timestamps, bounded workers, reusable frame leases and generation
  checks prevent unbounded backlog and obsolete-frame presentation.
- Four decks feed independent linear-light A/B composites with 35 blend modes,
  transforms, crop, Solo/Bypass and fused built-in deck-effect groups. Geometry
  remains the UV prepass; Color and Stylize can change relative order.
- The program render is shared by an operator preview and clean second window.
  Display targeting, fullscreen, calibration overlays and surface recovery are
  observable rather than implicit.
- Two bounded master slots support built-in and validated one/two-pass WGSL
  packages with generation-safe last-known-good reload and one optional custom
  history texture per physical master slot.

### Performance workflow

- The 4 × 8 clip grid supports scene launches, quantization, folder import,
  missing-media relink, safe slot movement and explicit clip deletion.
- The selected deck has one primary, always-visible editor. Deck row labels and
  clip slots retarget it directly; secondary deck editors no longer bury the
  active controls.
- Show Mode locks setup and destructive/structural edits while retaining clip,
  scene, transport, mixer, Solo/Bypass, live deck-FX controls and compact
  master-effect identity/bypass/wet cards.
- MIDI supports multiple persisted controllers, learn/clear, relative modes,
  soft takeover and reconnect. OSC input/output shares the command gateway and
  supports bounded timetag scheduling.
- Audio RMS, bands and transient analysis plus beat/bar phase feed the deck and
  master modulation systems.

### Persistence and validation

- Project schema v6 migrates supported v1-v5 projects, validates values and
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

The repository now produces a locally ad-hoc-signed macOS application bundle
with privacy usage strings and bundled effects. Portable FFmpeg distribution,
notarization and distribution notices remain open before public release.

### 3. Diagnostics depth

FPS, surface health, decoder drop/repeat/late totals and RGBA lease counters are
visible. Deck-package GPU pass timings are implemented. Upload timing, frame-time
history, queue occupancy and per-deck decode latency are still needed to explain a marginal show machine without attaching a profiler.

### 4. Shader-path budgeting

Algorithmic WGSL packages execute in the two master slots and one stateless
slot per deck. The deck path selectively materializes active layers and has a
fixed 1080p/UHD memory ceiling; transparent-alpha coverage, visible
invisible-branch counters and per-deck pass-time telemetry are implemented.
Target-machine measurement remains a release gate. HDR, arbitrary pass graphs
and compute remain later work.
See [Shader system](SHADER_SYSTEM.md) for the accepted sequence and invariants.

## Engineering risks

### Concentrated application orchestration

`virtual-app/src/main.rs` remains roughly 1,450 lines and still coordinates
windowing, media, projects, cameras, tempo and rendering. Action dispatch,
output lifecycle and the toolbar/setup/diagnostics surfaces have been extracted;
`ui.rs` is now roughly 990 lines with focused `clips`, `deck`, `master_fx`,
`midi`, `midi_manager` and `theme` modules.

Next seams:

- `media_session.rs`: per-deck decoder, scheduler and transport orchestration
- Remaining top-level render-loop coordination after media-session ownership

These are behavior-preserving refactors and should land in small validated
checkpoints.

### Project migration discipline

Schema evolution reached v5 before golden project files were established. The
repository now checks in v1-v6 fixtures and proves migration, current-schema
save/reload and typed-graph compilation. Keep that fixture chain mandatory for
every future schema revision.

### External integration and licensing

- Update: GPL-2.0-or-later is selected and Link tempo/phase integration is
  implemented. Network/hardware certification and transport sync remain;
  see [Link](ABLETON_LINK.md).
- NDI requires its SDK and redistribution terms. Keep any integration in an
  optional crate/feature with a build that remains functional without the SDK.
- Syphon/Spout requires platform-specific native texture interop and should not
  be estimated as a thin Rust dependency addition.
- `wgpu` and `naga` must move together with the `egui-wgpu` compatibility line;
  do not introduce two incompatible `wgpu` versions.

## Recommended order

```text
repeatable release certification and golden project fixtures
    -> shader ABI conformance and GPU timing diagnostics
    -> selective deck-branch extraction and one bounded deck package slot
    -> four-deck show-machine certification of the new path
    -> remaining behavior-preserving media-session seam
    -> signed macOS release and FFmpeg licensing decision
    -> tempo sync (after Ableton Link licensing decision)
    -> feature-gated NDI output
    -> projection mapping and additional stage I/O
```

This ordering protects the working show path while turning the next external
integrations into optional, testable additions.
