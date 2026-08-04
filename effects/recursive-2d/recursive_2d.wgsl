struct MasterEffectGlobals {
    direction: vec2<f32>, texel_size: vec2<f32>, radius: f32, mix_amount: f32,
    mode: u32, feedback: f32, time_seconds: f32, parameter_count: u32,
    pass_index: u32, pass_count: u32, parameters: array<vec4<f32>, 8>, history_valid: u32,
}
@group(0) @binding(0) var effect_sampler: sampler;
@group(0) @binding(1) var original_texture: texture_2d<f32>;
@group(0) @binding(2) var effect_texture: texture_2d<f32>;
@group(0) @binding(3) var<uniform> globals: MasterEffectGlobals;
@group(0) @binding(4) var history_texture: texture_2d<f32>;
@group(0) @binding(5) var custom_history_texture: texture_2d<f32>;
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> VertexOutput {
    let p = array(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));
    let u = array(vec2(0.0,1.0),vec2(2.0,1.0),vec2(0.0,-1.0));
    var o: VertexOutput; o.position=vec4(p[i],0.0,1.0); o.uv=u[i]; return o;
}
fn rotate2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c=cos(a); let s=sin(a); return vec2(c*p.x-s*p.y,s*p.x+c*p.y);
}
fn hue_rotate(c: vec3<f32>, a: f32) -> vec3<f32> {
    let k=normalize(vec3(1.0)); return c*cos(a)+cross(k,c)*sin(a)+k*dot(k,c)*(1.0-cos(a));
}
@fragment fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let original=textureSample(original_texture,effect_sampler,input.uv);
    let fn_mode=u32(round(globals.parameters[0].x));
    let depth=u32(round(globals.parameters[0].y));
    let scale=globals.parameters[0].z; let rotation=globals.parameters[0].w;
    let fold=globals.parameters[1].x; let key=globals.parameters[1].yz;
    let zoom=globals.parameters[1].w; let spin=globals.parameters[2].x;
    let pulse=globals.parameters[2].y; let animate=globals.parameters[2].z;
    let source_mix=globals.parameters[2].w; let edge=globals.parameters[3].x;
    let hue_cycle=globals.parameters[3].y;
    let t=globals.time_seconds*animate;
    var p=(input.uv-vec2(0.5))*vec2(2.0,2.0)*zoom;
    p=rotate2(p,rotation+t*spin);
    var orbit=10.0;
    for (var i=0u; i<8u; i=i+1u) {
        if (i < depth) {
            let fi=f32(i);
            if (fn_mode == 0u) {
                p=abs(p)*scale-vec2(fold+key.x,key.y);
                p=rotate2(p,rotation*(0.35+fi*0.08)+t*spin*0.13);
            } else if (fn_mode == 1u) {
                p=vec2(p.x*p.x-p.y*p.y,2.0*p.x*p.y)*scale*0.58+key;
                p=rotate2(p,rotation+t*spin*0.08);
            } else {
                let d=max(dot(p,p),0.035);
                p=abs(vec2(p.x,-p.y)/d)*scale-vec2(fold,key.y+key.x);
                p=rotate2(p,rotation+fi*0.31+t*spin*0.09);
            }
            p=p*(1.0+pulse*sin(t*1.7+fi));
            orbit=min(orbit,length(p));
        }
    }
    let sample_uv=fract(p*0.5+vec2(0.5));
    var recursive=textureSample(original_texture,effect_sampler,sample_uv);
    let rings=0.5+0.5*cos(18.0*orbit+vec3(0.0,2.1,4.2)+t);
    recursive=vec4(mix(recursive.rgb,recursive.rgb*rings+0.22*rings,edge),recursive.a);
    recursive=vec4(hue_rotate(recursive.rgb,hue_cycle*6.2831853*(orbit+0.15*t)),recursive.a);
    let composed=mix(recursive,original,source_mix);
    return mix(original,composed,clamp(globals.mix_amount,0.0,1.0));
}
