# VIRTUAL

Four-deck live-performance video mixer with GPU-native HAP playback and
FFmpeg fallback, linear-light composition, deterministic show recovery and a
validated WGSL custom effect-package runtime.

Repository: <https://github.com/deastrobooking/VIRTUAL>

Licensed under [GPL-2.0-or-later](LICENSE). Ableton Link tempo/phase
synchronization is available under MIDI → Clock sync → Ableton Link.
See [Link setup and validation](docs/ABLETON_LINK.md).

For the local macOS event build, run `sh scripts/build-macos.sh` and open
`target/release/VIRTUAL.app`. See [event setup](docs/EVENT_SETUP.md) for projector,
sound-reactive modulation, tap tempo and HDMI capture. This bundle uses the
FFmpeg libraries installed on the build Mac; it is not a portable distribution.

## At a glance

| Path | What VIRTUAL provides |
|---|---|
| Media | Direct block-compressed HAP playback, conventional FFmpeg decode, stills and low-latency cameras |
| Performance | Four decks, 32 clip slots, eight scenes, A/B buses, 35 blend modes, MIDI (including beat-clock sync in and out), OSC and audio/beat modulation |
| Output | One offscreen program shared by the operator preview and a clean display-selectable output window |
| Safety | Bounded workers and queues, generation-safe media, last-known-good shaders, atomic projects and crash-recoverable takes |
| Extensibility | Typed graph contracts plus manifest-driven one/two-pass WGSL packages in two master slots |

The application is functional and extensively GPU-tested. Release work is
focused on physical show-machine certification, distribution, deeper timing
diagnostics and the staged [shader-system upgrade](docs/SHADER_SYSTEM.md).

## Implemented

The current source tree includes:

- wgpu + winit + egui render loop with explicit linear/sRGB handling.
- Vidvox's reference HAP decoder behind a bounded safe Rust API.
- BC1, BC3, BC4, BC6H and BC7 plane models, including HAP Q and HAP Q Alpha.
- Direct block-compressed upload and sampling; HAP is never expanded to CPU
  RGBA.
- MOV probing and raw HAP packet reads through libavformat, with exact rational
  timestamps.
- A bounded generation-safe frame scheduler with hold, drop, late, repeat and
  invalidation accounting.
- Four independently assignable decks with non-blocking drag-and-drop movie
  probing and stale-import rejection.
- Import health ratings for HAP, ProRes, DNxHD/DNxHR, H.264, H.265 and other
  FFmpeg-decodable movie formats.
- Bounded per-deck decoder workers, timestamp scheduling and FFmpeg/libswscale
  fallback conversion to RGBA.
- Four-source linear-light GPU composition with per-deck levels, master
  opacity and next-frame blackout.
- One offscreen program render shared by the operator preview and a clean
  second output window, with connected-display selection, output
  enable/fullscreen, 720p/1080p/UHD presets, custom composition sizing, test
  card, identification overlay and live surface-health diagnostics.
- Per-deck play/pause, restart, freeze, loop/one-shot, 0.25–4× playback and
  asynchronous generation-safe seeking.
- Independent A/B bus composites with assignable decks and linear or
  equal-power crossfading between the completed bus images.
- Per-deck position, uniform scale, rotation and independent horizontal or
  vertical flip, four-edge crop, and Fit/Fill/Stretch source modes, with
  transparent pixels outside transformed layer bounds.
- Thirty-five alpha-correct per-deck blend modes inside each bus, grouped as
  Standard, Contrast, Component and VIRTUAL in the picker. The separable and
  non-separable modes follow W3C Compositing and Blending Level 1, so Color
  Dodge, Soft Light, Vivid Light, Hue, Color, Luminosity and the rest match
  what a compositing tool does; every mode has a hand-derived GPU readback
  test.
- Nine signature blend modes with no Photoshop equivalent: Negation, Invert,
  Reflect, Glow, Phoenix, Hue Shift, Fractal Fold, Xor Crush and Solarize.
  Fractal Fold folds channels through a layer-driven triangle wave, Xor Crush
  quantises both layers to five bits and exclusive-ors them, and Hue Shift
  rotates the backdrop by the layer's own hue angle.
