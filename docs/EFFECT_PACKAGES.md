# Effect package authoring (`master-v1`)

This document describes the shader contract that executes today: `master-v1`,
placed in either of the two master slots. Manifest v1 implies target `master`
and ABI `master-v1`; manifest v2 must state those values explicitly. The runtime
supports one or two fragment passes and one optional fixed previous-slot
history resource. It is not yet a per-deck, compute or arbitrary-DAG contract.

Oneiroi discovers these packages from immediate child directories under every
resolved effect resource root. Each package directory contains `effect.json`
and a WGSL file referenced by that manifest.

The shipped root is resolved independently of the process launch directory:
the source workspace is used during development, `Contents/Resources/effects`
inside a macOS app bundle, or an `effects` directory beside a release binary.
An existing `effects` directory in the active show workspace is scanned too.
`ONEIROI_EFFECT_PATH` adds one or more platform-separated roots for a custom
rig. User packages live in
`~/Library/Application Support/Oneiroi/effects` on macOS, or under
`$XDG_DATA_HOME/oneiroi/effects` (falling back to
`~/.local/share/oneiroi/effects`) on other platforms.

Roots are evaluated in this order:

1. Trusted bundled/development resources
2. The active show workspace
3. `ONEIROI_EFFECT_PATH` entries in platform path-list order
4. The per-user effect directory

The first valid package ID wins. A lower-priority duplicate is excluded and
reported in registry diagnostics; it never silently replaces a bundled
package. Invalid edits to a previously loaded manifest retain that manifest's
last-known-good descriptor and GPU pipeline until the exact file becomes valid
again, changes identity, or is removed.

## Manifest

```json
{
  "format": "oneiroi-effect",
  "version": 1,
  "id": "spectral-echo",
  "name": "Spectral echo",
  "role": "master_effect",
  "shader": "spectral_echo.wgsl",
  "vertex_entry": "vs_main",
  "fragment_entry": "fs_combine",
  "passes": [
    {
      "fragment_entry": "fs_extract"
    },
    {
      "fragment_entry": "fs_combine"
    }
  ],
  "parameters": [
    {
      "id": "spread",
      "label": "Spread",
      "minimum": 0.0,
      "maximum": 0.08,
      "default": 0.018
    }
  ]
}
```

IDs contain lowercase ASCII letters, digits and hyphens. Shader paths are
package-relative `.wgsl` paths without traversal. A package declares 1–32
unique finite parameter ranges. Parameter IDs are persisted in projects, while
the shader receives values in manifest declaration order.

### Extended controls and looks

Parameters remain sliders by default. Packages can opt into a richer generated
interface without adding app-specific code:

```json
{
  "id": "function",
  "label": "Function",
  "group": "Recursive function",
  "minimum": 0.0,
  "maximum": 2.0,
  "default": 0.0,
  "control": "choice",
  "options": [
    {"label": "Mirror IFS", "value": 0.0},
    {"label": "Julia warp", "value": 1.0},
    {"label": "Möbius petals", "value": 2.0}
  ]
}
```

`control` supports `slider`, `toggle`, and `choice`. Contiguous parameters with
the same non-empty `group` receive a shared GUI heading. A package-level
`description` appears above the controls.

Package-level `presets` provide one-click looks. Each preset has a stable ID,
label, optional description, and a partial `values` object keyed by parameter
ID. Unlisted controls retain their current value, which makes a preset useful
as either a complete look or a focused transform. The GUI also provides a
reset button that restores every manifest default.

`master_effect` registers a selectable custom master-slot effect. `passes` may
contain one or two fragment entry points. When omitted, the legacy
`fragment_entry` declares a one-pass effect.
`master_processor` is reserved for replacing the built-in blur/feedback
processor pipeline.

## `master-v1` bindings

Custom shaders must use group 0 with these bindings:

| Binding | WGSL resource |
|---|---|
| 0 | Filtering sampler |
| 1 | Original slot-input `texture_2d<f32>` |
| 2 | Intermediate effect `texture_2d<f32>` |
| 3 | `MasterEffectGlobals` uniform |
| 4 | Previous final-frame history `texture_2d<f32>` |
| 5 | Previous output for this custom slot `texture_2d<f32>` |

The uniform layout is:

```wgsl
struct MasterEffectGlobals {
    direction: vec2<f32>,
    texel_size: vec2<f32>,
    radius: f32,
    mix_amount: f32,
    mode: u32,
    feedback: f32,
    time_seconds: f32,
    parameter_count: u32,
    pass_index: u32,
    pass_count: u32,
    parameters: array<vec4<f32>, 8>,
    history_valid: u32,
}
```

Scalar parameter `i` is read from
`globals.parameters[i / 4][i % 4]`. `mix_amount` is the slot wet control.
`time_seconds` is monotonic performance time. A custom shader normally samples
`original_texture`, computes its effect and applies the common wet mix.
`history_valid` is appended after the original master-v1 parameter array, so
older shaders that omit binding 5 and this final field retain the same uniform
offsets and remain compatible.

