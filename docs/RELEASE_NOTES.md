# Release notes

## Unreleased

### Operator interface

- Added five project-persisted palette presets, an optional accent override,
  compact/cozy/roomy control density and automatic, grid, cascade or stacked
  deck layouts.
- Added theme-aware semantic colors and contrast-safe selected, warning and
  MIDI-map states across dark and daylight palettes.
- Expanded the default operator workspace and added vertical scrolling so the
  full mixer, device, diagnostics and effect surfaces remain reachable at
  smaller window sizes.
- Added a live status rail for program output, audio, MIDI and OSC.
- Promoted master freeze and emergency blackout to the top-level show-control
  bar while retaining the detailed master controls.
- Added distinct deck accents and clearer selected-deck borders.
- Improved the clip grid with larger scene launch controls and separate visual
  states for selected, queued and actively playing clips.
- Added drag-to-move clip slots with a visible grab cursor and safe swap
  behavior for occupied destinations. Media settings and cached previews move
  with the clip, while in-flight imports are protected from stale results.
- Added a transient **Show Mode** performance lock. It keeps clip/scene launch,
  transport, levels, solo/bypass, per-deck effect sliders, crossfader, freeze
  and blackout live while locking setup, imports, media management and
  structural effect edits.
- Added compact live master-effect cards to Show Mode, keeping custom
  algorithmic package names plus bypass and wet controls visible while
  selection, ordering and advanced parameters remain locked.
- Added an always-visible preflight rail for output health, missing/loading
  media, waiting MIDI hardware, unsaved project state and rejected effect
  reloads.
- Reworked the main hierarchy around one always-visible selected-deck
  performance editor with FX expanded. Setup now starts closed, clip-grid deck
  labels select the editor directly, and the other full deck editors live in
  an optional secondary section instead of pushing controls off-screen.
- Added a visible selected-clip delete button plus `Delete`/`Backspace`
  shortcuts, backed by one journaled cleanup path for pending imports, queued
  launches and preview caches. Show Mode protects all three entry points.
- Added an always-visible Deck input strip to the clip view for choosing and
  switching a selected deck to a camera. Live camera frames can now be recorded
  into the selected empty clip slot with non-blocking Start/Stop controls,
  bounded frame delivery and asynchronous finalization/import.
- Fixed deck selection becoming direction-dependent during UI command capture.
  Switching from a later deck back to an earlier deck no longer restores the
  previous one-hot selection while processing its falling edge.
- Fixed active clip deletion leaving the deck decoder and last compositor
  texture alive. Deleting the playing slot now invalidates the complete deck
  source, and the selected FX editor warns when master freeze is intentionally
  holding program output against visible control changes.
- Isolated transient UI intent in a dedicated action dispatcher and grouped the
  program window, monitor inventory, surface, presenter and recovery counters
  under one output lifecycle owner. This is a behavior-preserving architecture
  change that reduces render-loop coupling ahead of the diagnostics split.
- Completed the operator UI decomposition with focused toolbar/preflight,
  pre-show setup and output/frame-pipeline diagnostics modules. Show Mode and
  the existing `UiAction` boundary continue to govern the same controls.

### Mixer and control integration

- Added 27 blend modes alongside the original eight modes, including component
  and Oneiroi signature families.
- Restored launch-independent discovery for bundled algorithmic effect
  packages, with additional executable-adjacent, macOS bundle, per-user and
  `ONEIROI_EFFECT_PATH` roots.
- Kept each custom effect's last-known-good GPU pipeline active while registry
  refresh recompiles it; a refresh no longer creates a temporary pass-through
  frame.
- Retired stale selectable or processor pipelines when a valid manifest moves
  to another role or target, while malformed edits still retain the previous
  working generation. Reload diagnostics now distinguish retained
  last-known-good output, the restored built-in processor and a neutral
  fallback.
- Added backward-compatible manifest-v2 target and ABI metadata plus one
  stateless `deck-v1` slot per deck. Selected deck branches materialize after
  built-ins, execute on target-validated GPU pipelines, then re-enter the layer
  blend while decks without packages retain the fused fast path.
- Added per-deck package selection, generated controls, looks, wet/bypass,
  last-known-good reload diagnostics and project-v6 persistence. Chromatic
  Split and all three algorithmic packages now support deck/master placement.
- Required manifest v2 to declare its ABI explicitly, and preserved exact
  resource-root precedence when a higher-priority package becomes temporarily
  invalid instead of promoting a lower-priority duplicate.
- Added per-deck bloom, threshold, radius and chromatic-spread controls plus the
  Halation preset.
- Extended MIDI learn, feedback snapshots and project validation to all 18
  built-in deck effect parameters.
- Added simultaneous multi-controller input, an Ableton-style click-to-map
  overlay and a dedicated MIDI Manager for device status and assignment
  auditing.
- Persisted the requested controller rig per project. Missing controllers stay
  visible, retry every two seconds and can be forgotten explicitly; opening a
  different project disconnects controllers that do not belong to its rig.
- Centralized the persisted deck-effect target count and added boundary tests
  to prevent future control-surface drift.

### Validation

- Added real-GPU coverage proving a bundled package executes on a deck and
  package bypass returns byte-for-byte to the fused compositor output.

- Added a checked-in, legacy-shaped project-v1 fixture that verifies migration
  to v5, preservation of v1 values, safe defaults for later fields and an
  atomic current-schema save/reload.
- Added a project-v2 fixture covering dedicated output settings, audio
  analysis, built-in master effects, clip playback, transforms and deck effect
  slots while proving v3-v5 fields migrate to safe defaults.
- Added a project-v3 fixture covering custom master-effect package identity,
  named parameters, stable-key modulation and MIDI targets while proving v4-v5
  identity, take, graph and seed fields migrate safely.
- Added a project-v4 fixture covering stable project/take identities and linked
  journal metadata while proving v5 graph injection and deterministic-seed
  defaults preserve the historical project.
- Added a native project-v5 fixture with stable identities, deterministic seeds
  and the explicit 11-node compatibility graph. It loads without mutation,
  compiles through the built-in graph registry and round-trips atomically.
- Mixer shader validation and GPU readback cover every blend-mode identity and
  bloom falloff behavior.
- Real-GPU tests now load and render Recursive 2D Lab, Fractal Volume 3D and
  Hyper Recursion 4D+, while layer tests cover luma-key reveal and verify that
  upper-layer effects execute before non-Normal compositing.
- The workspace passes formatting, full tests and strict Clippy; the extended
  10,000-reopen decoder soak remains an explicit pre-show/release-candidate
  check.

### Shader architecture and documentation

- Added a canonical shader-system plan covering the per-deck precomposition
  seam, `deck-v1`, shared WGSL modules, typed N-pass resources, optional HDR,
  compute and offline ISF/ShaderToy import in dependency order.
- Reconciled the README, architecture, package authoring, feature matrix,
  graph runtime, operator guide, engineering review, roadmap and release gates
  around the implemented deck-v1 executor and its remaining control gates.
- Added an optional VS Code recommendation for `wgsl-analyzer`; Naga and
  real-GPU tests remain the authoritative validation paths.

### Release-candidate follow-up

- Added `docs/RELEASE_CHECKLIST.md` as the repeatable automated, media,
  output/device failure and packaging gate for candidate builds.
- Run the ignored extended decoder reopen soak on the target show machine.
- Perform a hardware smoke test for display reconnect, audio permissions,
  MIDI reconnect/feedback and sustained 1080p output.
- Confirm packaging, signing, notarization and FFmpeg distribution decisions
  before assigning a release version.
