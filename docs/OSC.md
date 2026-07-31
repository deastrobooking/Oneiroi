# OSC transport

Oneiroi accepts bounded OSC 1.0 input over UDP. Open **OSC input**, enter a
local bind address such as `0.0.0.0:9000`, then choose **Listen**. Use
`127.0.0.1:9000` when only software on the same machine should connect.

Incoming messages are decoded on a background thread and delivered through a
fixed 256-message queue. The render loop never waits for UDP input. Packet,
decoded-message, malformed-packet and queue-drop counters remain visible in
the operator panel.

Set a **Feedback target** such as `127.0.0.1:9001` and choose **Send
feedback** to publish accepted state changes. Output uses a separate fixed
256-message worker queue. Connecting sends an initial snapshot of master,
deck, tempo and output values; sent, dropped and socket-error counters remain
visible.

Every accepted message enters the same command gateway as UI, keyboard and
MIDI control. Journal records retain the sender socket address as their OSC
origin, so recovery and timeline replay remain deterministic.

## Routes

Deck, clip and scene numbers in OSC addresses are one-based.

| Address | Argument | Result |
|---|---:|---|
| `/vjx/crossfader` | float 0–1 | A/B crossfader |
| `/vjx/master/opacity` | float 0–1 | Master opacity |
| `/vjx/master/blackout` | bool or 0/1 | Master blackout |
| `/vjx/master/freeze` | bool or 0/1 | Master freeze |
| `/vjx/tempo` | float 20–400 | Set BPM |
| `/vjx/output/enabled` | bool or 0/1 | Show/hide clean output |
| `/vjx/output/fullscreen` | bool or 0/1 | Toggle output fullscreen |
| `/vjx/deck/{1-4}/level` | float 0–1 | Deck opacity |
| `/vjx/deck/{1-4}/play` | bool or 0/1 | Play/pause deck |
| `/vjx/deck/{1-4}/freeze` | bool or 0/1 | Freeze deck |
| `/vjx/deck/{1-4}/speed` | float 0.25–4 | Playback speed |
| `/vjx/deck/{1-4}/select` | optional trigger | Select deck |
| `/vjx/deck/{1-4}/restart` | optional trigger | Restart deck |
| `/vjx/deck/{1-4}/clip/{1-8}/launch` | optional trigger | Launch clip |
| `/vjx/scene/{1-8}/launch` | optional trigger | Launch scene |

Trigger routes default to `1` when sent without arguments. Sending a trigger
value below `0.5` is treated as a release edge and does not mutate or journal
state. OSC integer, float, double and boolean arguments are accepted; string
arguments are decoded but are not currently mapped to controls.

Standard OSC bundles are accepted, including nested bundles up to eight
levels. NTP timetags are converted to monotonic deadlines and executed on the
first render frame at or after the deadline. Immediate and past timetags run
on receipt. The pending queue is capped at 1,024 messages and the scheduling
horizon at 24 hours; rejected future messages increment the schedule-drop
counter. Journal `ShowTime` records actual execution rather than arrival.

Feedback uses the same addresses listed above with one float argument. Trigger
routes emit `1`; continuous and boolean routes emit their accepted concrete
state. Feedback is state notification, not a guaranteed-delivery protocol.

## Security boundary

OSC UDP has no authentication. Bind to loopback or isolate the show-control
network when untrusted hosts are present. Disconnecting OSC input clears
pending scheduled messages, closes the socket and joins its worker without
affecting program output.
