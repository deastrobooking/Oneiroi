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
       fused built-in deck groups + modulation
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

The live pixel path is now described by a typed compatibility graph at
startup. Its immutable plan and event-sourced session run beside the proven
fixed compositor while GPU node lowering is developed:

```text
ProjectGraph -> validate / schedule / budget -> immutable RenderPlan
      |                                             |
      +-> shadow GraphTransaction                   v
                                            renderer lowering
                                                   |
UI/controller -> ShowCommand -> SessionEventLog    v
                                     fused composite / master / output
```

Device-neutral MIDI, keyboard performance controls and continuous UI control
targets enter the same record-before-apply gateway. Command origin is retained
as operator, keyboard, MIDI device, automation, score, OSC or remote identity.
Editor-owned structures cross an adjacent snapshot/diff boundary: the prior
state is restored, stable field commands are recorded, then the accepted state
is applied. Successful asynchronous media probes record clip-slot identity at
the same acceptance boundary.

## Workspace responsibilities

| Crate | Responsibility |
|---|---|
| `oneiroi-core` | Exact media time, frame clock, tempo/tap tempo and device-neutral control mapping |
| `oneiroi-graph` | Typed ports and node contracts, graph validation, immutable plan compilation, resource lifetimes and shadow transactions |
| `oneiroi-hap-sys` | Pinned Vidvox HAP reference implementation and raw FFI |
| `oneiroi-hap` | Validated safe HAP decode into BC-compressed planes |
| `oneiroi-media` | Probe, demux, decode workers, clip bank, transport, scheduling, thumbnails and cameras |
| `oneiroi-render` | Render-plan lowering, GPU resources, HAP/RGBA upload, effects, LFO resolution and four-deck composition |
| `oneiroi-io` | Versioned project JSON, atomic save and recovery paths |
| `oneiroi-session` | Serializable show commands, session state, checkpoints, replay, branches, named takes and bounded crash-safe journal persistence |
| `oneiroi-app` | Window/event loop, UI and orchestration |

The graph and session crates remain device-neutral. See
[Graph and session runtime](GRAPH_RUNTIME.md) for the compatibility boundary
and the next lowering steps. [Shader system](SHADER_SYSTEM.md) defines the
current package boundary and the measured path toward per-deck packages.

## Media paths

HAP media follows:

```text
MOV -> libavformat packet -> Vidvox HAP decode -> BC texture blocks
    -> direct wgpu compressed texture upload
```

HAP playback is not expanded to CPU RGBA. HAP Q is converted from scaled YCoCg
while sampling, and HAP Q Alpha combines its BC3 color and BC4 alpha planes.
The playback-independent thumbnail/preload worker may decode one first frame
to RGBA; it never enters the timed HAP playback path.

Conventional video and stills follow:

```text
container -> FFmpeg codec -> libswscale RGBA -> reusable wgpu RGBA texture
```

Camera sources use an explicit FFmpeg input-device backend. macOS currently
uses AVFoundation. Camera workers discard stale frames when their bounded
output queue is full.

## Clip first-frame readiness

The playback-independent thumbnail worker decodes each ready slot's first
visible frame once. From that decode it produces a 160×90 UI thumbnail and a
640×360-or-smaller launch preview. The UI cache owns both under the clip path
and request ID, so replacement, clear and project-epoch changes reject or
remove stale frames.

Launching first clears the old deck generation, then uploads the retained
preview synchronously to the reusable RGBA texture before starting the
full-resolution decoder. The preview can therefore appear in the same render
frame and is replaced by the first generation-valid scheduled frame. A preview
uses at most 921,600 bytes; all 32 slots use at most 29,491,200 bytes, excluding
the much smaller UI textures.

## Per-clip playback ranges

Every clip slot owns a `ClipPlayback` value independent of deck transport:
source-time In and optional Out points, restart/resume launch mode and an
optional duration in musical beats. The effective end is the earliest of the
explicit Out point, media duration and `In + beats × 60 / BPM`.

