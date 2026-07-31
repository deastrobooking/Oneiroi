# Effect package authoring

Oneiroi discovers custom one- or two-pass master effects from immediate child
directories under `effects/`. Each directory contains `effect.json` and a WGSL
file referenced by that manifest.

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

`master_effect` registers a selectable custom slot effect. `passes` may contain
one or two fragment entry points. When omitted, the legacy `fragment_entry`
declares a one-pass effect.
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

Choose **Refresh registry** after adding a package. Registered manifests and
WGSL are then fingerprinted every 500 ms. Naga validates syntax and entry
points, and wgpu validates the fixed bind-group/pipeline contract on a worker.
A successful candidate replaces that package pipeline between frames.

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
