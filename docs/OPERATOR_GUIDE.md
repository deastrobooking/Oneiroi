# Oneiroi operator guide

## Session journal

Every app run records its supported performance commands under
`.oneiroi/session/` in the current workspace. The operator header reports
journaled command and checkpoint counts, queue overruns and persistence errors.

Journal writing is bounded and asynchronous. If storage stalls or fails,
program output continues and the in-memory performance take remains active.
The runtime creates an atomically replaced recovery checkpoint every 600
rendered frames.

Open **Session recovery**, select **Scan journals**, choose a prior take and
select **Restore take**. The active journal is never offered as a recovery
candidate. The panel reports checkpoints and safely ignored torn tails. Load
the matching `.oneiroi` project first so recovered clip launches resolve to the
same media slots. Restore applies mixer, transport, output, effect, LFO and
modulation state, then continues recording in a fresh journal.

Version-five projects carry a stable project identity and a bounded catalog of
take names, IDs, journal filenames and creation times. The recovery scan hides
journals linked to another project and labels older journals as unlinked.
Enter a printable name before **Start named take** or **Restore as named
branch**. Scoped deterministic seeds can be edited in the same panel. Seeds
and the active typed graph are saved with the project.

Use the timeline slider to select an earlier show time, then choose **Restore
cursor as branch**. Replay starts from the closest preceding checkpoint.
Enter a label and choose **Add marker** while recording; after scanning that
journal, marker buttons position the replay cursor at their exact show times.
Project take metadata can be renamed or removed; removal only unlinks the
catalog entry and deliberately leaves the journal file on disk.

Choose **Export copy** to copy the selected take into the displayed export
directory (relative paths resolve from the workspace), or **Archive copy** to
copy it under `.oneiroi/archive`. Each operation creates a new unique folder
with the journal and its checkpoint when present. Existing exports and source
session files are never overwritten, moved or removed.

MIDI mappings, performance keyboard shortcuts and continuous mixer/effect/LFO
controls use the same command gateway. Their origin is retained in the journal,
so a replay can distinguish an operator gesture from a specific MIDI device or
keyboard emergency command.

Open **OSC input**, enter a UDP bind address and choose **Listen** to accept
remote mixer, deck, clip, scene, tempo and output commands. Use loopback
(`127.0.0.1:9000`) unless the show-control network should reach the app. OSC
sender addresses are retained as journal origins, and malformed/overflow
counters are visible beside the listener status. The complete route table is
in [OSC.md](OSC.md).

Deck transforms, crop/source/blend choices, effect-slot order, LFO and
modulation routing, master effects/modulation, and accepted media assignments
are also journaled as stable field commands. Project opening establishes a new
baseline and is not recorded as live performance traffic.

Oneiroi is a four-deck live video mixer. Each deck can play a clip or receive a
live camera/capture-card feed, run its own effects and modulation, and feed the
A/B crossfader.

## Start the application

```sh
cargo run -p oneiroi-app
cargo run -p oneiroi-app -- clip-a.mov clip-b.mp4
cargo run -p oneiroi-app -- performance.oneiroi
```

For performance testing, use a release build:

```sh
cargo run --release -p oneiroi-app
```

FFmpeg and its development libraries must be installed. HAP uses the direct
GPU-compressed path; other supported movies and stills use FFmpeg.

## Load and trigger clips

1. Select deck A, B, C or D.
2. Select one of its eight clip slots.
3. Drag a MOV, MP4, MKV, AVI, WebM, MXF, PNG or JPEG file onto the window.
4. Wait for the slot to show its thumbnail and filled-circle first-frame
   readiness marker.
5. Click the slot to launch it.

To populate multiple slots, select the desired starting slot and drag a folder
onto the window. Oneiroi recursively finds supported media, sorts paths
lexically, fills from the selected slot, wraps across decks and skips occupied
slots. At most the 32 available clip addresses are assigned. The status line
reports scanning, probe progress, truncation caused by available capacity and
completion.

Folder scanning accepts MOV, MP4, M4V, MKV, AVI, WebM, MXF, PNG, JPG and JPEG.
It is bounded to 16 directory levels and 4,096 entries per directory. Files
that fail probing remain visible in their assigned slot with an error while
the rest continue importing.

Scene buttons and number keys `1`–`8` launch the same slot across all four
decks. Choose Immediate, Next beat or Next bar before triggering.

A filled circle means the bounded first-frame launch preview is ready. An open
circle means metadata is ready but the preview worker is still decoding. The
header shows the total ready count out of 32. On a ready launch, this preview
appears immediately and is replaced by the full-resolution decoder output.

Each active deck exposes level, A/B bus assignment, play/pause, restart,
freeze, loop/one-shot, speed and seek controls. Camera decks expose freeze but
disable file-only transport controls.

