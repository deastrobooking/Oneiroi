// Milestone 1 proof of life: a spinning triangle driven entirely by uniforms.
//
// Colours are written in linear space; the sRGB swapchain format encodes on
// write. Nothing in this project applies a manual pow(2.2) anywhere.

struct Globals {
    time: f32,
    spin: f32,
    aspect: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
};

const TAU: f32 = 6.283185307179586;

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    let turn = f32(idx) / 3.0;
    let angle = TAU * turn + globals.time * globals.spin;

    var out: VsOut;
    out.pos = vec4<f32>(0.75 * sin(angle) / globals.aspect, 0.75 * cos(angle), 0.0, 1.0);
    out.color = 0.5 + 0.5 * cos(
        TAU * (turn + globals.time * 0.1) + vec3<f32>(0.0, 2.0944, 4.1888)
    );
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
