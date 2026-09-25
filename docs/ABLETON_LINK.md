# Ableton Link

VIRTUAL uses rusty_link 0.4.9, wrapping Ableton's official implementation.
Build with CMake 3.14+, a C++ compiler and libclang available, in addition to
the existing Rust and FFmpeg requirements:

```sh
cargo build --release -p virtual-app
```

In the MIDI panel, expand **Clock sync** and select **Ableton Link**.
Networking is disabled until selected. Peer count is shown beside the selector.
Internal and MIDI modes disconnect Link. Source selection is saved in projects;
older projects still default to Internal.

Link supplies tempo and beat/bar phase to existing quantized launches and
modulation. BPM edits and tap tempo are shared with peers. Incoming MIDI clock
does not override Link. Supported session tempos are 20–400 BPM; outside that
range the status reports the limitation and the local clock retains its last
supported timing. Optional MIDI output follows tempo, but its pulse phase is
not locked to Link.

Large beat-position jumps from joining or changing an external timeline move
queued launches to the next beat/bar boundary on that timeline. Small phase
corrections retain their targets. A zero peer count reports waiting for peers.

This is tempo/phase integration, not transport integration: Link start/stop
sync is not enabled and peer transport changes do not start or stop decks.
Application-thread state capture is not a hard-realtime operation. No Link
calls are made from audio callbacks. Peer tempo changes are journaled; continuous
phase corrections are not recorded for exact network-session replay.

## Validation before a show

- Run two instances on the same LAN and select Link; verify each sees a peer.
- Change tempo from each side; verify both tempo and four-beat phase agree.
- Queue a next-beat/bar clip launch; repeat with a slow rendering frame rate.
- Disconnect/reconnect a peer and switch Internal/MIDI/Link while playing.
- Save/reload a Link project and check source selection and peer discovery.
- Verify with Ableton Live and measure visual output latency on the show rig.

Automated tests cover local Link tempo capture/commit and mapping its clock
to the application's launch clock. Network discovery, real-device phase
alignment and GUI operation require the above manual checks.

Next: opt-in transport synchronization, bounded off-thread capture if frame
profiling warrants it, and an SDK-free build feature.