Expand **Selected clip playback** below the clip grid to configure the selected
slot:

- **Restart at In** always launches from the trim start.
- **Resume last position** remembers where that slot was when another source
  replaced it; reaching the end causes the next resume to start at In.
- **In** and optional **Out** are source-time seconds.
- **BPM-relative duration** limits the range to a number of beats from In. The
  effective end uses whichever comes first: Out, media end or beat duration.

The deck playhead, Restart, Loop and One shot controls all respect this
effective range. Changing BPM immediately changes a beat-relative boundary.

Conventional-codec slots show their indexed keyframe count in the deck
metadata and clip tooltip. A `capped` marker means the clip reached the
65,536-entry safety limit; seeking still works, but targets after the indexed
region may require a longer forward decode from the last indexed anchor. HAP
clips use their direct compressed path and do not report a conventional
keyframe index.

The performance line includes **RGBA pool** diagnostics:

- **alloc** counts new pixel buffers or capacity growth.
- **reuse** counts frames served by returned storage.
- **live** is the number of leases still held by decode/scheduling/render.
- **discard** counts non-blocking returns dropped because the bounded pool was
  full or gone.
- **MiB** is cumulative allocated pixel capacity, not current resident memory.

During stable-resolution playback, `alloc` should flatten while `reuse`
continues increasing. Growth after a resolution switch is expected; continuous
growth at a fixed resolution should be treated as a soak-test failure.

## Decoder failure rehearsal and soak

The normal workspace tests include an injected mid-stream decoder failure,
recovery on a new generation, 100,000 frame-buffer reuse cycles, 10,000
generation changes and 64 FFmpeg reopen cycles. Before a release candidate,
run the extended 10,000-reopen decoder soak:

```sh
cargo test -p oneiroi-media --test hap_mov_demux \
  extended_decoder_reopen_soak -- --ignored --exact
```

The test fails if fixed-resolution RGBA allocation grows beyond the decoder's
bounded working set, a generation is mislabeled, a reopen fails or a frame
lease remains live after the source ends. This complements, but does not
replace, a show-machine soak with the actual media, capture devices and output
displays.

## Connect a camera or capture card

1. Select the destination deck.
2. Choose an AVFoundation device in the Camera toolbar.
3. Set the requested width, height and frame rate.
4. Click **Connect to Deck**.

Use **Refresh** after attaching hardware. A manual AVFoundation device ID such
as `0` can be entered when discovery does not return a label. macOS may require
camera permission for Oneiroi or Terminal. HDMI capture cards that appear as
AVFoundation video devices use the same path.

Live capture uses a bounded queue. If rendering stalls, stale camera frames are
dropped to keep latency from growing.

## Effects

Open **GPU effects** on a deck. Available controls include:

- Hue, contrast, saturation, black level, white level and gamma
- Bit reduction and black-light inversion
- Mirror, neon glow, fractal fold, jitter and find edges
- Pixelate and luma key

The chain has three rows: **Geometry**, **Color + levels**, and **Stylize +
key**. Use the arrow buttons to reorder them. Every row has an independent
**Bypass** and **wet** control; zero wet returns that stage to its dry input
without changing its parameter knobs. **Load preset** offers Neutral, Neon
night, Blacklight and Glitch. **Reset chain** or **Reset effects** restores
neutral parameters, full wet, no bypass and the legacy-compatible
Geometry → Color → Stylize order.

Effects run on each source before deck composition. Slot order, bypass and wet
mix are saved in the project. Existing projects that predate effect slots open
with the legacy-compatible default order.

### Master effects and blur

Expand **Master effects** below the crossfader and master controls. Two
reorderable slots can be Empty, Separable blur or Feedback / trails. Blur
exposes a 0–32 pixel radius. Feedback exposes 0–0.99 persistence; larger values
retain more of the previous final frame. Both use the common bypass and wet
controls. The arrow buttons change master evaluation order, and **Reset master
effects** returns both slots to Empty.

With both slots empty or bypassed, composition renders directly to program
output. Enabling blur or feedback activates fixed ping-pong/history targets
allocated at the chosen composition resolution; no effect texture is created
during a frame. The program target also reserves one custom history texture per
master slot. UHD uses about 189.8 MiB of additional bounded texture storage
versus the direct path, so certify the target GPU at show resolution.

Feedback history resets on a source launch/change, active source removal,
project load, composition resize, blackout, or after feedback is disabled.
The first frame after reset is clean and seeds new history. Master freeze holds
the exact final frame and pauses history evolution; blackout still takes
priority and clears the future history state.

