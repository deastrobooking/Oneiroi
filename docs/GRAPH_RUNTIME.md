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
- Clip launches
- Scene launches
- Tap, half-time and double-time tempo changes
- Program-output enable changes
- Future graph transaction commits

A complete state checkpoint is retained every 600 rendered frames. Replay can
start from the latest checkpoint before a target time and deterministically
apply the remaining commands.

## Deliberate limitations of this slice

- Deck source and deck-effect nodes are fused into the existing compositor;
  they are not independently executable passes yet.
- Node kinds beyond the compatibility graph do not yet have GPU executors.
- Continuous UI parameters, MIDI/OSC input and transport operations do not all
  route through `ShowCommand` yet.
- Event logs are serializable in memory but are not yet streamed to a
  crash-safe on-disk journal.
- Graphs and takes are not yet part of `.oneiroi` project version 3.
- Shadow-graph editing and commit controls do not yet have operator UI.
- Color and resolution declarations are carried into the plan; full
  edge-by-edge inference and conversion-node insertion remain.

These boundaries preserve project compatibility and the existing live output
while the graph becomes executable one node family at a time.

## Next implementation slice

1. Route all operator and controller mutations through `ShowCommand`.
2. Add an append-only journal writer with periodic atomic checkpoints.
3. Persist graphs, take metadata and deterministic seeds in a backward-
   compatible project migration.
4. Add the shadow-graph editor and preview/commit controls.
5. Add executors for explicit delay/rate-adapter nodes, then unfuse deck
   branches where graph editing requires independent passes.
