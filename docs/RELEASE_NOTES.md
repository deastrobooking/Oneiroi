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
- Fixed deck selection becoming direction-dependent during UI command capture.
  Switching from a later deck back to an earlier deck no longer restores the
  previous one-hot selection while processing its falling edge.
- Fixed active clip deletion leaving the deck decoder and last compositor
  texture alive. Deleting the playing slot now invalidates the complete deck
  source, and the selected FX editor warns when master freeze is intentionally
  holding program output against visible control changes.

### Mixer and control integration

- Added 27 blend modes alongside the original eight modes, including component
  and Oneiroi signature families.
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
- The workspace passes formatting, full tests and strict Clippy; the extended
  10,000-reopen decoder soak remains an explicit pre-show/release-candidate
  check.

### Release-candidate follow-up

- Added `docs/RELEASE_CHECKLIST.md` as the repeatable automated, media,
  output/device failure and packaging gate for candidate builds.
- Run the ignored extended decoder reopen soak on the target show machine.
- Perform a hardware smoke test for display reconnect, audio permissions,
  MIDI reconnect/feedback and sustained 1080p output.
- Confirm packaging, signing, notarization and FFmpeg distribution decisions
  before assigning a release version.
