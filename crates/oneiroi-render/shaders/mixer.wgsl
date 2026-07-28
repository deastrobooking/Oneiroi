struct MixerGlobals {
    levels: vec4<f32>,
    source_kinds: vec4<u32>,
    master_opacity: f32,
    blackout: u32,
    _padding: vec2<u32>,
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
    let rgb = vec3(
        y + cocg.x - cocg.y,
        y + cocg.y,
        y - cocg.x - cocg.y,
    );
    return vec4(rgb, 1.0);
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

fn over(back: vec4<f32>, straight_front: vec4<f32>, level: f32) -> vec4<f32> {
    let alpha = clamp(straight_front.a * level, 0.0, 1.0);
    let front = vec4(straight_front.rgb * alpha, alpha);
    return front + back * (1.0 - alpha);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if globals.blackout != 0u {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    let a = interpret(textureSample(source_a, source_sampler, input.uv), textureSample(alpha_a, source_sampler, input.uv).r, globals.source_kinds.x);
    let b = interpret(textureSample(source_b, source_sampler, input.uv), textureSample(alpha_b, source_sampler, input.uv).r, globals.source_kinds.y);
    let c = interpret(textureSample(source_c, source_sampler, input.uv), textureSample(alpha_c, source_sampler, input.uv).r, globals.source_kinds.z);
    let d = interpret(textureSample(source_d, source_sampler, input.uv), textureSample(alpha_d, source_sampler, input.uv).r, globals.source_kinds.w);

    var mixed = vec4(0.0);
    mixed = over(mixed, a, globals.levels.x);
    mixed = over(mixed, b, globals.levels.y);
    mixed = over(mixed, c, globals.levels.z);
    mixed = over(mixed, d, globals.levels.w);
    mixed *= globals.master_opacity;
    return vec4(mixed.rgb, 1.0);
}
