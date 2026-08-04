# Feature status

This matrix reflects the current source tree, not the aspirational MVP notes.

## Working

| Area | Current implementation |
|---|---|
| Decks and clips | Four decks, eight persistent slots per deck and eight scene launches |
| Import | File/folder drag/drop, bounded recursive scanning, deterministic slot assignment, probing, thumbnails, first-frame launch previews, movie metadata and PNG/JPEG stills |
| Codecs | Direct HAP family path plus FFmpeg fallback for conventional codecs |
| Cameras | AVFoundation discovery/manual ID, requested size/FPS and any-deck assignment |
| Playback | Play, pause, restart, freeze, seek, loop/one-shot, 0.25–4× speed and per-slot In/Out ranges |
| Timing | Exact timestamps, bounded keyframe indexes, indexed conventional-codec reopen, bounded schedulers and generation-safe stale-frame rejection |
| Triggering | Immediate, next-beat and next-bar clip/scene launches, per-slot restart/resume and BPM-relative beat duration |
| Mixing | Independent A/B composites, 35 blend modes, Solo/Bypass, transforms, crop/source modes, linear/equal-power crossfade, master opacity and blackout |
| Output | Offscreen preset/custom program target, clean second window, display selection, aspect preservation, enable/fullscreen, calibration overlays and surface-health diagnostics |
| Effects | Three reorderable deck groups plus two master slots; original fractal fold, recursive 2D, volumetric 3D and 4D–6D projections, grouped package controls and one-click looks, color/levels, mirror, neon, jitter, edges, bit reduction, black light, pixelate, luma key, separable blur and persistent feedback/trails |
| Modulation | Three LFOs and eight bipolar routes per deck across 18 continuous effect destinations |
| OSC | Bounded OSC 1.0 UDP input/output, nested bundles, NTP-timetag scheduling, initial state snapshots, live health counters and origin-aware routes for mixer, decks, clips, scenes, tempo and output |
| Musical control | Manual BPM, Tap, half/double, beat/bar phase and synchronized LFO divisions |
| Audio modulation | Native input capture, bounded queue, RMS/FFT bands, transient, adaptive normalization, live meters and five audio plus beat/bar matrix sources |
| Persistence | Atomic save, autosave, recovery, asynchronous restore, automatic v1–v4-to-v5 loading, stable project/take identity, deterministic seeds, active graph metadata and missing-media relinking |
| Operator safety | Selected-deck primary editor, direct deck-row targeting, Show Mode performance lock, preflight rail and button/keyboard clip deletion |
| Diagnostics | FPS, decoder drop/repeat/late counters, RGBA allocation/reuse/live/discard telemetry, output surface state, presentation skips/recovery and display-topology changes |

## Partial foundations

| Area | Present | Still required |
|---|---|---|
| MIDI | Native device discovery/input, reconnect, learn UI, activity/drop diagnostics, absolute/momentary/toggle/relative modes, editable ranges, soft takeover, broad target routing and persistence | Output feedback, MIDI clock and physical-controller soak validation |
| Audio analysis | Gain, noise floor, attack/release, normalization, band and transient analysis | Show-device disconnect/soak validation |
| Effects system | Three persisted deck slots, two persisted master slots, common bypass/dry-wet/reset, five factory deck presets, bounded ping-pong blur, reset-safe feedback history, registry-discovered custom master effects, named persistence, last-known-good hot reload, three master LFOs, eight custom-parameter routes, generated MIDI learn, atomic one/two-pass graphs and optional fixed per-slot custom history | Arbitrary package-owned texture declarations |
| Output routing | Shared operator/output presentation, connected-display selection, persisted descriptor, topology polling and surface recovery diagnostics | Stronger identity across display topology changes and show-machine soak testing |
| Performance | Bounded workers, reusable CPU RGBA frame leases, 29.5 MB maximum first-frame cache, capped keyframe indexes, deterministic decoder faults and accelerated/extended soak coverage | Physical-media and show-machine soak certification |
| Typed graph runtime | Versioned typed node contracts, six rate domains, validation, explicit feedback rules, deterministic scheduling, immutable plans, resource lifetime reuse and GPU/memory budgets; the 11-node four-deck graph lowers to authoritative fused-composite, master-effect and output stages | Add executors beyond the compatibility graph, complete color/resolution inference and independently execute nodes that cannot be fused |
| Live transactions | Isolated shadow graphs, all-or-nothing preparation, last-known-good retention and frame/beat/bar/timecode commit scheduling | Graph editor/preview UI, prewarming and live operator commit controls |
| Performance replay | Serializable show commands, session state, checkpoints, deterministic replay and versioned JSONL journals; operators can start named takes, add labeled timeline markers, scrub full journal history into named branches, safely manage take metadata, export/archive unique bundle copies and edit scoped deterministic seeds | Add marker editing plus portable project/media manifests |

## Not implemented

- Recent-project list and graphical Save As browser
- OSC route expansion/discovery, Ableton Link, MIDI clock, NDI, Syphon/Spout and projection mapping
- Application packaging, signing and FFmpeg distribution
- Graph editor, Score view, Spatial view and compiled GPU node execution
