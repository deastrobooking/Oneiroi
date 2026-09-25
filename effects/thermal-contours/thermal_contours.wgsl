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


fn luma_at(uv: vec2<f32>) -> f32 {
    let c = textureSampleLevel(original_texture, effect_sampler, uv, 0.0);
    return dot(c.rgb, vec3(0.2126, 0.7152, 0.0722)) * c.a;
}
fn heat_palette(t: f32, mode: u32) -> vec3<f32> {
    if mode == 1u {
        return clamp(vec3(0.4, 0.5, 0.5) + vec3(0.5) * cos(6.2831853 * (vec3(t) + vec3(0.0, 0.32, 0.62))), vec3(0.0), vec3(1.0));
    }
    if mode == 2u { return vec3(t); }
    let cold = mix(vec3(0.01, 0.0, 0.04), vec3(0.45, 0.01, 0.22), smoothstep(0.0, 0.4, t));
    let warm = mix(cold, vec3(1.0, 0.2, 0.01), smoothstep(0.3, 0.7, t));
    return mix(warm, vec3(1.0, 0.95, 0.55), smoothstep(0.65, 1.0, t));
}
@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSampleLevel(original_texture, effect_sampler, input.uv, 0.0);
    let luminance = dot(original.rgb, vec3(0.2126, 0.7152, 0.0722));
    let heat = clamp((luminance - 0.5) * globals.parameters[0].y + 0.5 + globals.parameters[0].z, 0.0, 1.0);
    let dx = vec2(globals.texel_size.x, 0.0);
    let dy = vec2(0.0, globals.texel_size.y);
    let gradient = length(vec2(luma_at(input.uv + dx) - luma_at(input.uv - dx), luma_at(input.uv + dy) - luma_at(input.uv - dy)));
    let phase = heat * globals.parameters[1].x + globals.time_seconds * globals.parameters[1].z;
    let line_distance = abs(fract(phase) - 0.5);
    let aa = max(fwidth(phase), 0.015);
    let line = 1.0 - smoothstep(0.035, 0.035 + aa, line_distance);
    var color = heat_palette(heat, u32(round(globals.parameters[0].x)));
    color = color * (1.0 - globals.parameters[0].w * line) + vec3(gradient * globals.parameters[1].y);
    color = mix(clamp(color, vec3(0.0), vec3(1.0)), original.rgb, globals.parameters[1].w);
    color = select(vec3(0.0), color, original.a > 0.0);
    return mix(original, vec4(color, original.a), clamp(globals.mix_amount, 0.0, 1.0));
}
