# Effect package authoring

Oneiroi discovers custom one-pass master effects from immediate child
directories under `effects/`. Each directory contains `effect.json` and a WGSL
file referenced by that manifest.

## Manifest

```json
{
  "format": "oneiroi-effect",
  "version": 1,
  "id": "chromatic-split",
  "name": "Chromatic split",
  "role": "master_effect",
  "shader": "chromatic_split.wgsl",
  "vertex_entry": "vs_main",
  "fragment_entry": "fs_main",
  "parameters": [
    {
      "id": "amount",
      "label": "Amount",
      "minimum": 0.0,
      "maximum": 0.08,
      "default": 0.012
    }
  ]
}
```

IDs contain lowercase ASCII letters, digits and hyphens. Shader paths are
package-relative `.wgsl` paths without traversal. A package declares 1–32
unique finite parameter ranges. Parameter IDs are persisted in projects, while
the shader receives values in manifest declaration order.

`master_effect` registers a selectable custom slot effect.
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
    _padding: vec2<u32>,
    parameters: array<vec4<f32>, 8>,
}
```

Scalar parameter `i` is read from
`globals.parameters[i / 4][i % 4]`. `mix_amount` is the slot wet control.
`time_seconds` is monotonic performance time. A custom shader normally samples
`original_texture`, computes its effect and applies the common wet mix.

## Lifecycle and safety

Choose **Refresh registry** after adding a package. Registered manifests and
WGSL are then fingerprinted every 500 ms. Naga validates syntax and entry
points, and wgpu validates the fixed bind-group/pipeline contract on a worker.
A successful candidate replaces that package pipeline between frames.

Invalid edits never replace the last working pipeline. A package that is
missing or has never compiled uses the built-in neutral copy pass, so loading
an unavailable project effect cannot blank program output.

Custom packages currently receive one pass in either of the two master slots.
They cannot request extra textures or declarative multipass graphs.