Deck transport remains expressed in absolute source seconds but now carries an
In boundary. Restart, normalized seek, loop overshoot and one-shot completion
all operate on `[In, effective end]`. Before switching clips the app records
the outgoing position in runtime slot state. A restart launch starts at In; a
resume launch restores that position if it remains inside the current range,
otherwise it starts at In. Decoder reopen/seek remains generation tagged, so
range changes and loops cannot admit frames from an older launch.

Trim, launch mode and beat duration are persisted per slot with Serde defaults
for older projects. Runtime resume positions are intentionally not project
edits; the active deck position is still included in recovery state.

## Conventional-codec keyframe indexes

Conventional-codec probing scans video packets on the existing background
probe/restore worker and retains keyframe PTS values as exact `MediaTime`
entries. Entries are sorted, deduplicated and capped at 65,536 per clip; a
completion flag distinguishes a full index from a capped one. HAP bypasses
this index because its direct demux/decode path remains separate.

For launch-at-In, resume, loop, restart or playhead seek, the main thread asks
the index for the nearest timestamp at or before the exact target. The FFmpeg
worker opens the source, calls container seek at that anchor, flushes decoder
state, and still discards decoded frames before the exact target. The
scheduler generation changes before the reopen, so neither packets around the
old cursor nor frames produced by an earlier request can reach presentation.
An empty or pre-first-keyframe lookup safely falls back to decoding from the
start.

## Reusable CPU frame leases

Each deck decoder owns a `FrameBufferPool` shared across file reopen, seek,
loop and camera-session replacement. FFmpeg conversion acquires a correctly
sized RGBA vector from the pool, fills it row by row, and wraps it in
reference-counted `FrameData`. Scheduler or payload clones share the same
lease instead of copying pixels.

When the final owner drops, the lease clears the vector and calls `try_send`
on a bounded return channel. The render thread therefore never locks or waits
to recycle storage. A full or disconnected return channel discards the vector
and increments telemetry. The next decode either reuses a returned capacity or
allocates a replacement. Pool capacity is derived from the deck decoder's
bounded frame queue, preserving a finite retained-buffer set.

Atomics count pixel-buffer allocations/reallocations, successful reuse,
returns, discarded returns, live leases and cumulative allocated capacity.
The operator bar aggregates these counters across four decks. A stable
resolution should show allocation count flattening while reuse continues to
rise; a resolution change may cause a deliberate capacity growth.

## Folder import

A dedicated one-command folder scanner performs recursive filesystem work away
from the render thread. It descends at most 16 levels, considers at most 4,096
entries per directory, filters the supported movie/still extensions, and
returns no more paths than currently available clip slots. Directory entries
and the final result are lexically sorted for deterministic assignment.

Assignment begins at the selected deck/slot, wraps through all 32 addresses and
skips slots that already contain ready or pending media. Each assigned path
then enters the existing bounded per-slot restore/probe and thumbnail/preload
workers; folder import does not route through an active deck decoder.
Request IDs reject superseded scans, while project epochs reject results from
a folder scan that predates a project open. The UI tracks probe completion by
address and reports remaining files without waiting on worker threads.

## Missing-media relink

Every path-bearing slot exposes an explicit native-file-picker relink action;
missing selected slots also show the action inline. Starting a relink replaces
only the pending media path and clears the old probe error, retaining the
slot's trim, restart/resume mode, beat duration and resume position. The new
path immediately participates in project snapshots, so saving while a probe is
pending still records the operator's latest selection.

Relink probes use the same bounded restore worker as project and folder loads.
Completion must match both the current project epoch and the slot's current
path. Consequently, opening another project or choosing a second replacement
cannot allow an older result to overwrite the newer intent. A successfully
relinked live slot is relaunched; inactive slots remain ready without changing
the program output.

## Timing and stale-work rejection

