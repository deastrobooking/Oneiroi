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

@group(0) @binding(0) var effect_sampler: sampler;
@group(0) @binding(1) var original_texture: texture_2d<f32>;
@group(0) @binding(2) var effect_texture: texture_2d<f32>;
@group(0) @binding(3) var<uniform> globals: MasterEffectGlobals;
@group(0) @binding(4) var history_texture: texture_2d<f32>;
@group(0) @binding(5) var custom_history_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array(
        vec2(-1.0, -1.0),
        vec2( 3.0, -1.0),
        vec2(-1.0,  3.0),
    );
    let uvs = array(
        vec2(0.0, 1.0),
        vec2(2.0, 1.0),
        vec2(0.0, -1.0),
    );
    var output: VertexOutput;
    output.position = vec4(positions[index], 0.0, 1.0);
    output.uv = uvs[index];
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSample(original_texture, effect_sampler, input.uv);
    if globals.history_valid == 0u {
        return original;
    }
    let persistence = globals.parameters[0].x;
    let drift = globals.parameters[0].y;
    let bleed = globals.parameters[0].z;
    let history_uv = input.uv + vec2(0.0, drift);
    let history = textureSample(custom_history_texture, effect_sampler, history_uv);
    let red = textureSample(
        custom_history_texture,
        effect_sampler,
        history_uv + vec2(bleed, 0.0),
    ).r;
    let blue = textureSample(
        custom_history_texture,
        effect_sampler,
        history_uv - vec2(bleed, 0.0),
    ).b;
    let melted = mix(original, vec4(red, history.g, blue, history.a), persistence);
    return mix(original, melted, clamp(globals.mix_amount, 0.0, 1.0));
}
