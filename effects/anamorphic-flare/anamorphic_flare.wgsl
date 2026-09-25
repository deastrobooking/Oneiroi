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


fn highlight(uv: vec2<f32>) -> vec4<f32> {
    let source = textureSampleLevel(original_texture, effect_sampler, uv, 0.0);
    let luma = dot(source.rgb, vec3(0.2126, 0.7152, 0.0722));
    let threshold = globals.parameters[0].x;
    let knee = max(globals.parameters[0].y, 0.01);
    let bright = smoothstep(threshold - knee, threshold + knee, luma);
    // Premultiplied energy prevents invisible source RGB from producing flares.
    return vec4(source.rgb * source.a * bright, source.a);
}
@fragment
fn fs_extract(input: VertexOutput) -> @location(0) vec4<f32> {
    // Prefilter across one gather interval to soften gaps between streak taps.
    let step_size = globals.parameters[0].z / 32.0;
    var sum = vec4(0.0);
    for (var i = -2; i <= 2; i += 1) {
        sum += highlight(input.uv + vec2(f32(i) * step_size, 0.0));
    }
    return sum / 5.0;
}
@fragment
fn fs_combine(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSampleLevel(original_texture, effect_sampler, input.uv, 0.0);
    let span = globals.parameters[0].z;
    let dispersion = globals.parameters[1].y;
    var streak = vec3(0.0);
    var weights = 0.0;
    for (var i = -8; i <= 8; i += 1) {
        let x = f32(i) / 8.0;
        let weight = exp(-3.0 * x * x);
        let offset = vec2(x * span, 0.0);
        let a = textureSampleLevel(effect_texture, effect_sampler, input.uv + offset * (1.0 + dispersion * 0.2), 0.0);
        let b = textureSampleLevel(effect_texture, effect_sampler, input.uv + offset, 0.0);
        let c = textureSampleLevel(effect_texture, effect_sampler, input.uv + offset * (1.0 - dispersion * 0.2), 0.0);
        streak += vec3(a.r, b.g, c.b) * weight;
        weights += weight;
    }
    let tint = mix(vec3(1.0, 0.55, 0.15), vec3(0.2, 0.6, 1.0), globals.parameters[1].x * 0.5 + 0.5);
    let energy = streak / max(weights, 0.001) * tint * globals.parameters[0].w;
    let color = clamp(original.rgb + energy, vec3(0.0), vec3(1.0));
    return mix(original, vec4(color, original.a), clamp(globals.mix_amount, 0.0, 1.0));
}