Media timestamps use exact rational `MediaTime`; floating-point frame counters
are never the playback clock. Each deck has a generation number. Replacement,
seek, restart and eject invalidate the previous generation. Workers and
schedulers carry the generation on every frame, preventing a late frame from a
previous clip from flashing after a transition.

The scheduler holds the most recent valid frame on underrun and drains stale
frames to the newest eligible timestamp. Decoder and render queues are bounded.

## Decoder failure and soak validation

The deck-decoder test constructor accepts a one-shot failure plan. It arms
against the first successfully opened generation and emits the normal
generation-tagged `DecoderEvent::Error` after a deterministic decoded-frame
count. Consuming the fault allows the same worker to open a later generation,
which exercises the application's real error and recovery channel without
shipping a corrupt fixture. Normal application workers are constructed with
the fault option disarmed.

The default test gate accelerates the long-run invariants: 100,000 fixed-size
frame-lease cycles must allocate once, 10,000 seek generations must never
select an obsolete frame, and 64 real FFmpeg reopen cycles must keep RGBA
allocations bounded. An ignored 10,000-reopen test extends the actual decoder
path for pre-show or release-candidate soak runs.

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

Each deck owns three persisted built-in effect groups containing Geometry,
Color + Levels and Stylize + Key exactly once. The default Geometry → Color →
Stylize display order reproduces the legacy controls. Geometry is always the
bounded UV prepass regardless of its displayed row; only Color and Stylize
change their relative pixel-evaluation order. Row order, bypass and wet mix are
project state, and older projects receive the canonical order during
deserialization. Modulation resolves named parameters before group evaluation,
so reordering a group does not invalidate MIDI or matrix targets.

The current deck implementation remains fused into a single compositor pass;
these built-in groups are not independently executable shader packages.
Geometry wet mix interpolates transformed UVs, while Color and Stylize wet mix
interpolate their RGBA stage result. The planned deck-package path will
materialize only an active deck's processed layer after these groups and before
bus blending. The inactive path must remain the existing bit-identical fused
pass. See [Shader system](SHADER_SYSTEM.md) for that staged extraction.

The master chain supplies the current multipass boundary. Its two reorderable
slots accept Empty, Separable blur, Feedback / trails or a Custom package. When
neither slot is active, the compositor renders directly into the final program
texture and the postprocessor records no passes. With an active chain,
composition first renders into a dedicated input. Blur uses horizontal and
vertical passes, Feedback uses the bounded history target, and a custom package
uses its validated one- or two-pass sequence. An inactive slot inside an active
chain performs a copy into the next fixed stage so slot order remains
deterministic.

Feedback samples one previous final program frame, blends it toward the current
slot input using the persisted 0–0.99 persistence value, and applies the
slot's common wet mix. Only after both master slots finish is the final texture
copied into history. A newly enabled or reset feedback slot copies clean
current output on its first frame, seeds history, and begins trails on the next.

History becomes invalid on clip launch, camera connection, active-slot clear
or eject, project application, composition resize, blackout, and every rendered
period without active feedback. Master freeze skips both composition and
postprocessing, retaining the final texture and history exactly; blackout
overrides freeze, renders black immediately and invalidates history.

The current SDR master target allocates exactly seven program-resolution
RGBA8-sRGB textures at composition resize: composition input, one shared
horizontal scratch target, one ping target, the built-in feedback history, two
custom slot histories and final output. Both slots reuse scratch/ping and never
allocate during a frame. Relative to the original final target, bounded
additional capacity is `6 × width × height × 4` bytes: about 47.5 MiB at 1080p
or 189.8 MiB at UHD. These figures describe the current SDR baseline, not the
planned deck targets or optional HDR tier. Each pass has its own uniform buffer,
preventing later queue writes from changing parameters of earlier encoded
passes.

### Effect package validation and reload

