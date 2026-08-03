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

- Mixer shader validation and GPU readback cover every blend-mode identity and
  bloom falloff behavior.
- The workspace passes formatting, full tests and strict Clippy; the extended
  10,000-reopen decoder soak remains an explicit pre-show/release-candidate
  check.

### Release-candidate follow-up

- Run the ignored extended decoder reopen soak on the target show machine.
- Perform a hardware smoke test for display reconnect, audio permissions,
  MIDI reconnect/feedback and sustained 1080p output.
- Confirm packaging, signing, notarization and FFmpeg distribution decisions
  before assigning a release version.
