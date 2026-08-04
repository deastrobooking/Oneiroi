struct MasterEffectGlobals {
    direction: vec2<f32>, texel_size: vec2<f32>, radius: f32, mix_amount: f32,
    mode: u32, feedback: f32, time_seconds: f32, parameter_count: u32,
    pass_index: u32, pass_count: u32, parameters: array<vec4<f32>,8>, history_valid: u32,
}
@group(0) @binding(0) var effect_sampler:sampler;
@group(0) @binding(1) var original_texture:texture_2d<f32>;
@group(0) @binding(2) var effect_texture:texture_2d<f32>;
@group(0) @binding(3) var<uniform> globals:MasterEffectGlobals;
@group(0) @binding(4) var history_texture:texture_2d<f32>;
@group(0) @binding(5) var custom_history_texture:texture_2d<f32>;
struct VertexOutput{@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>}
@vertex fn vs_main(@builtin(vertex_index) i:u32)->VertexOutput{
 let p=array(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));let u=array(vec2(0.0,1.0),vec2(2.0,1.0),vec2(0.0,-1.0));var o:VertexOutput;o.position=vec4(p[i],0.0,1.0);o.uv=u[i];return o;
}
fn rot2(p:vec2<f32>,a:f32)->vec2<f32>{let c=cos(a);let s=sin(a);return vec2(c*p.x-s*p.y,s*p.x+c*p.y);}
fn field(p0:vec3<f32>,kind:u32,iters:u32,scale:f32,fold:f32,twist:f32,t:f32)->vec2<f32>{
 var p=p0;var orbit=20.0;var derivative=1.0;
 for(var i=0u;i<7u;i=i+1u){if(i<iters){
  if(kind==0u){p=abs(p)-vec3(fold,fold*0.83,fold*1.14);let xy=rot2(p.xy,twist*p.z+t*0.05);p=vec3(xy,p.z);}
  else if(kind==1u){let xy=abs(rot2(p.xy,0.785398+twist*p.z))-vec2(fold);let z=p.z-floor((p.z+1.0)*0.5)*2.0;p=vec3(xy,z);}
  else{let r=max(length(p),0.08);let a=atan2(p.y,p.x)*2.0+twist;let b=acos(clamp(p.z/r,-1.0,1.0))*2.0;p=pow(r,1.35)*vec3(sin(b)*cos(a),sin(b)*sin(a),cos(b))-vec3(fold,0.15,0.0);}
  p=p*scale;derivative=derivative*scale;orbit=min(orbit,length(p));
 }}
 return vec2(abs(length(p)-fold)/max(derivative,0.001),orbit);
}
@fragment fn fs_main(input:VertexOutput)->@location(0) vec4<f32>{
 let original=textureSample(original_texture,effect_sampler,input.uv);
 let kind=u32(round(globals.parameters[0].x));let iters=u32(round(globals.parameters[0].y));let scale=globals.parameters[0].z;let fold=globals.parameters[0].w;
 let yaw=globals.parameters[1].x;let pitch=globals.parameters[1].y;let fov=globals.parameters[1].z;let depth=globals.parameters[1].w;
 let twist=globals.parameters[2].x;let density=globals.parameters[2].y;let travel=globals.parameters[2].z;let source_mix=globals.parameters[2].w;
 let light=globals.parameters[3].x;let palette=globals.parameters[3].y;let spin=globals.parameters[3].z;let animate=globals.parameters[3].w;let t=globals.time_seconds*animate;
 var screen=(input.uv-vec2(0.5))*vec2(2.0,2.0);screen.x=screen.x*(globals.texel_size.y/max(globals.texel_size.x,0.000001));
 var ray=normalize(vec3(screen*fov,1.0));let ray_xz=rot2(ray.xz,yaw+t*spin);ray=vec3(ray_xz.x,ray.y,ray_xz.y);let ray_yz=rot2(ray.yz,pitch);ray=vec3(ray.x,ray_yz);
 var origin=vec3(0.0,0.0,depth);let origin_xz=rot2(origin.xz,yaw+t*spin);origin=vec3(origin_xz.x,origin.y,origin_xz.y);
 var glow=vec3(0.0);var transmittance=1.0;var warped_uv=input.uv;
 for(var step=0u;step<28u;step=step+1u){let distance=f32(step)*travel;let point=origin+ray*distance;let result=field(point,kind,iters,scale,fold,twist,t);let fog=exp(-result.x*22.0)*density*0.075;let phase=result.y*0.7+distance*0.3+palette*6.2831853;let color=0.55+0.45*cos(vec3(phase,phase+2.1,phase+4.2));glow=glow+color*fog*transmittance;transmittance=transmittance*(1.0-clamp(fog*0.32,0.0,0.35));warped_uv=fract(point.xy*0.12+vec2(0.5));}
 let sampled=textureSample(original_texture,effect_sampler,warped_uv);
 let volume=vec4(mix(sampled.rgb,glow*light+sampled.rgb*0.2,0.82),original.a);
 let composed=mix(volume,original,source_mix);
 return mix(original,composed,clamp(globals.mix_amount,0.0,1.0));
}
