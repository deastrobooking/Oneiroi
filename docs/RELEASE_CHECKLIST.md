# Release-candidate checklist

Use this checklist on the target show machine for every candidate. Record the
date, commit, release-binary hash, macOS version, GPU, displays, audio device,
MIDI controllers and media fixture paths with the result.

## Automated gate

```sh
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --release
shasum -a 256 target/release/oneiroi
```

Before a numbered release, also run the opt-in decoder soak:

```sh
cargo test -p oneiroi-media --test hap_mov_demux \
  extended_decoder_reopen_soak -- --ignored --exact
```

## Media fixture set

Keep a local, redistribution-safe fixture folder containing:

- two and four simultaneous 1920 × 1080 60 fps HAP clips;
- conventional long-GOP H.264 or H.265 and intraframe ProRes/DNx media;
- one PNG and one JPEG still;
- one live camera or capture-card input when hardware is available;
- a saved v5 project that fills representative slots and enables deck/master FX.

Record media duration, codec, dimensions and frame rate. Do not commit licensed
show content to the repository.

## Thirty-minute performance pass

1. Start `target/release/oneiroi` and load the fixture project.
2. Run two 1080p60 HAP decks for ten minutes, then four for twenty minutes.
3. Exercise scenes, quantized launches, seek/restart, crossfader, deck FX,
   master effects, freeze, blackout and Show Mode.
4. Select decks in both directions, especially D → A/B, and confirm the primary
   editor follows every deck-row and clip-slot selection.
5. Move and delete a non-playing test clip, including the Delete/Backspace path
   outside Show Mode; verify Show Mode blocks deletion.
6. Confirm RGBA allocation stabilizes for fixed-resolution conventional media
   and decoder dropped/repeated/late counters remain explainable.

Pass criteria: no crash, blank program frame, unbounded counter/memory growth,
stuck deck selection or stale frame after seek/source replacement.

## Output and lifecycle rehearsal

- Move output between every intended display and toggle fullscreen/windowed.
- Verify aspect preservation, test card, Identify, freeze and blackout.
- Disconnect/reconnect the program display and confirm recovery telemetry.
- Disable/re-enable output without stopping media decode.
- Sleep/wake once and repeat output enable/fullscreen.
- Exercise a composition-size change and verify feedback/history resets cleanly.

## Device and failure rehearsal

- Deny then grant camera/microphone permission on a clean test account.
- Disconnect/reconnect the selected audio input and each requested MIDI device.
- Verify MIDI soft takeover, multiple-controller input and emergency controls.
- Start/stop OSC input and feedback; send malformed and future-timetag packets
  and confirm bounded error/drop counters.
- Make the session-journal destination temporarily unavailable or unwritable;
  program output must continue and the error must remain visible.
- Restore a prior take, scrub to a marker and continue as a named branch.

## Shader and effect-package rehearsal

- Launch the release binary from an unrelated working directory and verify the
  bundled processor plus Chromatic Split, Spectral Echo, Temporal Melt,
  Recursive 2D Lab, Fractal Volume 3D and Hyper Recursion 4D+ are discovered.
- Repeat discovery from the packaged application bundle and verify a workspace
  package with a duplicate bundled ID is rejected rather than substituted.
- Select and render all three algorithmic packages on real GPU hardware.
- While one is live, introduce a WGSL syntax error and an invalid manifest;
  program output must keep the last-known-good pipeline and expose the error.
- Restore the package, then test a valid package-ID rename and physical package
  removal. The old identity must disappear without retaining a stale
  descriptor or pipeline.
- Exercise master package dry/wet, bypass, reorder, Show Mode cards, blackout,
  freeze and composition resize while watching for invalid history reuse.

For the `deck-v1` runtime, run four simultaneous 1080p deck packages and verify
dry/bypass bit-exactness, transparent-alpha preservation,
effect-before-every-blend ordering, invisible-deck culling and recorded 1080p/
UHD texture ceilings. See [Shader system](SHADER_SYSTEM.md).

Use the synthetic release benchmark as a repeatable package-path baseline,
then corroborate it with the media fixture pass above:

```sh
cargo run --release -p oneiroi-render --example perf -- \
  --decks 4 --width 1920 --height 1080 \
  --deck-package chromatic-split --frames 600 --warmup 60 --runs 3 --json
```

## Packaging gate

Before assigning a release version:

- produce a macOS app bundle with camera and microphone usage descriptions;
- settle dynamic/static FFmpeg distribution and ship required notices;
- sign and notarize the bundle;
- test first launch on a clean machine without the development toolchain;
- archive the exact binary hash, project fixture, test record and known issues.
