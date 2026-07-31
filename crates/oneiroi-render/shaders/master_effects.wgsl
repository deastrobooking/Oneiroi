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

fn blur(uv: vec2<f32>) -> vec4<f32> {
    let step = globals.direction * globals.texel_size * (globals.radius / 4.0);
    var color = textureSample(effect_texture, effect_sampler, uv) * 0.227027;
    color += textureSample(effect_texture, effect_sampler, uv + step) * 0.1945946;
    color += textureSample(effect_texture, effect_sampler, uv - step) * 0.1945946;
    color += textureSample(effect_texture, effect_sampler, uv + step * 2.0) * 0.1216216;
    color += textureSample(effect_texture, effect_sampler, uv - step * 2.0) * 0.1216216;
    color += textureSample(effect_texture, effect_sampler, uv + step * 3.0) * 0.054054;
    color += textureSample(effect_texture, effect_sampler, uv - step * 3.0) * 0.054054;
    color += textureSample(effect_texture, effect_sampler, uv + step * 4.0) * 0.016216;
    color += textureSample(effect_texture, effect_sampler, uv - step * 4.0) * 0.016216;
    return color;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSample(original_texture, effect_sampler, input.uv);
    if globals.mix_amount <= 0.0001 {
        return original;
    }
    if globals.mode == 1u {
        let history = textureSample(history_texture, effect_sampler, input.uv);
        let trail = mix(original, history, clamp(globals.feedback, 0.0, 0.99));
        return mix(original, trail, clamp(globals.mix_amount, 0.0, 1.0));
    }
    return mix(original, blur(input.uv), clamp(globals.mix_amount, 0.0, 1.0));
}
