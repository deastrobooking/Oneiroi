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

fn effect_offset() -> vec2<f32> {
    let spread = globals.parameters[0].x;
    let rotation = globals.parameters[0].z;
    let pulse = 0.75 + 0.25 * sin(globals.time_seconds * 2.0);
    return vec2(cos(rotation), sin(rotation)) * spread * pulse;
}

@fragment
fn fs_extract(input: VertexOutput) -> @location(0) vec4<f32> {
    let offset = effect_offset();
    let a = textureSample(original_texture, effect_sampler, input.uv + offset);
    let b = textureSample(original_texture, effect_sampler, input.uv - offset);
    return vec4(a.r, (a.g + b.g) * 0.5, b.b, max(a.a, b.a));
}

@fragment
fn fs_combine(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSample(original_texture, effect_sampler, input.uv);
    let offset = effect_offset() * 0.5;
    let echo_amount = globals.parameters[0].y;
    let forward = textureSample(effect_texture, effect_sampler, input.uv + offset);
    let backward = textureSample(effect_texture, effect_sampler, input.uv - offset);
    let echo_color = mix(forward, backward, 0.5);
    let effected = mix(original, echo_color, clamp(echo_amount, 0.0, 1.0));
    return mix(original, effected, clamp(globals.mix_amount, 0.0, 1.0));
}