- Per-deck bloom gathered inside the composite pass: soft-knee bright pass,
  16-tap golden-angle disc, resolution-independent radius, and a chromatic
  spread that carries red further than blue. Decks at zero level, bypassed or
  excluded by solo skip the gather entirely, so cost tracks decks actually on
  screen. Measured at 1080p on an M3 Pro: 2.78 ms/frame for four decks
  without bloom, 6.90 ms with bloom on all four.
- Per-deck Solo isolation and non-destructive layer Bypass, including
  multi-solo operation across both buses.
- Native audio-input discovery and capture through a bounded allocation-free
  callback queue, with worker-thread FFT analysis for RMS, bass, mid, high and
  transient signals.
- Audio RMS/bands/transient sources in every deck's modulation matrix, with
  live meters, gain/noise-floor/envelope controls and project persistence.
- Optional adaptive RMS normalization plus beat-phase and four-beat bar-phase
  modulation sources.
- Native per-deck mirror, neon glow, fractal fold, scanline jitter, find-edges,
  bit reduction, black-light inversion, pixelate and luma-key effects.
- Three built-in deck-effect groups with independent bypass and dry/wet,
  legacy-compatible project defaults and Neutral, Neon Night, Blacklight,
  Glitch and Halation presets. Geometry remains the UV prepass; Color and
  Stylize follow their relative displayed order.
- A scrollable, stage-oriented operator UI with live program/audio/MIDI/OSC
  status, top-level blackout/freeze controls, deck accents and explicit
  selected/queued/active clip states.
- An always-visible rig preflight rail plus a transient Show Mode that protects
  setup, media management and structural editors while keeping performance
  controls, compact master-effect identity/bypass/wet cards and deck effects
  live.
- Two reorderable master-effect slots with persisted bypass/dry-wet controls
  plus separable Gaussian blur and persistent feedback/trails backed by fixed
  ping-pong and history textures.
- A versioned master-effect package manifest with validated parameter schemas,
  background WGSL/pipeline reload and atomic last-known-good fallback when a
  candidate is invalid.
- Registry-discovered custom master effects with schema-generated sliders,
  named parameter persistence and neutral pass-through when a saved package is
  unavailable. The bundled Chromatic Split package is the reference example.
- Analog CRT, Thermal Contours and Gravitational Lens packages for deck or
  master placement, plus a two-pass Anamorphic Flare master effect, each with
  three named looks.
- Kaleidoscope, Mirror Symmetry and Mirror Mosaic for deck or master placement,
  with nine presets, movable centers, rotation, zoom and animated spin/drift.
- Bundled dual-placement algorithmic effects for recursive 2D transforms, volumetric
  3D fractal fields and projected 4D–6D recursion, with grouped controls and
  three one-click looks per package. They execute in a master slot or in the
  stateless package slot on any deck.
- Launch-directory-independent effect discovery from development, adjacent
  release and macOS bundle resources, plus the per-user effect directory and
  optional `VIRTUAL_EFFECT_PATH` roots.
- Three master LFOs and eight stable-ID custom-parameter routes with audio,
  beat and bar sources, plus generated per-parameter MIDI Learn/Clear controls.
- Declarative one- or two-pass custom package sequences compiled and installed
  atomically, reusing the fixed master scratch/ping resources. Spectral Echo is
  the bundled two-pass reference.
- An optional fixed previous-slot-output history resource for custom packages,
  with validity signaling and deterministic lifecycle resets. Temporal Melt is
  the bundled temporal reference.
- Per-deck hue, contrast, saturation, black/white levels and gamma grading,
  plus three assignable LFO lanes with sine, triangle, saw-up, saw-down and
  square waves.
- Eight-route per-deck modulation matrix: route any LFO to multiple FX
  destinations with bipolar amounts, optional inversion and optional direct
  assignments.