The **Effect package** field points to a versioned JSON manifest. The bundled
default is `effects/master-effects/effect.json`. Choose **Watch** after changing
the path; the app checks the manifest and referenced WGSL every 500 ms.
**Reload now** requests an immediate compile even when the files appear
unchanged. Successful reloads show the package name and fingerprint. Rejected
schema, WGSL or GPU pipeline changes are shown in amber and the last working
pipeline remains on program output.

Package shader paths must be relative to the manifest and cannot traverse out
of their directory. A replacement `master_processor` package must retain the
documented master-v1 bindings and declared vertex/fragment entry points.

Packages with role `master_effect` in an immediate subdirectory of `effects/`
appear when a master slot is set to **Custom package**. Select the package and
its manifest controls are created automatically. **Refresh registry** rescans
after adding or removing a package. Parameter values and package IDs are saved
in project version 3. If a saved package is missing or rejected, that slot
passes its input through unchanged. The package panel reports whether the
selected effect uses one or two bounded render passes.

Expand **Master modulation matrix** for three free-running or tempo-synced LFOs
and eight routes. A route can use a master LFO, RMS, bass, mid, high, transient,
beat phase or bar phase, then target any parameter in either active custom
slot. Amount is bipolar; negative values invert the source. Targets use stable
package/parameter identity, so reordering controls in a manifest does not
redirect saved routes.

Each generated custom parameter also has **MIDI learn** and **Clear** buttons.
Connect a controller, click Learn and move the desired hardware control.
Custom ranges outside 0–1 can be adjusted in the MIDI mapping table's output
range fields.

**Chromatic Split** is the bundled one-pass example. **Spectral Echo** is the
two-pass example: its first pass produces an intermediate in the fixed scratch
texture and its second pass combines that intermediate into the slot output.
**Temporal Melt** demonstrates the optional previous-slot-output history.
Its first frame after selection or reset is clean; subsequent frames sample the
saved slot output. The custom package panel identifies temporal packages.

## Layer transforms

Open **Layer transform** on a deck to adjust:

- Horizontal and vertical position in normalized output coordinates
- Uniform scale from 0.05× through 4×
- Rotation from -360° through 360°
- Independent horizontal and vertical flips
- Left, right, top and bottom normalized crop
- **Fit** to preserve the full image with transparent bars
- **Fill** to preserve aspect while centrally cropping to cover the layer
- **Stretch** to map the cropped source directly to the layer

Pixels moved outside the layer bounds become transparent instead of smearing
the source edge. **Reset transform** restores centered, unscaled, unrotated
and uncropped Stretch geometry. Transform settings are stored per deck in the
project.

## Blend modes

Choose a blend mode beside each deck's Bus A/Bus B assignment:

- Normal
- Add
- Screen
- Multiply
- Difference
- Lighten
- Darken
- Overlay

The mode controls how that deck combines with layers already accumulated
inside its assigned bus. Blending is calculated in linear light with
alpha-correct source-over coverage. The selected mode is stored in the project;
older projects load as Normal.

## Solo and bypass

Use **Solo** to isolate a deck without changing any other deck settings.
Multiple soloed decks remain active together, including decks assigned to
different buses. When any Solo is active, non-solo decks are excluded before
bus composition.

Use **Bypass** to remove a deck from composition while preserving its level,
bus, transform, blend mode and effects. Bypass takes precedence over Solo.
These controls are stored in the project; older projects load with every deck
active and unsoloed.

## LFOs and modulation matrix

Open **LFOs + Mod Matrix** on a deck.

Each deck has three LFO sources. An LFO can run in Hz or synchronize to the
internal beat clock at 1/16, 1/8, 1/4, 1/2, 1, 2, 4 or 8 beats per cycle.
Waveforms are sine, triangle, saw up, saw down and square.

Enable **Direct** for a simple one-source/one-destination assignment. Disable
Direct to use the LFO only as a matrix source.

The matrix has eight routes per deck:

- Choose LFO 1–3, Audio RMS, bass, mid, high, transient, beat phase or bar
  phase as the source.
- Choose any continuous effect parameter as the destination.
- Set an amount from `-1.0` to `+1.0`.
- Negative amounts invert the modulation.
- Multiple routes can share a source or destination and are summed safely.

## Audio-reactive modulation

Choose an input in the **Audio** toolbar and click **Connect**. On macOS, grant
microphone/audio-input permission if prompted. The analysis panel displays RMS,
bass, mid, high and transient meters plus sample rate, channel count, bounded
queue overruns and callback errors.

Analysis controls are:

- **Gain**: scales all normalized signals.
- **Noise floor**: suppresses low-level room/device noise.
- **Attack** and **release**: smooth RMS and band envelopes.
- **Transient**: scales positive RMS onsets.
- **Adaptive normalization**: slowly adjusts analysis gain toward the selected
  target RMS; adaptation speed controls how quickly it follows level changes.

