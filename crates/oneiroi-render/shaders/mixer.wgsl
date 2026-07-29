struct MixerGlobals {
    levels: vec4<f32>,
    source_kinds: vec4<u32>,
    contrast: vec4<f32>,
    saturation: vec4<f32>,
    hue: vec4<f32>,
    black_level: vec4<f32>,
    white_level: vec4<f32>,
    gamma: vec4<f32>,
    pixelate: vec4<f32>,
    luma_key: vec4<f32>,
    neon: vec4<f32>,
    fractal: vec4<f32>,
    jitter: vec4<f32>,
    find_edges: vec4<f32>,
    bit_reduction: vec4<f32>,
    blacklight: vec4<f32>,
    mirror: vec4<u32>,
    position_x: vec4<f32>,
    position_y: vec4<f32>,
    scale: vec4<f32>,
    rotation: vec4<f32>,
    flip_horizontal: vec4<u32>,
    flip_vertical: vec4<u32>,
    bus_assignments: vec4<u32>,
    crossfade_gains: vec2<f32>,
    master_opacity: f32,
    time_seconds: f32,
    blackout: u32,
    _padding_a: u32,
    _padding_b: u32,
    _padding_c: u32,
}

struct EffectConfig {
    contrast: f32,
    saturation: f32,
    hue: f32,
    black_level: f32,
    white_level: f32,
    gamma: f32,
    pixelate: f32,
    luma_key: f32,
    neon: f32,
    fractal: f32,
    jitter: f32,
    find_edges: f32,
    bit_reduction: f32,
    blacklight: f32,
    mirror: u32,
}

@group(0) @binding(0) var source_sampler: sampler;
@group(0) @binding(1) var source_a: texture_2d<f32>;
@group(0) @binding(2) var source_b: texture_2d<f32>;
@group(0) @binding(3) var source_c: texture_2d<f32>;
@group(0) @binding(4) var source_d: texture_2d<f32>;
@group(0) @binding(5) var alpha_a: texture_2d<f32>;
@group(0) @binding(6) var alpha_b: texture_2d<f32>;
@group(0) @binding(7) var alpha_c: texture_2d<f32>;
@group(0) @binding(8) var alpha_d: texture_2d<f32>;
@group(0) @binding(9) var<uniform> globals: MixerGlobals;

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

fn ycocg_to_rgba(encoded: vec4<f32>) -> vec4<f32> {
    let scale = encoded.b * (255.0 / 8.0) + 1.0;
    let cocg = (encoded.rg - vec2(0.5)) / scale;
    let y = encoded.a;
    return vec4(
        y + cocg.x - cocg.y,
        y + cocg.y,
        y - cocg.x - cocg.y,
        1.0,
    );
}

fn interpret(primary: vec4<f32>, alpha: f32, kind: u32) -> vec4<f32> {
    if kind == 1u {
        return ycocg_to_rgba(primary);
    }
    if kind == 2u {
        return vec4(1.0, 1.0, 1.0, primary.r);
    }
    if kind == 3u {
        let color = ycocg_to_rgba(primary);
        return vec4(color.rgb, alpha);
    }
    return primary;
}

fn sample_source(
    primary: texture_2d<f32>,
    alpha_texture: texture_2d<f32>,
    uv: vec2<f32>,
    kind: u32,
) -> vec4<f32> {
    return interpret(
        textureSample(primary, source_sampler, uv),
        textureSample(alpha_texture, source_sampler, uv).r,
        kind,
    );
}

fn over(back: vec4<f32>, straight_front: vec4<f32>, level: f32) -> vec4<f32> {
    let alpha = clamp(straight_front.a * level, 0.0, 1.0);
    let front = vec4(straight_front.rgb * alpha, alpha);
    return front + back * (1.0 - alpha);
}

fn layer_uv(
    input_uv: vec2<f32>,
    position: vec2<f32>,
    scale: f32,
    rotation: f32,
    flip_horizontal: u32,
    flip_vertical: u32,
) -> vec3<f32> {
    var local = input_uv - vec2(0.5) - position * 0.5;
    let angle = -rotation * 6.2831853;
    let sine = sin(angle);
    let cosine = cos(angle);
    local = vec2(
        cosine * local.x - sine * local.y,
        sine * local.x + cosine * local.y,
    ) / max(scale, 0.05);
    if flip_horizontal != 0u {
        local.x = -local.x;
    }
    if flip_vertical != 0u {
        local.y = -local.y;
    }
    let uv = local + vec2(0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) {
        return vec3(uv, 0.0);
    }
    return vec3(uv, 1.0);
}