- Free-running or tempo-synchronized LFO rates from 1/16 beat through eight
  beats, plus manual BPM, Tap, half-time and double-time controls.
- Reused RGBA and HAP GPU texture allocations for stable-resolution clips.
- Per-deck reusable CPU RGBA frame leases with bounded non-blocking returns;
  allocation, reuse, live-lease and discarded-return telemetry is visible
  during performance.
- Bounded per-clip conventional-codec keyframe indexes; trims, resume,
  restarts, loops and playhead seeks reopen at the nearest preceding keyframe
  and decode forward to the exact generation-tagged target.
- Native multi-device MIDI input discovery/capture with bounded callback
  delivery, per-project controller reconnection, live per-device diagnostics,
  click-to-map overlays and an assignment manager with learn, cancel, clear
  and mapping removal.
- Absolute, momentary, toggle, binary-offset and two's-complement mappings,
  editable ranges, inversion and soft takeover for mixer, transport, clip,
  scene, effect, LFO, matrix and emergency targets.
- MIDI beat-clock sync in both directions: a followed 24 PPQN clock drives
  tempo and re-anchors beat phase every quarter note, honouring Start,
  Continue, Stop and Song Position; a dedicated sender thread clocks
  downstream gear from its own schedule with late-pulse, resync and error
  telemetry. Both ends are saved with the project. See
  [docs/MIDI_SYNC.md](docs/MIDI_SYNC.md).
- Four rows of eight persistent clip slots with per-slot health metadata,
  playing/queued indicators and right-click clearing.
- Recursive folder drop/import with supported-media filtering, deterministic
  lexical ordering, selected-slot wraparound, occupied-slot skipping and a
  hard 32-slot assignment bound.
- Per-slot In/Out trim, restart-or-resume launch policy and optional
  BPM-relative beat duration, shared by seek, loop and one-shot boundaries.
- Eight scene launch buttons and `1`–`8` shortcuts that trigger the same slot
  across all four decks.
- Internal 20–400 BPM clock with immediate, next-beat and next-bar launch
  quantization, preserving musical phase across tempo changes.
- Aggregate dropped, repeated and late-frame monitoring in the operator UI.
- Versioned `.virtual` JSON projects containing all 32 media paths, active and
  selected clips, per-clip playback settings, mixer/transport/effect state,
  tempo settings and MIDI maps.
- Atomic Save/Save As-style path workflow, `Cmd/Ctrl+S`, bounded background
  project writes and five-second autosave, close-time recovery snapshots and explicit crash-recovery loading.
- Asynchronous 32-slot project restoration with project-epoch rejection;
  missing media remains visible with its original path and can be relinked
  through a native per-slot file browser without losing clip settings.
- Playback-independent thumbnail worker with a bounded request queue, fixed
  160×90 maximum UI output and at most one cached texture per clip slot.
- A retained 640×360 first-frame launch preview for every successfully probed
  slot; immediate GPU upload prevents blank output while the full decoder
  starts, with a fixed 29.5 MB worst-case cache across all 32 slots.
- Thumbnail previews for HAP and FFmpeg media, with stale-result rejection and
  diagnostic text-tile fallback when preview decoding fails.
- PNG and JPEG still-image import through FFmpeg, decoded once and held without
  continuous decoder load.
- Native macOS camera/capture-card discovery and manual device-ID entry, with any supported video input
  connectable to any of the four decks at a requested resolution/frame rate.
- Bounded low-latency live decoding that drops stale capture frames under
  render backpressure instead of accumulating camera delay.
- Cancellable camera reads with cooperative open/read deadlines, plus bounded
  camera-to-clip recording that preserves capture timing across dropped frames.
  Raw RGBA recording storage estimates are visible beside the record controls.
- Camera-aware deck controls and project persistence; saved live inputs
  reconnect when a project is restored.
- A typed, device-neutral audiovisual `ProjectGraph` with versioned contracts,
  explicit rate domains and feedback boundaries, deterministic pass ordering,
  hard GPU/memory estimates and immutable compiled plans.