The master postprocessor can watch a versioned `oneiroi-effect` JSON manifest.
The manifest declares package identity, a package-relative WGSL path,
vertex/fragment entry points and parameter schemas. Validation rejects path
traversal, unsupported versions, duplicate or malformed controls, ranges that
do not cover the current master controls, malformed WGSL and entry-point stage
mismatches before the candidate reaches wgpu.

A dedicated worker fingerprints the manifest and shader every 500 ms. Changed
source is parsed with Naga and compiled against the established master bind
group contract outside the presentation loop. A wgpu validation error scope
captures pipeline-layout failures. The render thread receives only a completed
candidate and swaps it between frames; parse, schema or GPU compilation errors
update operator diagnostics while the existing pipeline remains active. Thus
editing or breaking a watched package cannot replace the last-known-good
program path.

The explicit **Refresh registry** action still performs its discovery and
schema/WGSL validation scan synchronously before handing registered packages
to that worker. `SHADER_SYSTEM.md` tracks migration of the manual scan to a
bounded, generation-tagged worker as the remaining S0 realtime gate.

The bundled processor manifest is resolved from the trusted shipped effect root
rather than from the process launch directory. Immediate child packages under
each resolved resource root that target the master runtime are registered by ID
and appear as Custom package choices in either master slot. That includes
manifest-v1 `master_effect` packages and manifest-v2 `effect` packages declaring
target `master` with ABI `master-v1`. Resource-root precedence is deterministic;
a lower-priority duplicate ID is excluded and reported rather than replacing
the first package. Generated controls store values by parameter ID, then pack
them into the fixed 32-float uniform array in declaration order, so schema
reordering does not silently exchange saved controls.

Each registered effect declares one or two fragment passes in the existing
two-slot graph. A one-pass effect renders directly to the slot output. For two
passes, the first writes the shared horizontal scratch texture and the second
reads that intermediate through `effect_texture` while writing the normal slot
output. Both custom slots reuse the same fixed scratch/ping resources
sequentially. Missing or GPU-incompatible packages execute the built-in neutral
copy path. No texture, bind group or pipeline is allocated during a frame.
Chromatic Split demonstrates one pass and Spectral Echo demonstrates two. See
`EFFECT_PACKAGES.md`.

The reload worker compiles every declared fragment entry into a candidate
pipeline set under one validation scope. Only a completely valid set is
published to the render thread, so a broken second pass cannot partially
replace the last-known-good package. `pass_index` and `pass_count` in the fixed
uniform ABI let shared shader code distinguish stages.

A package may additionally declare `previous_slot_output` history. Binding 5
then samples one fixed full-resolution texture owned by that physical master
slot. `history_valid` is zero on the first frame and after source launch/change,
active-source removal, project application, resize, blackout, effect
disable/bypass, missing package or package identity change. After a successful
slot render, its output is copied into that history and the next frame receives
`history_valid = 1`. Master freeze skips the copy and retains history. The two
textures are allocated with the composition target regardless of package use,
making the memory ceiling independent of live package selection.

Custom parameter automation uses a stable 64-bit key derived from package ID
and parameter ID, never the parameter's manifest position. Three master LFOs
feed an eight-route matrix alongside RMS, bass, mid, high, transient, beat
phase and bar phase. Each route scales across half the target
schema range and clamps at the validated bounds. Routes are evaluated directly
while packing the existing uniform array, with no cloned chain or heap work in
the render path. MIDI uses the same key, while project v3 retains the readable
package and parameter IDs.

Free-running LFOs derive phase from elapsed seconds. Synchronized LFOs derive
phase from the internal clock's beat position, so BPM changes retain musical
phase.

## Color and composition

Each deck is processed and composited in deck order into its assigned Bus A or
Bus B accumulator. Only after both accumulators are complete are the selected
linear or equal-power gains applied. This prevents a layer on one bus from
changing the internal result of the other bus.

