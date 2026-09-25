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
    let original = textureSampleLevel(original_texture, effect_sampler, input.uv, 0.0);
    let aspect = globals.texel_size.y / max(globals.texel_size.x, 0.000001);
    let center = globals.parameters[1].xy;
    let p = (input.uv - center) * vec2(aspect, 1.0);
    let radius = max(globals.parameters[0].y, 0.05);
    let influence = pow(clamp(1.0 - length(p) / radius, 0.0, 1.0), globals.parameters[0].w);
    let pulse = 1.0 + globals.parameters[1].z * sin(globals.time_seconds * globals.parameters[1].w * 6.2831853);
    let magnification = clamp(1.0 + globals.parameters[0].x * pulse * influence, 0.1, 3.0);
    let angle = globals.parameters[0].z * influence;
    let warped = vec2(cos(angle) * p.x - sin(angle) * p.y, sin(angle) * p.x + cos(angle) * p.y) / magnification;
    let uv = warped / vec2(aspect, 1.0) + center;
    let sample = textureSampleLevel(original_texture, effect_sampler, uv, 0.0);
    let effected = select(vec4(0.0), sample, all(uv >= vec2(0.0)) && all(uv <= vec2(1.0)));
    return mix(original, effected, clamp(globals.mix_amount, 0.0, 1.0));
}
