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


fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}
fn screen_sample(uv: vec2<f32>) -> vec4<f32> {
    let sample = textureSampleLevel(original_texture, effect_sampler, uv, 0.0);
    return select(vec4(0.0), sample, all(uv >= vec2(0.0)) && all(uv <= vec2(1.0)));
}
@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSampleLevel(original_texture, effect_sampler, input.uv, 0.0);
    let screen = input.uv * 2.0 - 1.0;
    let curvature = globals.parameters[0].x;
    var uv = (screen * (1.0 + curvature * dot(screen, screen))) * 0.5 + 0.5;
    let tick = floor(globals.time_seconds * 30.0);
    let row = floor(uv.y * globals.parameters[0].z);
    uv.x += (hash21(vec2(row, tick)) - 0.5) * globals.parameters[1].y;
    let source = screen_sample(uv);
    let bleed = globals.parameters[1].x;
    let red = screen_sample(uv + vec2(bleed, 0.0));
    let blue = screen_sample(uv - vec2(bleed, 0.0));
    // Ignore hidden RGB in transparent neighbors when separating channels.
    var color = vec3(red.r * red.a, source.g * source.a, blue.b * blue.a) / max(source.a, 0.0001);
    let line = 0.5 + 0.5 * cos(uv.y * globals.parameters[0].z * 6.2831853);
    color *= 1.0 - globals.parameters[0].y * line;
    let column = u32(input.position.x) % 3u;
    let phosphor = vec3(select(0.55, 1.0, column == 0u), select(0.55, 1.0, column == 1u), select(0.55, 1.0, column == 2u));
    color *= mix(vec3(1.0), phosphor, globals.parameters[0].w);
    color += (hash21(input.position.xy + vec2(tick, -tick)) - 0.5) * globals.parameters[1].z;
    color *= 1.0 - globals.parameters[1].w * smoothstep(0.2, 1.6, dot(screen, screen));
    let visible_color = select(vec3(0.0), clamp(color, vec3(0.0), vec3(1.0)), source.a > 0.0);
    let effected = vec4(visible_color, source.a);
    return mix(original, effected, clamp(globals.mix_amount, 0.0, 1.0));
}
