# Graph and session runtime

Oneiroi now has the first control-plane slice needed to grow beyond a fixed
layer mixer without destabilizing the proven renderer.

## Current boundary

The Perform view still executes through the existing bounded
`FourDeckCompositor`. At startup, the same topology is also represented and
compiled as this compatibility macro:

```text
Deck A source -> Deck A effects --\
Deck B source -> Deck B effects ----\
Deck C source -> Deck C effects -----+-> four-deck mixer
Deck D source -> Deck D effects ----/          |
                                                v
                                        master effects
                                                |
                                                v
                                         program output
```

That produces an immutable 11-node `RenderPlan`. `oneiroi-render` now lowers
that logical graph into three executable stages:

```text
four source/effect branches -> fused FourDeckComposite
                            -> MasterEffects
                            -> ProgramOutput
```

The application iterates this lowered schedule to encode composition, master
effects and presentation. The plan is therefore authoritative for live stage
ordering while the fixed compositor remains the optimized executor for the
four deck branches. This is deliberate pass fusion, not a parallel rendering
path.

## Typed graph

`oneiroi-graph` owns:

- Stable node, graph-revision and schema identities
- Typed ports for textures, masks, depth, motion, audio, spectra, geometry,
  particles, events, control values, camera poses and skeletons
- Audio, control, video, event, async CPU and external rate domains
- Versioned node contracts with latency, determinism, state, realtime-safety,
  permission, quality, fallback, color and resolution declarations
- Validation for contracts, ports, types, required inputs and single-writer
  inputs
- Cycle rejection unless the loop crosses an explicit temporal-break node
- Explicit rate-adapter records for cross-domain connections
- Deterministic topological pass ordering
- Conservative GPU-cost and transient-texture budget rejection
- Texture lifetime analysis and non-overlapping transient slot reuse

Compiler output is immutable and cheaply cloneable. It contains no GPU, media
or operating-system handles. The plan retains compiled connections so a
renderer can verify topology before selecting executors.

## Renderer lowering

The initial lowering accepts only the exact built-in compatibility topology.
It rejects:

- Unknown node kinds without a renderer executor
- Rate adapters that the renderer cannot execute yet
- Missing or duplicated mixer, master or output nodes
- Shared branches that cannot represent four independent decks
- Non-linear working color
- Extent disagreement between mixer, master and output

Composition-size changes recompile and lower the graph before reallocating
program textures. A plan that exceeds its texture budget leaves the current
extent active. If a committed graph cannot be lowered, the runtime restores
the previous graph and immutable plan.

## Live transactions

`TransactionManager` keeps the active graph and plan untouched while a cloned
shadow graph is edited. Preparation validates and compiles the complete shadow
graph. A failed preparation leaves the active and last-known-good plans
unchanged.

A ready plan can be scheduled for:

- Next frame
- Next beat
- Next bar
- An exact frame
- An exact beat tick
- An exact timecode frame

The swap happens only when the main-thread timeline reaches the target.
Transactions can also be discarded before commit.

## Commands, checkpoints and takes

`oneiroi-session` defines serializable `ShowCommand`, `ShowTime`,
`SessionState`, `StateCheckpoint`, `PerformanceTake` and `SessionEventLog`
types. The event log is append-only: alternate decisions create a branch or a
new named take instead of rewriting recorded commands.

The app currently records:

- Initial graph activation
- Clip/scene launches, clip clears, deck ejects and seeks
- Tap, manual, half-time and double-time tempo changes
- Program-output enable, fullscreen, display and accepted extent changes
- All device-neutral MIDI `ControlTarget` updates
- Keyboard blackout, freeze, crossfader, output and scene commands
- UI mixer, deck transport, effect, LFO, modulation-matrix and custom-effect
  parameter changes
- Camera source assignments
- Future graph transaction commits

## Authoritative control gateway

MIDI and keyboard updates enter a single application gateway. The gateway:

1. Removes release edges that do not mutate state.
2. Converts launches and blackout to semantic command variants.
3. Records and journals the command with its origin and show time.
4. Applies the concrete mixer, transport or effect mutation.

For continuous egui widgets, the app snapshots 192 built-in performance
targets plus dynamic custom-effect parameters before drawing. Changed values
are restored to their prior value and then reapplied through the gateway. This
keeps mouse drags on the same command path as MIDI and keyboard input rather
than merely logging already-mutated values.

