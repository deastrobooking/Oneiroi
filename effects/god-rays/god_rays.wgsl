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


// Reflect at every integer boundary instead of stretching edge pixels.
fn mirror_uv(uv: vec2<f32>) -> vec2<f32> {
    return 1.0 - abs(fract(uv * 0.5) * 2.0 - 1.0);
}

fn rotate(p: vec2<f32>, angle: f32) -> vec2<f32> {
    return vec2(cos(angle) * p.x - sin(angle) * p.y,
                sin(angle) * p.x + cos(angle) * p.y);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSampleLevel(original_texture, effect_sampler, input.uv, 0.0);
    let center = globals.parameters[0].xy;
    let step = (center - input.uv) * globals.parameters[0].z / 32.0;
    let threshold = globals.parameters[0].w;
    let softness = max(globals.parameters[1].w, 0.01);
    var rays = vec3(0.0);
    var weight = 1.0;
    var uv = input.uv;
    for (var i = 0u; i < 32u; i += 1u) {
        uv += step;
        let sample = textureSampleLevel(original_texture, effect_sampler, uv, 0.0);
        let luma = dot(sample.rgb, vec3(0.2126, 0.7152, 0.0722));
        let highlight = smoothstep(threshold, threshold + softness, luma);
        rays += sample.rgb * sample.a * highlight * weight;
        weight *= globals.parameters[1].y;
    }
    let warmth = globals.parameters[1].z;
    let tint = vec3(1.0 + warmth * 0.3, 1.0, 1.0 - warmth * 0.3);
    // Preserve the source matte so a transparent deck does not become opaque.
    let color = original.rgb + rays * tint * globals.parameters[1].x / 32.0;
    let effected = vec4(clamp(color, vec3(0.0), vec3(1.0)), original.a);
    return mix(original, effected, clamp(globals.mix_amount, 0.0, 1.0));
}