- An 11-node compatibility graph for the current four-deck pipeline plus
  isolated shadow transactions with frame/beat/bar/timecode commits and
  last-known-good retention.
- Renderer lowering that validates the graph topology, fuses the four
  source/effect branches into the proven compositor, then drives master
  effects and program presentation from the immutable schedule.
- Serializable show commands, session checkpoints, deterministic replay,
  branching and named performance takes. Clip/scene launches, tempo changes
  and output-enable changes now enter the live event log.
- A bounded background session-journal writer with versioned JSONL records,
  atomic 600-frame checkpoints, torn-tail recovery and live health reporting;
  disk I/O never runs on the render thread.
- A record-before-apply control gateway shared by MIDI, keyboard and continuous
  UI performance values, preserving operator/device origin while mapping clip,
  transport, tempo, blackout and output operations to typed replay commands.
- Deterministic structural field capture for transforms/crop/source/blend,
  effect-slot and LFO/modulation routing, master effects/modulation, and
  successful movie, folder and relink assignments.
- An operator session-recovery browser that validates prior journals, restores
  checkpoint-plus-tail state and continues safely in a fresh baseline journal.
- Version-six projects persist stable project/take identities, bounded take
  metadata, deterministic seed maps and the active typed graph. Operators can
  start named takes or restore a prior session as a named branch.
- Full journal history can be scrubbed to an earlier checkpoint-aware position;
  take metadata can be renamed or unlinked without deleting its journal file.
- Labeled timeline markers jump the recovery cursor to exact show times, and
  take journals/checkpoints can be exported or archived into unique folders
  without changing the source session bundle.
- Bounded OSC 1.0 UDP input accepts mixer, deck, clip, scene, tempo and output
  routes through the authoritative command gateway, with sender origins and
  malformed/drop telemetry retained for show diagnostics.
- OSC feedback publishes an initial state snapshot and accepted changes from a
  separate bounded worker, while bundle NTP timetags execute through a bounded
  monotonic scheduler without stalling rendering.

## Shader system direction

VIRTUAL uses WGSL compiled through Naga and `wgpu`. `master-v1` provides one or
two fragment passes and optional fixed history per master slot. Stateless,
one-pass `deck-v1` packages run after a deck's built-ins and before its layer
blend; dual-placement packages can run in either runtime. Watched changes compile away
from presentation and swap only after the complete candidate succeeds. Manual
registry refresh also runs on a bounded latest-wins worker and discards stale
scan generations.

Per-deck packages now execute through a selectively materialized branch while
the no-package path stays fused. Each deck exposes package selection, generated
parameters, looks, bypass and wet controls, with project-v6 persistence,
last-known-good reload, eight stable modulation routes, stable MIDI/OSC
destinations and per-deck GPU pass timing. Show-machine certification remains. Shared WGSL modules, typed
N-pass graphs, optional HDR intermediates, compute effects and offline
ISF/ShaderToy conversion follow as separately budgeted phases.

See [docs/SHADER_SYSTEM.md](docs/SHADER_SYSTEM.md) for the contracts,
acceptance gates and decisions from the shader review. Arbitrary package-owned
resources, temporal deck effects and multi-pass deck packages have not landed.

## Quick start

```sh
cargo run          # the app; select a deck and drop movies
cargo run -- a.mp4 b.mov c.mkv d.webm  # preload decks A-D
cargo run -- show.virtual              # open and restore a project
VIRTUAL_EFFECT_PATH=/path/to/effects cargo run  # add package roots
cargo test         # includes a headless GPU readback test
cargo run -p virtual-media --example probe_movie -- footage.mp4
cargo run -p virtual-render --example dump_frame > frame.raw \
  && ffmpeg -f rawvideo -pix_fmt rgba -s 512x512 -i frame.raw -y frame.png
```

Safety/performance keys: `B` toggles blackout, `Space` toggles master freeze,
`O` toggles program output, arrow keys move the crossfader, `Home` centers it,
`1`–`8` launch scenes, and `Delete`/`Backspace` removes the selected clip when
Show Mode is off and no text field owns the key. `Cmd/Ctrl+S` saves to the
project path shown in the operator window.

