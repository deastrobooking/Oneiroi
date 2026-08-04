# Shader system

This document is the canonical description and upgrade plan for Oneiroi's
shader runtime. `EFFECT_PACKAGES.md` remains the authoring reference for the
currently executable package ABI; `ROADMAP.md` owns the wider product order.

## Executive decision

Oneiroi will keep WGSL, `wgpu` and Naga as its native shader stack. The next
shader milestone is a bounded per-deck package stage, not compute, arbitrary
pass graphs, HDR or shader-format import. Those later capabilities build on the
same typed resource and scheduling boundary, but landing them together would
make GPU cost, memory use and failure behavior unpredictable during a show.

The accepted layer order is:

```text
source decode
    -> layer transform + crop + Fit / Fill / Stretch source mapping
    -> built-in Geometry UV stage
    -> Color + Levels / Stylize + Key in their relative order
    -> planned deck package slot
    -> layer blend into Bus A or Bus B
    -> bus crossfade
    -> two package-capable master slots
    -> final sRGB presentation
```

A **layer** is the processed image from one deck immediately before it enters
its bus blend. The deck package stage therefore affects only its own layer and
executes before Difference, Screen, Multiply and every other layer blend mode.

## Current system

| Area | Current contract |
|---|---|
| Native language | WGSL parsed by Naga and compiled by `wgpu` 29 |
| Working color | Linear-sRGB shader math in RGBA8-sRGB render targets; final presentation remains sRGB |
| Built-in deck effects | Geometry is a fixed UV prepass; Color + Levels and Stylize + Key can exchange relative order inside the fused compositor |
| Package placement | Master slots only |
| Package sequence | One or two fragment passes; this is a bounded sequence, not a DAG |
| Package ABI | Manifest v1 implicitly targets `master-v1`; manifest v2 validates explicit target/ABI metadata while deck candidates remain catalog-only |
| Temporal state | At most one fixed previous-slot-output history texture per physical master slot |
| Reload | Background parse/compile, generation-tagged swap and last-known-good retention |
| Discovery | Bundled, active-workspace, configured and user roots with deterministic duplicate handling |
| Algorithmic packages | Recursive 2D Lab, Fractal Volume 3D and Hyper Recursion 4D+; currently master-only |

The four logical deck-effect nodes in the typed graph still lower into the
single optimized compositor pass. They describe the built-in groups above;
they do not yet represent independently executable effect packages.

## Realtime invariants and the remaining S0 gap

Automatic watched-package reload already performs file reads, shader parsing
and pipeline creation on its worker. Manual **Refresh registry** still performs
one synchronous discovery and Naga-validation scan on the application thread;
moving that scan to a bounded, generation-tagged worker is an explicit S0 gate
before user and per-deck catalogs are allowed to scale.

Every shader upgrade must converge on and preserve these rules:

- No shader parse, filesystem read, pipeline creation or resource allocation
  occurs on the presentation path.
- A rejected package generation cannot replace a working pipeline or blank the
  program output.
- Dry, bypassed and missing-package behavior is a neutral pass-through.
- GPU passes, texture counts, formats and history resources have explicit hard
  ceilings before a graph is accepted.
- Inactive and invisible deck branches are culled before package work.
- Package identity, parameter identity and saved automation never depend on
  declaration order.
- Alpha and color-space contracts are tested at every new pass boundary.
- Final presentation remains sRGB even when a future internal HDR tier is
  selected.

## Upgrade sequence

### S0 — Contract and tooling foundation

Status: target-aware catalog foundation implemented; asynchronous registry
discovery and the layout-tooling spike remain.

1. Keep one canonical shader roadmap and use consistent terminology across the
   repository.
2. Add manifest-v2 target/ABI metadata that distinguishes master and deck
   packages without making deck execution available prematurely. (implemented)
3. Partition discovery by target so a deck-only package is never compiled
   against the `master-v1` layout. (implemented)
4. Retain Rust/WGSL offset, size, entry-point and real-GPU conformance tests.
5. Add a documented `wgsl-analyzer` editor recommendation.
6. Run a focused spike comparing generated Rust/WGSL bindings with `encase`;
   choose one only if it improves the next ABI without introducing a second
   incompatible Naga or `wgpu` line.
7. Move manual registry discovery and schema/WGSL validation to a bounded,
   generation-tagged worker. Publish only the newest completed catalog and
   keep the prior catalog visible while a scan is running.

Acceptance:

- Manifest v1 packages remain source- and project-compatible.
- Target metadata is validated, discoverable and excluded from unsupported
  execution paths.
- Manual registry refresh cannot stall presentation and rejects stale scan
  generations.
- Full tests and strict Clippy remain clean.

### S1 — Extract the per-deck precomposition seam

Status: committed next.

Split deck image production from bus blending while preserving the existing
fused path when no deck package is active:

```text
inactive fast path: source + built-ins + blend (current fused pass)

active package path:
source + built-ins -> materialized layer -> package -> blend-only compositor
```

The branch split must be selective: one active package must not force package
passes for the other three decks. Fixed layer targets and one reusable scratch
target are budgeted at composition resize or graph preparation, never inside a
frame. Timing is recorded per extracted branch and package pass before the
path becomes operator-selectable.