The native callback only downmixes into fixed-size chunks and attempts a
non-blocking bounded-queue write. FFT and smoothing run on a worker. If the
queue is full, the chunk is dropped and the overrun counter increases. A
callback error resolves all audio matrix sources to zero.

Beat phase ramps from 0 to 1 every beat. Bar phase ramps from 0 to 1 across
four beats. Both follow the internal tempo clock and retain phase when BPM
changes.

## Tempo

Enter BPM directly or use:

- **Tap**: establishes tempo after two taps and averages recent taps.
- **½**: halves the current BPM.
- **×2**: doubles the current BPM.

Tempo changes preserve the current musical position. The toolbar displays beat
position, beat phase and four-beat bar phase.

## Mixing and emergency controls

Assign decks to Bus A or Bus B, then use the crossfader. Linear and equal-power
curves are available.

Stage-safety shortcuts:

| Key | Action |
|---|---|
| `B` | Toggle master blackout |
| `Space` | Toggle master freeze |
| `O` | Toggle the clean program output |
| `Left` / `Right` | Move the crossfader |
| `Home` | Center the crossfader |
| `1`–`8` | Launch a scene |
| `Cmd/Ctrl+S` | Save the current project |

## Program output

Oneiroi renders the mixer once into an offscreen program texture. The operator
window previews that texture beneath the controls, while the separate
**oneiroi · PROGRAM** window presents it without UI.

Use the top toolbar to:

- Show or hide **Program output**
- Toggle borderless **Fullscreen**
- Select a connected display and refresh the list after reconnecting hardware
- Select 720p, 1080p or UHD composition resolution
- Enter a custom width and height, then click **Apply**
- Show a color-bar/grid **Test card** or magenta **Identify** frame

The selected display receives the window and borderless fullscreen target.
Press `Escape` to leave fullscreen. Display preference, output visibility,
fullscreen state, composition resolution and calibration-overlay state are
stored in the project. Press `O` for an immediate output-window disable/enable
action.

Expand **Output health** to verify the current display, swapchain size,
composition size and FIFO presentation mode. The counters distinguish
presented and skipped frames, automatic reconfigurations, successful
recoveries, timeouts, occlusion, validation errors and display-topology
changes. Connected displays are polled every two seconds, so reconnecting an
adapter or projector updates the target list without restarting the app.

## MIDI controllers

Expand **MIDI control**, choose a controller and press **Connect**. To create a
mapping, choose a target, press **Learn**, then move a knob, encoder, fader or
button. **Cancel learn** exits without changing a mapping; **Clear target**
removes every mapping for the selected target. Individual rows can also be
removed.

Each row supports:

- **Absolute** for normal knobs/faders and pitch bend.
- **Momentary** for press/release behavior.
- **Toggle** for one-button latching.
- **Relative offset** for encoders centered on value 64.
- **Relative 2's comp** for encoders sending `1`/`127` increments.
- Editable output minimum/maximum, inversion and pickup/soft takeover.

Mappings cover crossfader/master controls, all four deck transports and
levels, clip and scene launches, effects, LFO parameters and modulation-matrix
routes. Blackout and master freeze act immediately; clip and scene launches
still follow the current quantization setting.

The activity line shows received packets, queue drops and parse errors. If a
connected controller disappears, Oneiroi keeps its mappings and attempts to
reconnect the selected identity every two seconds. Use **Disconnect** to stop
that automatic reconnect intent.

## Projects and recovery

The project toolbar can open and save `.oneiroi` files. Version-three projects store all 32
clip paths, per-slot trim/launch/beat settings, deck state, camera reconnect
settings, mixer values, transport, effects, LFOs, modulation routes, tempo,
output settings and MIDI mapping data.
Version-one projects are upgraded when loaded.

Oneiroi writes a recovery autosave after changes and on close. Use **Recover
autosave** when the recovery copy is newer. Missing files remain represented in
their original slots. Select a missing slot and press **Browse and relink…**,
or right-click any path-bearing slot and choose **Relink media…**. The native
picker starts beside the previous file when that directory still exists.
Relinking preserves the slot's In/Out trim, launch mode and beat duration. If
the slot is live, a successful relink launches the replacement automatically.

## Pre-show checklist

1. Run a release build on the actual show machine.
2. Confirm camera, audio and MIDI permissions.
3. Trigger every required clip and inspect media-health warnings.
4. Verify HAP clips report the direct path.
5. Exercise blackout and freeze.
6. Watch FPS and dropped/repeated/late counters.
7. Exercise every MIDI mapping, including pickup after moving a saved
   parameter away from the hardware position.
8. Disconnect and reconnect the controller and confirm activity resumes.
9. Save the project, close it, and verify restoration.
10. Select the stage display and verify the test card through the full signal path.
9. Disable sleep, automatic updates and unnecessary background applications.
