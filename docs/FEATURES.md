# Feature status

This matrix reflects the current source tree, not the aspirational MVP notes.

## Working

| Area | Current implementation |
|---|---|
| Decks and clips | Four decks, eight persistent slots per deck and eight scene launches |
| Import | Drag/drop probing, thumbnails, movie metadata and PNG/JPEG stills |
| Codecs | Direct HAP family path plus FFmpeg fallback for conventional codecs |
| Cameras | AVFoundation discovery/manual ID, requested size/FPS and any-deck assignment |
| Playback | Play, pause, restart, freeze, seek, loop/one-shot and 0.25–4× speed |
| Timing | Exact timestamps, bounded schedulers and generation-safe stale-frame rejection |
| Triggering | Immediate, next-beat and next-bar clip/scene launches |
| Mixing | Independent A/B composites, eight blend modes, Solo/Bypass, transforms, crop/source modes, linear/equal-power crossfade, master opacity and blackout |
| Output | Offscreen preset/custom program target, clean second window, display selection, aspect preservation, enable/fullscreen, calibration overlays and surface-health diagnostics |
| Effects | Color/levels, mirror, neon, fractal, jitter, edges, bit reduction, black light, pixelate and luma key |
| Modulation | Three LFOs and eight bipolar routes per deck across 14 continuous destinations |
| Musical control | Manual BPM, Tap, half/double, beat/bar phase and synchronized LFO divisions |
| Persistence | Atomic save, autosave, recovery, asynchronous restore and automatic v1-to-v2 loading |
| Diagnostics | FPS, decoder drop/repeat/late counters, output surface state, presentation skips/recovery and display-topology changes |

## Partial foundations

| Area | Present | Still required |
|---|---|---|
| MIDI | Device-neutral learn, mapping modes, soft takeover and persistence | Platform MIDI device adapter and complete UI |
| Missing media | Missing paths remain visible and resavable | File browser and explicit relink workflow |
| Effects system | Native GPU effect chain and parameter model | Manifest packages, reorderable slots, presets and safe hot reload |
| Output routing | Shared operator/output presentation, connected-display selection, persisted descriptor, topology polling and surface recovery diagnostics | Stronger identity across display topology changes and show-machine soak testing |
| Performance | Bounded workers and GPU texture reuse | Reusable CPU frame leases, keyframe index and soak/failure testing |

## Not implemented

- Audio input capture, RMS/FFT bands, transient detection and normalization
- Audio sources in the modulation matrix
- Blur and persistent feedback/trails passes
- Dedicated master-effect slots
- Clip in/out trim and BPM-relative clip duration
- First-frame preload for every ready slot
- Folder import
- Recent-project list and graphical Save As/relink browsers
- OSC, Ableton Link, MIDI clock, NDI, Syphon/Spout and projection mapping
- Application packaging, signing and FFmpeg distribution
