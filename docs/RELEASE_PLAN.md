# Release plan

Target: first signed and notarized macOS release.

Baseline recorded on 2026-08-30 from commit `2bfff4f`:

- [x] `cargo fmt --check`
- [x] `cargo test --workspace` (270 passed; extended decoder soak ignored)
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [x] `cargo build --release`
- [x] Record the current development binary SHA-256:
  `17f0476d3d5637a80feb261335f0c0fa55cf2b98ab2d0284a21b866900cc27e9`

The development hash is evidence for the baseline only. The final archive must
record the hash of the packaged, signed release artifact.

Step 1 validation after the implemented code gates: 273 workspace tests pass;
the extended decoder soak remains intentionally ignored until target-machine
certification.

## 1. Close advertised shader release gates

- [x] Move manual effect-registry discovery and manifest/WGSL validation to a
  bounded, generation-tagged worker.
- [x] Publish only the newest completed registry scan and retain the visible
  catalog while a scan is running.
- [x] Add stable, persisted deck-package MIDI destinations.
- [x] Add stable deck-package OSC input and feedback routes.
- [x] Add eight bounded, persisted deck-package modulation routes per deck,
  using stable parameter keys and the existing LFO/audio/beat/bar sources.
- [x] Add transparent-alpha regression coverage for the deck-package boundary.
- [x] Add visible per-frame deck-package execution/culling telemetry and
  regression coverage for invisible-deck culling.
- [x] Add non-blocking per-deck GPU timestamps for precomposition and package
  passes, with triple-buffered readback and operator diagnostics.
- [ ] Validate per-pass GPU timing on release hardware.
- [x] Extend the release benchmark with selectable `deck-v1` package branches.
- [x] Record a provisional 2026-08-31 four-deck 1080p result on the development
  M3 Pro: Chromatic Split, HAP BC1, 3 × 600 measured frames, 3.12 ms median
  sustained, 320.04 fps, 1.10× run spread, 1.55 ms mean precomposition and
  1.54 ms mean package GPU time per active deck, passing the 16.67 ms budget.
- [x] Record the fixed deck-package target ceiling: 39.6 MiB at 1080p and
  158.2 MiB at UHD (five RGBA8-sRGB textures).
- [ ] Repeat and archive the four-deck benchmark on the designated show
  machine; the development result is not release certification.
- [ ] If any item is deferred, remove or qualify the corresponding promise in
  the release notes and feature documentation.

## 2. Produce a self-contained macOS application bundle

- [ ] Add a repeatable `.app` packaging command or script.
- [ ] Add bundle identity, version and icon metadata.
- [ ] Add camera and microphone usage descriptions to `Info.plist`.
- [ ] Install the executable under `Contents/MacOS` and bundled effects under
  `Contents/Resources/effects`.
- [ ] Verify effect discovery when launched from an unrelated directory.

## 3. Resolve FFmpeg distribution

- [ ] Choose and document the dynamic/static FFmpeg distribution strategy.
- [ ] Remove the release binary's dependency on `/opt/homebrew/opt/ffmpeg`.
- [ ] Bundle and relocate every required non-system dynamic library when using
  dynamic distribution.
- [ ] Complete the FFmpeg licensing review and ship all required notices.

## 4. Certify a release candidate on the target show machine

- [ ] Run the ignored 10,000-reopen decoder soak.
- [ ] Run the complete media fixture and thirty-minute performance pass in
  [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).
- [ ] Rehearse display reconnect, sleep/wake and composition resizing.
- [ ] Rehearse camera/audio permissions, device loss, MIDI reconnect, OSC
  failures and an unavailable session-journal destination.
- [ ] Rehearse shader syntax/manifest failures, package rename/removal,
  last-known-good behavior and Show Mode controls.
- [ ] Record the date, commit, binary hash, macOS version, GPU, displays, audio
  interface, MIDI controllers and fixture details with the result.

## 5. Sign and notarize

- [ ] Sign nested libraries and the application bundle.
- [ ] Apply the hardened-runtime and entitlement configuration required by the
  selected distribution path.
- [ ] Notarize and staple the application.
- [ ] Verify signatures, notarization and Gatekeeper acceptance.

## 6. Verify clean-machine installation

- [ ] Test first launch on a clean supported Mac without Homebrew, Rust or the
  development toolchain.
- [ ] Verify permissions, media decode, bundled effects, output selection and
  project save/recovery.

## 7. Finalize and publish

- [ ] Replace `Unreleased` in `RELEASE_NOTES.md` with the version and date.
- [ ] Reconcile stale project-v5/project-v6 wording across the documentation.
- [ ] Record known issues and supported macOS/hardware expectations.
- [ ] Assign the release version only after packaging and certification pass.
- [ ] Tag the certified commit and archive the exact signed bundle hash,
  fixture/project, test record, notices and known issues.