fn effect_uv(input_uv: vec2<f32>, effect: EffectConfig) -> vec2<f32> {
    var uv = input_uv;
    if effect.mirror != 0u {
        uv.x = abs(uv.x * 2.0 - 1.0);
    }
    if effect.fractal > 0.0001 {
        let centered = uv - vec2(0.5);
        let radius = length(centered);
        let angle = atan2(centered.y, centered.x);
        let segments = mix(2.0, 12.0, effect.fractal);
        let folded = abs(fract(angle / 6.2831853 * segments + 0.5) - 0.5);
        let kaleidoscope = vec2(cos(folded * 6.2831853), sin(folded * 6.2831853))
            * radius + vec2(0.5);
        uv = mix(uv, kaleidoscope, effect.fractal);
    }
    if effect.jitter > 0.0001 {
        let row = floor(uv.y * mix(24.0, 240.0, effect.jitter));
        let noise = fract(sin(row * 78.233 + floor(globals.time_seconds * 24.0) * 17.17) * 43758.5453);
        uv.x += (noise - 0.5) * effect.jitter * 0.16;
        uv.y += sin(globals.time_seconds * 18.0 + uv.x * 40.0) * effect.jitter * 0.006;
    }
    if effect.pixelate > 0.0001 {
        uv = (floor(uv / effect.pixelate) + vec2(0.5)) * effect.pixelate;
    }
    return clamp(uv, vec2(0.0), vec2(1.0));
}

fn hue_rotate(color: vec3<f32>, turns: f32) -> vec3<f32> {
    let angle = turns * 6.2831853;
    let axis = normalize(vec3(1.0));
    return color * cos(angle)
        + cross(axis, color) * sin(angle)
        + axis * dot(axis, color) * (1.0 - cos(angle));
}

fn edge_strength(
    primary: texture_2d<f32>,
    alpha_texture: texture_2d<f32>,
    uv: vec2<f32>,
    kind: u32,
) -> f32 {
    let dimensions = vec2<f32>(textureDimensions(primary));
    let texel = 1.0 / max(dimensions, vec2(1.0));
    let left = sample_source(primary, alpha_texture, uv - vec2(texel.x, 0.0), kind).rgb;
    let right = sample_source(primary, alpha_texture, uv + vec2(texel.x, 0.0), kind).rgb;
    let up = sample_source(primary, alpha_texture, uv - vec2(0.0, texel.y), kind).rgb;
    let down = sample_source(primary, alpha_texture, uv + vec2(0.0, texel.y), kind).rgb;
    let luma_weights = vec3(0.2126, 0.7152, 0.0722);
    let gradient = vec2(dot(right - left, luma_weights), dot(down - up, luma_weights));
    return clamp(length(gradient) * 3.0, 0.0, 1.0);
}

fn apply_color_effects(color: vec4<f32>, edge: f32, effect: EffectConfig) -> vec4<f32> {
    var rgb = max(color.rgb, vec3(0.0));
    let initial_luma = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3(initial_luma), rgb, effect.saturation);
    rgb = hue_rotate(rgb, effect.hue);
    rgb = (rgb - vec3(0.5)) * effect.contrast + vec3(0.5);
    rgb = clamp(
        (rgb - vec3(effect.black_level))
            / max(effect.white_level - effect.black_level, 0.01),
        vec3(0.0),
        vec3(1.0),
    );
    rgb = pow(rgb, vec3(1.0 / max(effect.gamma, 0.1)));

    if effect.bit_reduction > 0.0001 {
        let steps = max(2.0, floor(mix(256.0, 2.0, effect.bit_reduction)));
        rgb = floor(rgb * steps + vec3(0.5)) / steps;
    }
    if effect.blacklight > 0.0001 {
        let inverse = vec3(1.0) - rgb;
        let ultraviolet = vec3(
            inverse.b * 0.35 + rgb.r * 0.2,
            inverse.g * 0.08 + rgb.b * 0.25,
            inverse.r * 0.65 + rgb.b,
        );
        rgb = mix(rgb, ultraviolet, effect.blacklight);
    }
    if effect.neon > 0.0001 {
        let neon_color = vec3(edge * 0.15, edge * edge * 0.9, edge);
        rgb = mix(rgb, rgb * 0.25 + neon_color * 1.8, effect.neon);
    }
    if effect.find_edges > 0.0001 {
        let edge_color = vec3(edge);
        rgb = mix(rgb, edge_color, effect.find_edges);
    }

    var alpha = color.a;
    if effect.luma_key > 0.0001 {
        alpha *= smoothstep(effect.luma_key - 0.05, effect.luma_key + 0.05, initial_luma);
    }
    return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), alpha);
}