Acceptance:

- With no package active, GPU output is bit-identical to the current fused
  compositor.
- A package branch preserves transparent pixels and straight/premultiplied
  alpha semantics through the final blend.
- Solo, bypass, zero level and inactive buses skip unnecessary package work.
- No per-frame allocation or pipeline creation occurs.

### S2 — One `deck-v1` package slot per deck

Status: planned after S1 measurements.

Ship one bounded fragment-package slot after the built-in deck groups and
before layer blending. The first release supports one pass and no temporal
history; two-pass support is enabled only if the measured shared-scratch design
meets the show-machine budget.

The slice includes:

- Project persistence and migration with stable package/parameter IDs
- Bypass, wet/dry, reset, looks and missing-package neutral fallback
- Last-known-good reload and target-specific diagnostics
- Deck LFO, audio/beat modulation, MIDI and OSC identities
- Explicit deck index, source extent, composition extent, time and alpha
  semantics in the `deck-v1` ABI
- Show Mode identity/bypass/wet controls

Acceptance:

- Dry and bypass are bit-exact neutral paths.
- A deck package visibly executes before every non-Normal layer blend.
- Transparent input stays transparent unless the package explicitly changes
  alpha under the ABI contract.
- Four active 1080p package branches and the UHD memory ceiling are recorded in
  `RELEASE_CHECKLIST.md` before release.

### S3 — Versioned shared WGSL modules

Status: planned.

Prototype a versioned `oneiroi-std` namespace for color conversion, blend
functions, hashes/noise, SDF/raymarch helpers and common fullscreen bindings.
`naga-oil` is a candidate composer, not a dependency decision. Any composer
must use an allow-listed module namespace, produce useful source-mapped errors
and include every transitive module in package fingerprints and hot-reload
watching.

The Rust/WGSL layout spike from S0 resolves here: adopt either generated
bindings or `encase` packing where it demonstrably reduces ABI drift. Do not
maintain two competing layout systems.

### S4 — Typed N-pass fragment package graph

Status: planned.

Replace the one/two-pass sequence with a small typed acyclic graph whose
manifest declares pass inputs, outputs, resolution scale, format, lifetime and
fallback. The existing graph compiler must reject cycles without temporal
breaks, over-budget texture lifetimes and unsupported formats before the
renderer sees the candidate. Bloom pyramids and separable filters belong here.

This is the first point where the term **package graph** applies. The current
one/two-pass master implementation remains a bounded pass sequence.

### S5 — Capability-gated HDR intermediates

Status: planned after S4 budgeting.

Add an optional RGBA16Float working tier for intermediate layer, package,
history and master targets. This roughly doubles their byte cost compared with
RGBA8 and therefore requires adapter capability checks, explicit 1080p/UHD
memory estimates, deterministic SDR fallback and readback tests for highlight
retention, alpha and final sRGB encoding. HDR here means internal processing;
HDR display signaling is a separate output project.

### S6 — Compute and stateful simulations

Status: later.

Add compute only through typed storage resources, fixed dispatch ceilings and
declared history/state lifetimes. Candidate workloads include particles,
reaction-diffusion, cellular automata and fluids. Audio FFT already runs on a
bounded worker and feeds modulation; moving it to compute is not a prerequisite
for visual compute effects.

### S7 — Offline ISF and ShaderToy import

Status: later.

Naga's GLSL frontend is only a parser; it does not provide ISF metadata,
ShaderToy uniforms/channels, GLSL dialect normalization, resource policy or
license provenance. Import will therefore be an offline conversion and
validation tool that emits a normal Oneiroi package. Implement ISF first, then
a narrower ShaderToy adapter. Runtime loading of arbitrary GLSL is not part of
the initial design.

## Tooling decisions from the review

| Proposal | Decision |
|---|---|
| `wgsl-analyzer` | Adopt as a documented, optional authoring aid now |
| `naga-oil` | Prototype in S3 after the deck ABI is stable |
| `wgsl_bindgen` / `wgsl_to_wgpu` | Evaluate against the exact `wgpu`/Naga 29 line; do not adopt without a compatibility and error-quality spike |
| `encase` | Evaluate as the alternative to code generation, especially for dynamic storage layouts |
| Compute passes | Defer until typed resource budgets and deck extraction exist |
| Arbitrary N-pass/DAG | Build as S4 through the graph compiler, not by raising the current pass-count constant |
| RGBA16Float | Add as an optional measured tier, not an unconditional format replacement |
| ISF / ShaderToy | Build offline adapters after the native package graph is mature |

## Memory gates

The current master SDR target owns seven full-resolution RGBA8-sRGB textures.
Its additional capacity beyond the direct final target is approximately
47.5 MiB at 1920×1080 and 189.8 MiB at 3840×2160.

S1 must publish the incremental cost of its fixed deck-layer and scratch
targets. S5 must publish both SDR and RGBA16Float totals. These figures are
acceptance gates, not estimates hidden in implementation notes.

## Out of scope for the deck-package milestone

- Compute and arbitrary storage buffers
- Package-owned unbounded textures
- More than one custom package slot per deck
- Temporal deck history
- ISF or ShaderToy runtime loading
- HDR display metadata or transfer-function negotiation
- Replacing the current master package ABI
