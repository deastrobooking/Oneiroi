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
    let aspect = vec2(globals.texel_size.y / max(globals.texel_size.x, 0.000001), 1.0);
    let p = rotate((input.uv - 0.5) * aspect, globals.parameters[1].x);
    let t = globals.time_seconds * globals.parameters[0].z;
    let seed = globals.parameters[1].y;
    var field = vec2(0.0);
    var weight = 1.0;
    var frequency = max(globals.parameters[0].y, 1.0);
    var total = 0.0;
    let detail = u32(clamp(round(globals.parameters[0].w), 1.0, 4.0));
    // Derivatives of a sinusoidal potential give a divergence-free field.
    // This is a procedural warp, not a stateful fluid simulation.
    for (var i = 0u; i < 4u; i += 1u) {
        if i < detail {
            let q = p * frequency + vec2(seed + t, seed * 1.73 + t * 0.73);
            field += vec2(cos(q.x) * sin(q.y), -sin(q.x) * cos(q.y)) * weight;
            total += weight;
        }
        frequency *= 2.03;
        weight *= 0.5;
    }
    let offset = rotate(field / max(total, 1.0), -globals.parameters[1].x);
    let uv = mirror_uv(input.uv + offset * globals.parameters[0].x / aspect);
    let effected = textureSampleLevel(original_texture, effect_sampler, uv, 0.0);
    return mix(original, effected, clamp(globals.mix_amount, 0.0, 1.0));
}
