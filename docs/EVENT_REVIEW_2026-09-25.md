# Event review — 2026-09-25

Scope: local event preparation on the user's Apple M3 Pro Mac, macOS 26.5.1
(25F80), based on `b6916af` plus this review's working-tree changes. Priorities:
projector output, audio-reactive modulation, tap tempo, camera and HDMI capture.

## Fixed

- **Queued launch timing:** joining an external timeline or receiving a large
  MIDI/Link beat jump could strand a clip on the previous beat count, or trigger
  it immediately off-grid. Jumps of at least one beat now move queued launches
  to the next boundary using each launch's original quantization. Smaller
  corrections keep their targets; immediate launches remain immediate.
- **Stale audio modulation:** a driver that stopped samples without emitting an
  error left the last loud reading active indefinitely. The worker now clears
  all readings after 250 ms without samples and recovers when samples resume.
  A failed input replacement also clears the prior snapshot.
- **Live controls:** BPM, tap tempo and connected-input audio meters are visible
  in Show Mode. The audio indicator flags callback errors.
- **Link status/test:** Link no longer displays MIDI's "Not following" status;
  no peers is explicitly reported. The opt-in test's original failure was an
  incorrect decimal precision assumption: 133 BPM arrives over Link as
  132.9999468000213 BPM because its wire format uses integer microseconds per
  beat. The test now checks the expected wire value in both directions.
- **Local packaging:** a repeatable app bundle includes effects, input/network
  privacy strings, ad-hoc signing, dependency inventory and a binary checksum.
  Bundled launches use a writable Application Support recovery directory.
- Fixed formatting failures and stale current-project-schema documentation.

## Validation

| Check | Result |
|---|---|
| `cargo fmt --check` | Passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Passed |
| `cargo test --workspace --locked` | 298 passed, 0 failed; 2 opt-in tests excluded |
| Two-peer Link discovery/tempo test, run explicitly | Passed in both directions |
| 10,000-reopen decoder soak, run explicitly | Passed |
| `sh scripts/build-macos.sh` | Passed; native arm64 executable and app bundle |
| Plist validation and strict code-signature verification | Passed |
| Bundled resources | All 18 effect manifests included |
| App startup from unrelated `/tmp` working directory | Ran for 10 seconds, no logged errors; Application Support journal created |

The startup process was terminated by the smoke-test harness after 10 seconds;
this is not a graceful-exit, interactive UI or sustained show certification.

Synthetic renderer benchmark, four decks at 1920 × 1080, 3 runs of 600 frames
after 60 warm-up frames, 16.67 ms sustained budget:

| Source / effects | Median sustained ms/frame | Equivalent throughput | Gate |
|---|---:|---:|---|
| HAP BC1, neutral | 3.6258 | 275.80 fps | Pass |
| RGBA8, neutral | 5.3424 | 187.18 fps | Pass |
| HAP BC1, Chromatic Split per deck | 2.8661 | 348.91 fps | Pass |

These runs measure synthetic upload/render throughput, not decoder throughput,
projector presentation, capture latency or the final combination of show effects.
Run-to-run machine load and GPU timing differ; do not interpret this as a claim
that enabling an effect makes the same live show faster.

## Hardware observed and work remaining

- Native video discovery found **FaceTime HD Camera** and **Guermok USB3 Video**.
- macOS lists Guermok with two input audio channels at 48 kHz and the MacBook
  microphone with one channel at 44.1 kHz. Audio signal and capture permissions
  still need to be checked interactively.
- Only the built-in display was connected. Projector/fullscreen routing,
  reconnect, adapters and sustained presentation remain unverified.
- Camera/card frame delivery, HDMI signal loss/reconnect, actual audio modulation
  and permissions were not certified. Run the [event setup](EVENT_SETUP.md) and
  [release checklist](RELEASE_CHECKLIST.md) with the actual show project.
- Link was checked with two peers on this machine, not Ableton Live or the DJ's
  network. Network topology, external phase alignment and end-to-end latency
  require rehearsal. Transport sync is not implemented.
- Spectral analysis reads a selected macOS audio input. Video-file soundtracks
  and embedded capture audio are not decoded by the video path. The card's
  separate audio input can be selected for modulation. Automatic BPM detection
  is not implemented; tap tempo and external clock sources are available.
- The local app uses `/opt/homebrew/opt/ffmpeg` dynamic libraries. It is ad-hoc
  signed, not notarized or self-contained for another machine. Distribution
  packaging/licensing gates remain in the release plan.

## Build identity

Artifacts:

- `target/release/virtual`
- `target/release/VIRTUAL.app`
- `target/release/VIRTUAL.sha256`
- `target/release/VIRTUAL-dynamic-libraries.txt`
- `target/release/event-review-2026-09-25/` — validation logs, benchmark JSON and
  a source snapshot for this working-tree build.

Native executable SHA-256:
`5be4f8c43af4930f0f657060d74dca762e3b0aee26266bae5054f76c360de5e3`

Ad-hoc-signed bundle executable SHA-256:
`2e60289be6d5538f9ba9663cfdeabbc5fb450cc8d106c5e18c8b678ce9b2a457`

Signing changes the bundled executable's hash. Rebuilding the bundle after
changing resources also changes its signature/hash; `VIRTUAL.sha256` records
the generated artifact. Source changes are local and have not been pushed.