Editor-owned structures use the same record-before-apply rule through a
structural snapshot around each UI frame. Deterministic field commands now
cover deck bus/equal-power selection, transforms and crop/source modes, blend,
solo/bypass, effect-slot order/group/mix/bypass, mirror, LFO waveform/sync and
direct routing, deck modulation sources/targets, the full master-effect chain,
and master modulation. Successful movie imports, folder imports and relinks
also record the accepted clip-slot media identity.

A complete state checkpoint is retained every 600 rendered frames. Replay can
start from the latest checkpoint before a target time and deterministically
apply the remaining commands.

## Crash-safe session journal

The in-memory take now feeds a bounded 4,096-record background writer. The
render thread only performs `try_send`; file serialization, writes and syncing
run on `oneiroi-session-journal`.

Each run creates:

```text
.oneiroi/session/session-<process>-<time>.jsonl
.oneiroi/session/session-<process>-<time>.checkpoint.json
```

The JSONL stream begins with a versioned format header and contains tagged
command and checkpoint records. Checkpoints:

1. Enter the journal in command order.
2. Sync preceding journal data.
3. Serialize to a temporary checkpoint file.
4. Flush and sync the temporary file.
5. Atomically rename it over the prior checkpoint.

Recovery loads the atomic checkpoint and returns only later commands. A torn,
non-newline-terminated final journal record is ignored; malformed complete
records, bad formats, unsupported versions and non-monotonic sequences are
hard errors.

The operator's **Session recovery** panel scans prior journals while excluding
the active writer. It shows the take name, reconstructed command count, latest
show time, checkpoint availability and torn-tail status. Restoring validates
and replays checkpoint plus tail, applies continuous and structural state to
the concrete mixer, then opens a fresh journal with an immediate baseline
checkpoint so command sequences restart monotonically.

Recovery also retains the complete validated command/checkpoint history for
operator timeline replay. The cursor selects the latest checkpoint at or
before its show time, applies only the following commands through that time,
and restores the result as a new named branch. Latest-state crash recovery
continues to use the smaller atomic-checkpoint tail.

Labeled marker records share the journal timeline without consuming command
sequence numbers. Marker buttons move the recovery cursor to exact show time.
Project take entries can be exported to a chosen directory or archived under
`.oneiroi/archive`; each action creates a unique directory containing copied
journal and checkpoint files and never moves, overwrites or deletes the live
source bundle.

Project schema v5 assigns a stable 128-bit identity and stores bounded take
metadata. New journal headers carry the project and take identities. Recovery
shows matching and legacy/unlinked journals, while linked journals belonging
to other projects are hidden.

Journal command/checkpoint counts, queue overruns and the last worker error are
shown beside graph health in the operator window. A journal failure never
blocks or stops program output; the in-memory take remains available.

## Deliberate limitations of this slice

- Deck source and deck-effect nodes are fused into the existing compositor;
  they are not independently executable passes yet.
- Node kinds beyond the compatibility graph do not yet have GPU executors.
- OSC input reaches the same command gateway as operator, keyboard and MIDI
  control. OSC output/feedback and timetag scheduling are not yet connected;
  project open/restore is intentionally treated as baseline state rather than
  live `ShowCommand` traffic.
- Recovery still assumes the matching project is loaded so clip indices map to
  the same media. Journals identify the project but do not embed its asset
  manifest, and camera/media history is not used to guess a missing baseline.
- Project v5 persists the active typed graph and scoped deterministic seeds.
  Persisted graphs must compile, lower and satisfy the active extent budget
  before the project is accepted. Command logs remain external journal files.
- Shadow-graph editing and commit controls do not yet have operator UI.
- Color and resolution declarations are carried into the plan; full
  edge-by-edge inference and conversion-node insertion remain.

These boundaries preserve project compatibility and the existing live output
while the graph becomes executable one node family at a time.

## Next implementation slice

1. Add OSC output feedback and optional bundle-timetag scheduling.
2. Add marker editing plus portable project/media manifests for take bundles.
3. Add the shadow-graph editor and preview/commit controls.
4. Add the shadow-graph editor and preview/commit controls.
5. Add executors for explicit delay/rate-adapter nodes, then unfuse deck
   branches where graph editing requires independent passes.