Release candidates should follow the automated, fixture, hardware-failure and
packaging gates in [docs/RELEASE_CHECKLIST.md](docs/RELEASE_CHECKLIST.md).

The **Program output** toolbar control shows or hides the clean output window.
Choose the connected display, then enable **Fullscreen**. Use **Test card** to
calibrate the signal path and **Identify** for a magenta frame/crosshair.
Expand **Output health** to inspect the active display, surface/composition
sizes, skipped presentations, reconfigurations and recoveries. `Escape` returns
the output to windowed mode.

In the app, click deck A, B, C or D and drag a movie onto the window. Imports
are probed on a background worker and the next deck is selected automatically.

For a live input, select a deck and use **Video input · camera / capture card**.
Connect the device, click **Refresh**, select its name, choose the capture
resolution/frame rate, then click **Connect to Deck**. Fractional rates such as
59.94 fps are supported. Start with **Automatic** pixel format; UYVY/YUYV 4:2:2
and NV12 are available when the device requires a specific format.

This path supports capture hardware exposed to macOS as AVFoundation video
inputs. Cards requiring a vendor-only SDK are not integrated. Match the incoming
HDMI/SDI signal and the card's supported mode; there is no universal connector
switch. Audio is selected separately. Grant macOS camera access to VIRTUAL (or
Terminal during development). Use **Manual input / custom size** for a device
name/index or a non-preset resolution. See [video inputs](docs/VIDEO_INPUTS.md)
for setup and troubleshooting.

## Documentation

- [Release notes](docs/RELEASE_NOTES.md)
- [Operator guide](docs/OPERATOR_GUIDE.md)
- [MIDI beat-clock sync](docs/MIDI_SYNC.md)
- [Feature status](docs/FEATURES.md)
- [Application review](docs/REVIEW.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Shader system and upgrade plan](docs/SHADER_SYSTEM.md)
- [Effect package authoring](docs/EFFECT_PACKAGES.md)
- [Effects review and new looks](docs/EFFECTS_REVIEW.md)
- [Graph and session runtime](docs/GRAPH_RUNTIME.md)
- [Prioritized roadmap](docs/ROADMAP.md)

## Layout

| Crate | Owns |
|---|---|
| `virtual-core` | Clock, parameters, modulation, scene graph. No GPU, no I/O — testable headless. |
| `virtual-graph` | Typed node contracts, validation, plan compilation, transient resource scheduling and graph transactions. |
| `virtual-hap-sys` | Pinned Vidvox HAP C reference source and raw bindings. |
| `virtual-hap` | Bounded safe HAP decode to GPU-native BC planes. |
| `virtual-media` | Demux, codec dispatch, frame queues and scheduling. |
| `virtual-render` | wgpu device, surface, render passes. Knows nothing about winit or egui. |
| `virtual-io` | Versioned project persistence plus bounded native MIDI and audio input adapters. |
| `virtual-session` | Event-sourced commands, state checkpoints, replay, branches and performance takes. |
| `virtual-app` | Windowing, UI, wiring. |

## Decisions already locked in

**wgpu is pinned to 29** because `egui-wgpu` 0.35 depends on 29.0. Bumping wgpu
alone puts two incompatible copies in the tree; bump them together.

**Colour space.** The swapchain is configured in gamma space with the sRGB
variant listed as a view format. Content renders through the `*Srgb` view — so
shaders work in linear and the hardware encodes on write — and the egui overlay
renders through the plain gamma-space view, which is what it wants. There is no
manual `pow(2.2)` anywhere and there should never be one.

**Presentation is Fifo (vsync).** Clip playback already uses exact media time
and generation-safe scheduling; frame counters are not a playback clock when
clips run at 24/25/30fps.

## Not decided yet

- Whether ffmpeg is vendored and statically linked, and the LGPL consequences
  for distribution.
