# Oneiroi architecture

## Runtime shape

The main thread owns `winit`, `egui`, the `wgpu` device, GPU textures,
composition and presentation. Blocking file, probe, thumbnail and decode work
runs away from the render loop.

```text
files / cameras
      |
      v
probe + decoder workers ---- generation-tagged frames
      |                                  |
      v                                  v
clip metadata                    bounded frame schedulers
      |                                  |
      +------------- main thread --------+
                         |
                         v
               GPU upload / texture reuse
                         |
                         v
              per-deck effects + modulation
                         |
                         v
              linear-light four-deck mixer
                         |
                         v
              offscreen program texture
                    /             \
                   v               v
          operator preview      clean output
              + egui               surface
```

## Workspace responsibilities

| Crate | Responsibility |
|---|---|
| `oneiroi-core` | Exact media time, frame clock, tempo/tap tempo and device-neutral control mapping |
| `oneiroi-hap-sys` | Pinned Vidvox HAP reference implementation and raw FFI |
| `oneiroi-hap` | Validated safe HAP decode into BC-compressed planes |
| `oneiroi-media` | Probe, demux, decode workers, clip bank, transport, scheduling, thumbnails and cameras |
| `oneiroi-render` | GPU resources, HAP/RGBA upload, effects, LFO resolution and four-deck composition |
| `oneiroi-io` | Versioned project JSON, atomic save and recovery paths |
| `oneiroi-app` | Window/event loop, UI and orchestration |

## Media paths

HAP media follows:

```text
MOV -> libavformat packet -> Vidvox HAP decode -> BC texture blocks
    -> direct wgpu compressed texture upload
```

HAP is not expanded to CPU RGBA. HAP Q is converted from scaled YCoCg while
sampling, and HAP Q Alpha combines its BC3 color and BC4 alpha planes.

Conventional video and stills follow:

```text
container -> FFmpeg codec -> libswscale RGBA -> reusable wgpu RGBA texture
```

Camera sources use an explicit FFmpeg input-device backend. macOS currently
uses AVFoundation. Camera workers discard stale frames when their bounded
output queue is full.

## Timing and stale-work rejection

Media timestamps use exact rational `MediaTime`; floating-point frame counters
are never the playback clock. Each deck has a generation number. Replacement,
seek, restart and eject invalidate the previous generation. Workers and
schedulers carry the generation on every frame, preventing a late frame from a
previous clip from flashing after a transition.

The scheduler holds the most recent valid frame on underrun and drains stale
frames to the newest eligible timestamp. Decoder and render queues are bounded.

## Effects and modulation

The compositor receives neutral base `DeckEffects` values from UI/project
state. Each frame, three per-deck LFO sources and eight modulation routes
resolve a temporary copy:

```text
resolved = clamp(base + direct LFOs + summed matrix routes)
```

The render thread receives only resolved values. Base knobs are not overwritten
by modulation. Routes are bipolar and may be matrix-only or combined with an
LFO's direct destination.

Free-running LFOs derive phase from elapsed seconds. Synchronized LFOs derive
phase from the internal clock's beat position, so BPM changes retain musical
phase.

## Color and composition

Each deck is processed and composited in deck order into its assigned Bus A or
Bus B accumulator. Only after both accumulators are complete are the selected
linear or equal-power gains applied. This prevents a layer on one bus from
changing the internal result of the other bus.

Layer geometry uses an inverse UV transform before source effects: output
position is translated back into deck-local coordinates, inverse-rotated,
scaled and optionally flipped. Coordinates outside `[0, 1]` resolve to
transparent black. Because this happens before source interpretation, RGBA,
HAP, still images and camera frames share identical transform behavior.

The compositor renders once into a fixed-resolution sRGB program texture.
Presentation passes sample that texture into the operator and output surfaces.
Their small uniform also selects calibration modes: a generated color-bar/grid
test card and a magenta identification frame/crosshair. Display discovery and
window placement remain in `oneiroi-app`; the render crate has no winit types.
Surface acquisition returns an explicit health status alongside the optional
frame. Lost, outdated and suboptimal surfaces reconfigure automatically; the
application records skips, timeouts, occlusion, validation failures and the
next healthy recovery. Display topology is polled on a two-second cadence.
The content views encode linear shader output to sRGB; the egui overlay then
uses the operator surface's non-sRGB view, matching egui's output convention.
Operator-window resizing does not change composition resolution.

## Persistence

`.oneiroi` files are versioned JSON. The current schema is version 2 and
version-one files are migrated on load. Saves write a temporary sibling and rename
it atomically. Newly introduced fields use explicit Serde defaults so existing
projects remain readable. Autosave/recovery state is intentionally
separate from the user's saved project.

## Non-negotiable invariants

- The main/render thread never waits for media decode or disk I/O.
- Queues that can receive media frames are bounded.
- Obsolete generations never reach presentation.
- HAP playback never takes the FFmpeg pixel-decoding path.
- Base parameters remain distinct from resolved modulation values.
- A camera backlog is dropped rather than converted into latency.
- Invalid project values are rejected before application.
