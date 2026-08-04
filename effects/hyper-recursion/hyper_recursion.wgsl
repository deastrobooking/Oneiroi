struct MasterEffectGlobals{direction:vec2<f32>,texel_size:vec2<f32>,radius:f32,mix_amount:f32,mode:u32,feedback:f32,time_seconds:f32,parameter_count:u32,pass_index:u32,pass_count:u32,parameters:array<vec4<f32>,8>,history_valid:u32,}
@group(0) @binding(0) var effect_sampler:sampler;@group(0) @binding(1) var original_texture:texture_2d<f32>;@group(0) @binding(2) var effect_texture:texture_2d<f32>;@group(0) @binding(3) var<uniform> globals:MasterEffectGlobals;@group(0) @binding(4) var history_texture:texture_2d<f32>;@group(0) @binding(5) var custom_history_texture:texture_2d<f32>;
struct VertexOutput{@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>}
@vertex fn vs_main(@builtin(vertex_index) i:u32)->VertexOutput{let p=array(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));let u=array(vec2(0.0,1.0),vec2(2.0,1.0),vec2(0.0,-1.0));var o:VertexOutput;o.position=vec4(p[i],0.0,1.0);o.uv=u[i];return o;}
fn rot(p:vec2<f32>,a:f32)->vec2<f32>{let c=cos(a);let s=sin(a);return vec2(c*p.x-s*p.y,s*p.x+c*p.y);}
fn rotate4(q0:vec4<f32>,a:f32,b:f32,c:f32)->vec4<f32>{let xy=rot(q0.xy,a);var q=vec4(xy,q0.zw);let xw=rot(q.xw,b);q=vec4(xw.x,q.y,q.z,xw.y);let zw=rot(q.zw,c);return vec4(q.xy,zw);}
fn hue(c:vec3<f32>,a:f32)->vec3<f32>{let k=normalize(vec3(1.0));return c*cos(a)+cross(k,c)*sin(a)+k*dot(k,c)*(1.0-cos(a));}
@fragment fn fs_main(input:VertexOutput)->@location(0) vec4<f32>{
 let original=textureSample(original_texture,effect_sampler,input.uv);let dims=u32(round(globals.parameters[0].x));let fn_kind=u32(round(globals.parameters[0].y));let depth=u32(round(globals.parameters[0].z));let scale=globals.parameters[0].w;
 let a=globals.parameters[1].x;let b=globals.parameters[1].y;let c=globals.parameters[1].z;let slice=globals.parameters[1].w;let key=globals.parameters[2];
 let projection=globals.parameters[3].x;let source_mix=globals.parameters[3].y;let bands=globals.parameters[3].z;let animate=globals.parameters[3].w;let t=globals.time_seconds*animate;
 var q=vec4((input.uv-vec2(0.5))*2.0,0.25*sin(t*0.37),slice);var extra=vec2(slice,0.31);
 var orbit=20.0;
 for(var i=0u;i<7u;i=i+1u){if(i<depth){let fi=f32(i);q=rotate4(q,a+t*0.08,b-t*0.11,c+t*0.07);
  if(dims>=5u){extra=vec2(sin(extra.x*scale+q.z+fi*0.2),extra.y);q=vec4(q.xyz,q.w+extra.x*0.22);}
  if(dims>=6u){extra=vec2(extra.x,cos(extra.y*scale+q.y-q.w));q=vec4(q.xy,q.z+extra.y*0.18,q.w);}
  if(fn_kind==0u){q=abs(q)*scale-key;}
  else if(fn_kind==1u){let v=q.yzw;q=vec4(q.x*q.x-dot(v,v),2.0*q.x*v)+key*0.18;q=q*scale;}
  else{q=sin(q.zwxy*scale+key)+cos(q.wxyz-key*0.7);q=q*0.78;}
  orbit=min(orbit,length(q));
 }}
 let projected=q.xy/(1.0+projection*abs(q.w));let sample_uv=fract(projected*0.5+vec2(0.5));var recursive=textureSample(original_texture,effect_sampler,sample_uv);
 let band_color=0.5+0.5*cos(vec3(orbit*14.0+t*0.4,orbit*14.0+2.1,orbit*14.0+4.2));recursive=vec4(mix(recursive.rgb,hue(recursive.rgb,orbit*3.0)+band_color*0.35,bands/1.5),recursive.a);
 let composed=mix(recursive,original,source_mix);return mix(original,composed,clamp(globals.mix_amount,0.0,1.0));
}
