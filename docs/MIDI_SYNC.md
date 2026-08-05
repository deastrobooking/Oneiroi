# MIDI beat-clock sync

Oneiroi follows an external MIDI beat clock and generates one for other gear.
Both directions live in **Setup → MIDI control → Clock sync**.

The clock is the standard 24 pulses per quarter note (PPQN) beat clock, plus
Start (`0xFA`), Continue (`0xFB`), Stop (`0xFC`) and Song Position Pointer
(`0xF2`). Sync messages never take part in MIDI learn, so a device may both
clock the show and drive mapped controls.

## Following an external clock

1. Connect the source under **Input** as usual — a clock source is an ordinary
   MIDI input device.
2. Set **Tempo from** to **MIDI clock in**.
3. Optionally pin **From** to one device. Left on *Any connected device*, the
   first device that clocks becomes the master and keeps it until it goes quiet
   for half a second, so two clock sources on one rig cannot fight.

While locked the panel shows the master device, the measured tempo and whether
its transport is running. The BPM field, **Tap**, **½** and **×2** are disabled:
the incoming clock owns the tempo, and anything typed there would be
overwritten by the next pulse.

What the incoming clock drives:

| Message | Effect |
|---|---|
| Clock (`0xF8`) | Tempo estimate; beat phase is re-anchored every quarter note |
| Start (`0xFA`) | Musical position rewinds to beat 0 |
| Continue (`0xFB`) | Position resumes where it stopped |
| Stop (`0xFC`) | Position stops advancing; tempo tracking continues |
| Song Position (`0xF2`) | Position jumps to the addressed sixteenth note |

Tempo comes from the mean pulse interval over a quarter note rather than a
single interval — one interval carries several milliseconds of USB jitter,
which at 24 PPQN reads as tens of BPM. Intervals that could not belong to a
20–400 BPM clock are treated as a dropout: the window is discarded and the
estimate rebuilds rather than being dragged somewhere impossible.

Phase is re-anchored on quarter-note boundaries, not just tempo, so a followed
show cannot drift a bar away from its master over a long set. Everything that
reads musical time follows with it: quantized clip and scene launches, beat and
bar modulation sources, and beat-synced LFOs.

Intervals are measured with the driver's own packet timestamps, not with poll
time. Sampling pulses at frame rate would quantise them to ~16 ms and make a
followed tempo visibly wander.

The transport is *not* driven: Start and Stop move musical position and are
reported in the panel, but they do not start or stop decks. Deck transports
stay under operator and mapping control.

## Sending clock downstream

1. Pick a destination under **Clock out** and press **Connect**.
2. Tick **Send clock**. Oneiroi sends Start, then 24 PPQN pulses at the show
   tempo, until the box is cleared — which sends Stop.
3. **Continue** resumes downstream gear in place instead of rewinding it.

Pulses are emitted from a dedicated sender thread against its own schedule, not
from the render loop, for the same reason the follower uses driver timestamps: a
60 Hz frame cannot place a pulse every 20.8 ms, and frame-driven clock arrives
in clumps that downstream gear hears as swing.

Two protections matter on a show machine:

- A tempo change is measured from the last pulse already sent, so it never
  emits two pulses back to back or swallows one.
- If the sender loses the CPU for longer than a quarter second, the missed
  pulses are dropped and the schedule re-bases. A listener recovers from a gap;
  it does not recover from a burst. Each occurrence is counted as a resync.

The panel reports pulses, transport messages, late pulses (more than 2 ms
behind schedule), the worst lateness observed, resyncs and send errors. Output
ports are rescanned every two seconds; a destination that disappears drops the
sender and clears **Send clock** rather than leaving the panel claiming the show
is still clocking.

Following and sending compose: with **Tempo from** set to MIDI clock in and
**Send clock** on, Oneiroi re-clocks its own tempo downstream, which is the
usual way to put gear without a clock input behind a master that has one.

## What is saved

A project stores the clock source, the pinned input device, the output device
and whether clock was being sent. On load the destination is re-opened and
resumes sending if it was sending when the project was saved; missing hardware
is reported in the panel rather than retried silently, because unlike a
controller, a missing clock destination changes what downstream gear does.

Projects written before clock sync existed load with the internal tempo clock
selected and nothing sending.

## Related

- [Operator guide](OPERATOR_GUIDE.md) — tempo, quantization and controllers
- [OSC](OSC.md) — remote tempo and transport control
