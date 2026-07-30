# Oneiroi operator guide

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
4. Wait for the slot to show its thumbnail and ready state.
5. Click the slot to launch it.

Scene buttons and number keys `1`–`8` launch the same slot across all four
decks. Choose Immediate, Next beat or Next bar before triggering.

Each active deck exposes level, A/B bus assignment, play/pause, restart,
freeze, loop/one-shot, speed and seek controls. Camera decks expose freeze but
disable file-only transport controls.

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

Effects run on each source before deck composition. **Reset effects** restores
neutral values for that deck.

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

- Choose LFO 1, 2 or 3 as the source.
- Choose any continuous effect parameter as the destination.
- Set an amount from `-1.0` to `+1.0`.
- Negative amounts invert the modulation.
- Multiple routes can share a source or destination and are summed safely.

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

## Projects and recovery

The project toolbar can open and save `.oneiroi` files. Version-two projects store all 32
clip paths, deck state, camera reconnect settings, mixer values, transport,
effects, LFOs, modulation routes, tempo, output settings and MIDI mapping data.
Version-one projects are upgraded when loaded.

Oneiroi writes a recovery autosave after changes and on close. Use **Recover
autosave** when the recovery copy is newer. Missing files remain represented in
their slots so they can be relinked in a future release.

## Pre-show checklist

1. Run a release build on the actual show machine.
2. Confirm camera and media permissions.
3. Trigger every required clip and inspect media-health warnings.
4. Verify HAP clips report the direct path.
5. Exercise blackout and freeze.
6. Watch FPS and dropped/repeated/late counters.
7. Save the project, close it, and verify restoration.
8. Select the stage display and verify the test card through the full signal path.
9. Disable sleep, automatic updates and unnecessary background applications.