Each incoming layer selects one of 35 blend functions against the straight-color
backdrop, then uses the source and backdrop alpha terms to produce a
premultiplied bus accumulator. Standard, Contrast, Component and Oneiroi modes
therefore share correct source-over coverage and operate in linear light. The
separable and non-separable standard modes follow the W3C compositing formulas;
the nine Oneiroi modes retain the same alpha contract.

Before writing mixer globals, the CPU resolves deck visibility. If any Solo is
active, only soloed decks remain eligible; Bypass then excludes its deck even
when soloed. This produces effective zero levels without mutating the saved
deck levels or any composition/effect settings.

Layer geometry uses an inverse UV transform before source effects: output
position is translated back into deck-local coordinates, inverse-rotated,
scaled and optionally flipped. Coordinates outside `[0, 1]` resolve to
transparent black. Because this happens before source interpretation, RGBA,
HAP, still images and camera frames share identical transform behavior.
The cropped source aspect is compared with the composition aspect: Fit
restricts the visible layer region, Fill restricts the sampled source region,
and Stretch maps directly. Stretch is the compatibility default for projects
saved before source modes existed.

The current SDR compositor renders once into a fixed-resolution sRGB program
texture. Presentation passes sample that texture into the operator and output
surfaces. A planned RGBA16Float tier applies only to internal intermediates;
final presentation remains sRGB.
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

`.oneiroi` files are versioned JSON. The current schema is version 5 and
version-one through version-four files are migrated on load. Version 3 adds
stable custom-effect package IDs and named parameter values. Saves write a
temporary sibling and rename it atomically. Newly introduced fields use
explicit Serde defaults so existing projects remain readable. Autosave/recovery
state is intentionally separate from the user's saved project.

## Audio analysis

`oneiroi-io` owns CPAL device enumeration and the live stream. Its callback
downmixes interleaved input into fixed 1024-sample stack chunks and uses
`try_send` on an eight-slot synchronous queue. It never performs FFT, waits on
a lock or touches UI state. Queue pressure drops the newest completed chunk
and increments an atomic overrun counter.

The analysis worker owns the Hann window and 1024-point FFT. It publishes
smoothed RMS, peak, bass (20–250 Hz), mid (250 Hz–2 kHz), high (2–16 kHz) and
positive-onset transient values through a tiny snapshot lock. The render thread
copies that snapshot once per frame; the modulation resolver treats the five
audio values like additional sources. Callback failure snapshots resolve to
zero.

Adaptive normalization maintains a slowly smoothed gain toward a target RMS,
but only updates above the noise floor so silence cannot wind gain upward.
Manual gain remains multiplicative and normalization is opt-in. Beat and bar
phase do not pass through audio analysis; they are derived from the same
musical position used by synchronized LFOs and enter the generalized matrix as
sources 8 and 9.

## MIDI input and mapping

`oneiroi-io` uses the platform MIDI service through `midir`. Discovery assigns
a stable name-based identity, and the selected port feeds a 256-event
synchronous queue. The callback only parses note, CC or pitch-bend packets,
increments atomics and calls `try_send`; a full queue drops the newest event
and records the loss.

The main thread drains available events once per frame and passes them to the
device-neutral `MidiMapper`. Learn state, absolute and relative decoding,
toggle/momentary behavior, inversion, output scaling and pickup live in
`oneiroi-core`. Resolved updates then reach mixer, transport, clip/scene,
effect, LFO or matrix state. Blackout and master freeze are applied directly,
while clip and scene launches retain musical quantization. Device topology is
polled every two seconds; a missing selected controller is dropped safely and
reconnected by identity when it returns. Mappings remain persisted even while
their controller is absent.

## Non-negotiable invariants

- The main/render thread never waits for media decode or disk I/O.
- Queues that can receive media frames are bounded.
- Obsolete generations never reach presentation.
- HAP playback never takes the FFmpeg pixel-decoding path.
- Base parameters remain distinct from resolved modulation values.
- A camera backlog is dropped rather than converted into latency.
- MIDI callbacks never wait on the render thread and queue loss is observable.
- Invalid project values are rejected before application.