## Bounded passes

A one-pass package reads `original_texture` and writes the slot output.

For a two-pass package:

1. Pass 0 reads the slot input from `original_texture` and writes the shared
   scratch texture.
2. Pass 1 reads the unchanged slot input from `original_texture`, reads pass
   zero through `effect_texture`, and writes the slot output.

Apply `mix_amount` in the final pass so the intermediate remains fully
available. `pass_index` is zero-based and `pass_count` is one or two. Every
declared entry point is compiled as one candidate set; any failure retains the
entire previous set. The cap is structural: both master slots reuse existing
scratch and ping targets, and no package texture is allocated during a frame.

## Modulation and MIDI

Every declared parameter automatically appears as a target in the master
modulation matrix and receives MIDI Learn/Clear buttons. Oneiroi derives target
identity from the package ID and parameter ID, so parameter declaration order
may change without redirecting saved routes or mappings.

Master modulation sources are three free-running or tempo-synchronized LFOs,
RMS, bass, mid, high, transient, beat phase and four-beat bar phase. Route
amount is bipolar and spans half of the declared parameter range at magnitude
1. Final values are clamped to the manifest minimum and maximum.

## Lifecycle and safety

Choose **Refresh registry** after adding a package. The button currently runs
one synchronous discovery plus schema/WGSL validation scan; moving that manual
scan to the bounded reload worker is the remaining S0 scaling gate. Once a
package is registered, its manifest and WGSL are fingerprinted every 500 ms.
Naga validates syntax and entry points, and wgpu validates the fixed
bind-group/pipeline contract on the worker. A successful candidate replaces
that package pipeline between frames.

Invalid edits never replace the last working pipeline. A package that is
missing or has never compiled uses the built-in neutral copy pass, so loading
an unavailable project effect cannot blank program output.

Custom packages currently receive one or two passes in either master slot.
They may request one fixed history resource:

```json
"resources": {
  "history": "previous_slot_output"
}
```

When declared, binding 5 contains the physical slot's previous successful
output. `history_valid` is zero on the first frame and after source, project,
resize, blackout, disable/bypass, missing-package or identity resets. A shader
must return a clean current-frame result while validity is zero. After the slot
renders, Oneiroi copies its output into history for the next frame. Freeze
retains history without evolving it.

One full-resolution RGBA8-sRGB history texture is reserved for each master
slot, so package selection cannot change memory use during a performance.
Packages cannot request arbitrary auxiliary texture counts, formats or sizes.

## Per-deck and package-graph roadmap

Per-deck packages are a committed upgrade, but they are deliberately separate
from the executable `master-v1` authoring contract above. The renderer must
first materialize only the affected deck branch after its built-in effects and
before layer blending, preserve the no-package fused fast path, define the
deck alpha contract and publish the fixed memory/GPU budget.

Manifest v2 target metadata is the compatibility foundation for distinguishing
master and deck placement. Target discovery does not make a deck package
executable: the first operator-facing deck release will use a dedicated
`deck-v1` ABI, one stateless fragment slot per deck and neutral fallback for an
unsupported or missing package.

The catalog-level v2 shape is:

```json
{
  "format": "oneiroi-effect",
  "version": 2,
  "id": "future-deck-effect",
  "name": "Future deck effect",
  "role": "effect",
  "targets": ["deck"],
  "abi": "deck-v1",
  "shader": "effect.wgsl",
  "parameters": [
    {
      "id": "amount",
      "label": "Amount",
      "minimum": 0.0,
      "maximum": 1.0,
      "default": 0.5
    }
  ]
}
```

Version 2 currently requires exactly one target. `deck-v1` candidates must be
stateless and one-pass. They are validated and reported separately from master
packages, but intentionally never offered to the master compiler or operator
deck UI until S1/S2 are complete. At this stage, deck candidates receive
manifest, WGSL syntax and entry-point validation; target-specific bind-group
and real-GPU validation arrives with the deck executor. A v2 master candidate
uses target `master` with ABI `master-v1`; manifest v1 continues to imply both
of those values.

Shared WGSL imports arrive later through a versioned, allow-listed module
namespace. When that happens, fingerprints and hot reload must include every
transitive imported module. Raising the current two-pass cap is not the graph
design; a future typed package graph will declare pass inputs, output formats,
resolution, resource lifetimes and hard budgets explicitly.

See [Shader system](SHADER_SYSTEM.md) for the ordered delivery plan and
acceptance gates.

## Editor support

The repository recommends the optional
[`wgsl-analyzer`](https://github.com/wgsl-analyzer/wgsl-analyzer) VS Code
extension for WGSL completion, navigation and diagnostics. Naga validation in
Oneiroi remains authoritative for package loading, and the real-GPU tests
remain authoritative for the `wgpu` binding/pipeline contract.
