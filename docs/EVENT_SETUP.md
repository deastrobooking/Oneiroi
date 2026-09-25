# Event setup on this Mac

Build with `sh scripts/build-macos.sh`, then open `target/release/VIRTUAL.app`.
The native executable is also available at `target/release/virtual`.
This local Apple Silicon build uses this Mac's installed Homebrew FFmpeg
libraries. It is ad-hoc signed, not notarized or self-contained for other Macs.
Keep the installed libraries available through the event.

The app bundle includes its effects and camera, microphone and local-network
permission descriptions. Allow the permissions for inputs you choose to use.
Bundled launches store untitled recovery and session journals under
`~/Library/Application Support/VIRTUAL`; command-line launches retain their
working directory. Open your existing `.virtual` project explicitly.

## Projector

1. Connect the projector and use an extended desktop in macOS.
2. In Output settings, select its display, enable program output and fullscreen.
3. Use Identify and the test card to confirm routing and aspect ratio; turn both
   off before the set. Start with a 1920 × 1080 composition.
4. Test blackout, master freeze/resume and Show Mode. Rehearse cable reconnect
   and reselect the projector if macOS changes its display identity.
5. Run the actual project for at least 30 minutes with the intended effects and
   inputs. Watch FPS and output diagnostics. Synthetic tests do not certify the
   projector, adapters or the whole show chain.

## Sound-reactive visuals and tap tempo

1. Under Audio, select the room microphone, DJ mixer/audio interface, or the
   capture card's separately exposed audio input. Click Connect.
2. Open Audio analysis. Confirm moving RMS, Bass, Mid, High and Transient meters;
   adjust gain and noise floor. Attack/release shape the response. Adaptive
   normalization is optional and should be rehearsed with quiet and loud input.
3. In the selected deck's modulation matrix, choose Audio bass, mid, high,
   transient or RMS, select a destination and raise the route amount gradually.
   Deck-package routes and the master matrix expose their supported destinations.
   These signals control the available modulation destinations, not every
   application action.
4. Tap tempo is in the always-visible toolbar, including Show Mode. Tap with the
   DJ's beat. Audio analysis supplies spectral/envelope signals; it does not
   automatically estimate BPM. A locked MIDI clock disables manual tempo edits.
5. The toolbar retains audio meters during the set. If the input fails, reconnect
   it and confirm the meters before relying on modulation.

Video-file soundtracks and embedded HDMI audio are not decoded by the video
path. Select a separate macOS audio input for analysis. Capturing the HDMI card's
audio depends on the card exposing an audio device to macOS.

## Live camera and HDMI input

1. Select a deck and open **Video input · camera / capture card**.
2. Refresh and select the camera or HDMI capture device. On the review machine,
   discovery found **FaceTime HD Camera** and **Guermok USB3 Video**.
   macOS also lists Guermok as a two-channel, 48 kHz audio input; select it
   separately under Audio when you want to analyze the HDMI source's sound.
3. Match the source's supported resolution and frame rate, initially with
   Automatic pixel format. Connect to Deck and confirm a live image.
4. Test signal loss, reconnect, switching sources and loading the saved project.
   Discovery alone does not prove that the source has signal or decodes correctly.

See [video inputs](VIDEO_INPUTS.md) for fractional rates and driver limitations.

## Ableton Link

Select MIDI → Clock sync → Ableton Link, grant local-network access if prompted,
and check the peer count. Rehearse with the actual DJ rig. Link shares tempo and
phase; it does not start/stop decks. MIDI clock output follows tempo but does not
lock its pulse phase to Link. Large external beat jumps re-quantize queued clips
to the next boundary on the new timeline.

## Before leaving for the event

Save the show project, back up its media, and keep this exact build. Rehearse all
inputs, projector routing and emergency controls together. The full hardware
matrix is in [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).