fn process_source(
    primary: texture_2d<f32>,
    alpha_texture: texture_2d<f32>,
    input_uv: vec2<f32>,
    kind: u32,
    effect: EffectConfig,
) -> vec4<f32> {
    let uv = effect_uv(input_uv, effect);
    let color = sample_source(primary, alpha_texture, uv, kind);
    var edge = 0.0;
    if effect.neon > 0.0001 || effect.find_edges > 0.0001 {
        edge = edge_strength(primary, alpha_texture, uv, kind);
    }
    return apply_color_effects(color, edge, effect);
}

fn process_layer(
    primary: texture_2d<f32>,
    alpha_texture: texture_2d<f32>,
    transformed_uv: vec3<f32>,
    kind: u32,
    effect: EffectConfig,
) -> vec4<f32> {
    if transformed_uv.z == 0.0 {
        return vec4(0.0);
    }
    return process_source(primary, alpha_texture, transformed_uv.xy, kind, effect);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if globals.blackout != 0u {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    let effect_a = EffectConfig(globals.contrast.x, globals.saturation.x, globals.hue.x, globals.black_level.x, globals.white_level.x, globals.gamma.x, globals.pixelate.x, globals.luma_key.x, globals.neon.x, globals.fractal.x, globals.jitter.x, globals.find_edges.x, globals.bit_reduction.x, globals.blacklight.x, globals.mirror.x);
    let effect_b = EffectConfig(globals.contrast.y, globals.saturation.y, globals.hue.y, globals.black_level.y, globals.white_level.y, globals.gamma.y, globals.pixelate.y, globals.luma_key.y, globals.neon.y, globals.fractal.y, globals.jitter.y, globals.find_edges.y, globals.bit_reduction.y, globals.blacklight.y, globals.mirror.y);
    let effect_c = EffectConfig(globals.contrast.z, globals.saturation.z, globals.hue.z, globals.black_level.z, globals.white_level.z, globals.gamma.z, globals.pixelate.z, globals.luma_key.z, globals.neon.z, globals.fractal.z, globals.jitter.z, globals.find_edges.z, globals.bit_reduction.z, globals.blacklight.z, globals.mirror.z);
    let effect_d = EffectConfig(globals.contrast.w, globals.saturation.w, globals.hue.w, globals.black_level.w, globals.white_level.w, globals.gamma.w, globals.pixelate.w, globals.luma_key.w, globals.neon.w, globals.fractal.w, globals.jitter.w, globals.find_edges.w, globals.bit_reduction.w, globals.blacklight.w, globals.mirror.w);
    let uv_a = layer_uv(input.uv, vec2(globals.position_x.x, globals.position_y.x), globals.scale.x, globals.rotation.x, globals.flip_horizontal.x, globals.flip_vertical.x);
    let uv_b = layer_uv(input.uv, vec2(globals.position_x.y, globals.position_y.y), globals.scale.y, globals.rotation.y, globals.flip_horizontal.y, globals.flip_vertical.y);
    let uv_c = layer_uv(input.uv, vec2(globals.position_x.z, globals.position_y.z), globals.scale.z, globals.rotation.z, globals.flip_horizontal.z, globals.flip_vertical.z);
    let uv_d = layer_uv(input.uv, vec2(globals.position_x.w, globals.position_y.w), globals.scale.w, globals.rotation.w, globals.flip_horizontal.w, globals.flip_vertical.w);
    let a = process_layer(source_a, alpha_a, uv_a, globals.source_kinds.x, effect_a);
    let b = process_layer(source_b, alpha_b, uv_b, globals.source_kinds.y, effect_b);
    let c = process_layer(source_c, alpha_c, uv_c, globals.source_kinds.z, effect_c);
    let d = process_layer(source_d, alpha_d, uv_d, globals.source_kinds.w, effect_d);

    var bus_a = vec4(0.0);
    var bus_b = vec4(0.0);
    if globals.bus_assignments.x == 0u {
        bus_a = over(bus_a, a, globals.levels.x);
    } else {
        bus_b = over(bus_b, a, globals.levels.x);
    }
    if globals.bus_assignments.y == 0u {
        bus_a = over(bus_a, b, globals.levels.y);
    } else {
        bus_b = over(bus_b, b, globals.levels.y);
    }
    if globals.bus_assignments.z == 0u {
        bus_a = over(bus_a, c, globals.levels.z);
    } else {
        bus_b = over(bus_b, c, globals.levels.z);
    }
    if globals.bus_assignments.w == 0u {
        bus_a = over(bus_a, d, globals.levels.w);
    } else {
        bus_b = over(bus_b, d, globals.levels.w);
    }
    var mixed = bus_a * globals.crossfade_gains.x
        + bus_b * globals.crossfade_gains.y;
    mixed *= globals.master_opacity;
    return vec4(mixed.rgb, 1.0);
}
