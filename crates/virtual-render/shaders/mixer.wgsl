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
    bloom: vec4<f32>,
    bloom_threshold: vec4<f32>,
    bloom_radius: vec4<f32>,
    bloom_chroma: vec4<f32>,
    mirror: vec4<u32>,
    effect_slot_groups_0: vec4<u32>,
    effect_slot_groups_1: vec4<u32>,
    effect_slot_groups_2: vec4<u32>,
    effect_slot_enabled_0: vec4<u32>,
    effect_slot_enabled_1: vec4<u32>,
    effect_slot_enabled_2: vec4<u32>,
    effect_slot_mix_0: vec4<f32>,
    effect_slot_mix_1: vec4<f32>,
    effect_slot_mix_2: vec4<f32>,
    position_x: vec4<f32>,
    position_y: vec4<f32>,
    scale: vec4<f32>,
    rotation: vec4<f32>,
    flip_horizontal: vec4<u32>,
    flip_vertical: vec4<u32>,
    crop_left: vec4<f32>,
    crop_right: vec4<f32>,
    crop_top: vec4<f32>,
    crop_bottom: vec4<f32>,
    source_modes: vec4<u32>,
    blend_modes: vec4<u32>,
    bus_assignments: vec4<u32>,
    crossfade_gains: vec2<f32>,
    master_opacity: f32,
    time_seconds: f32,
    output_aspect: f32,
    blackout: u32,
    _padding_a: u32,
    _padding_b: u32,
    deck_override_mask: vec4<u32>,
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
    bloom: f32,
    bloom_threshold: f32,
    bloom_radius: f32,
    bloom_chroma: f32,
    mirror: u32,
    slot_groups: vec4<u32>,
    slot_enabled: vec4<u32>,
    slot_mix: vec4<f32>,
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
@group(1) @binding(0) var deck_override_a: texture_2d<f32>;
@group(1) @binding(1) var deck_override_b: texture_2d<f32>;
@group(1) @binding(2) var deck_override_c: texture_2d<f32>;
@group(1) @binding(3) var deck_override_d: texture_2d<f32>;

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

// Blend modes follow the separable and non-separable definitions in W3C
// Compositing and Blending Level 1, which is also what Photoshop implements.
// One deliberate difference: this engine composites in linear light, while
// Photoshop evaluates in gamma space, so mid-tone behaviour is smoother here
// and exact numeric parity with a Photoshop screenshot is not a goal.
//
// `mode` is uniform across the draw, so these branches cost a scalar jump
// rather than per-pixel divergence.

// The non-separable modes carry their own luminance weights in the spec.
// They stay distinct from the Rec. 709 weights the grading code uses.
const BLEND_LUMA: vec3<f32> = vec3(0.3, 0.59, 0.11);
const TAU: f32 = 6.2831853;

fn blend_lum(color: vec3<f32>) -> f32 {
    return dot(color, BLEND_LUMA);
}

fn clip_color(color: vec3<f32>) -> vec3<f32> {
    let luma = blend_lum(color);
    let low = min(color.r, min(color.g, color.b));
    let high = max(color.r, max(color.g, color.b));
    var clipped = color;
    if low < 0.0 {
        clipped = luma + ((clipped - luma) * luma) / max(luma - low, 0.00001);
    }
    if high > 1.0 {
        clipped = luma + ((clipped - luma) * (1.0 - luma)) / max(high - luma, 0.00001);
    }
    return clipped;
}

fn set_lum(color: vec3<f32>, luma: f32) -> vec3<f32> {
    return clip_color(color + (luma - blend_lum(color)));
}

fn blend_sat(color: vec3<f32>) -> f32 {
    return max(color.r, max(color.g, color.b)) - min(color.r, min(color.g, color.b));
}

fn set_sat(color: vec3<f32>, saturation: f32) -> vec3<f32> {
    let low = min(color.r, min(color.g, color.b));
    let high = max(color.r, max(color.g, color.b));
    if high <= low {
        return vec3(0.0);
    }
    return (color - low) * saturation / (high - low);
}

/// Hue angle in turns, taken from the RGB hexagon's chroma plane.
fn hue_angle(color: vec3<f32>) -> f32 {
    let alpha = color.r - 0.5 * (color.g + color.b);
    let beta = 0.8660254 * (color.g - color.b);
    return atan2(beta, alpha) / TAU;
}

fn hue_rotate(color: vec3<f32>, turns: f32) -> vec3<f32> {
    let angle = turns * TAU;
    let axis = normalize(vec3(1.0));
    return color * cos(angle)
        + cross(axis, color) * sin(angle)
        + axis * dot(axis, color) * (1.0 - cos(angle));
}

fn blend_screen(back: vec3<f32>, front: vec3<f32>) -> vec3<f32> {
    return back + front - back * front;
}

fn blend_color_dodge(back: vec3<f32>, front: vec3<f32>) -> vec3<f32> {
    let dodged = min(vec3(1.0), back / max(vec3(1.0) - front, vec3(0.00001)));
    let saturated = select(dodged, vec3(1.0), front >= vec3(1.0));
    return select(saturated, vec3(0.0), back <= vec3(0.0));
}

fn blend_color_burn(back: vec3<f32>, front: vec3<f32>) -> vec3<f32> {
    let burned = vec3(1.0) - min(vec3(1.0), (vec3(1.0) - back) / max(front, vec3(0.00001)));
    let floored = select(burned, vec3(0.0), front <= vec3(0.0));
    return select(floored, vec3(1.0), back >= vec3(1.0));
}

fn blend_hard_light(back: vec3<f32>, front: vec3<f32>) -> vec3<f32> {
    let low = 2.0 * back * front;
    let high = blend_screen(back, 2.0 * front - vec3(1.0));
    return select(low, high, front > vec3(0.5));
}

fn soft_light_curve(back: vec3<f32>) -> vec3<f32> {
    let low = ((16.0 * back - vec3(12.0)) * back + vec3(4.0)) * back;
    return select(sqrt(max(back, vec3(0.0))), low, back <= vec3(0.25));
}

fn blend_soft_light(back: vec3<f32>, front: vec3<f32>) -> vec3<f32> {
    let darkened = back - (vec3(1.0) - 2.0 * front) * back * (vec3(1.0) - back);
    let lightened = back + (2.0 * front - vec3(1.0)) * (soft_light_curve(back) - back);
    return select(lightened, darkened, front <= vec3(0.5));
}

fn blend_color(back: vec3<f32>, front: vec3<f32>, mode: u32) -> vec3<f32> {
    var result = front;
    switch mode {
        case 1u: {
            result = back + front;
        }
        case 2u: {
            result = blend_screen(back, front);
        }
        case 3u: {
            result = back * front;
        }
        case 4u: {
            result = abs(back - front);
        }
        case 5u: {
            result = max(back, front);
        }
        case 6u: {
            result = min(back, front);
        }
        case 7u: {
            result = blend_hard_light(front, back);
        }
        case 8u: {
            result = blend_color_dodge(back, front);
        }
        case 9u: {
            result = blend_color_burn(back, front);
        }
        case 10u: {
            result = blend_hard_light(back, front);
        }
        case 11u: {
            result = blend_soft_light(back, front);
        }
        case 12u: {
            result = back + front - 2.0 * back * front;
        }
        case 13u: {
            result = back + front - vec3(1.0);
        }
        case 14u: {
            let burn = blend_color_burn(back, 2.0 * front);
            let dodge = blend_color_dodge(back, 2.0 * (front - vec3(0.5)));
            result = select(dodge, burn, front <= vec3(0.5));
        }
        case 15u: {
            result = back + 2.0 * front - vec3(1.0);
        }
        case 16u: {
            let darker = min(back, 2.0 * front);
            let lighter = max(back, 2.0 * front - vec3(1.0));
            result = select(lighter, darker, front <= vec3(0.5));
        }
        case 17u: {
            result = select(vec3(0.0), vec3(1.0), (back + front) >= vec3(1.0));
        }
        case 18u: {
            result = back - front;
        }
        case 19u: {
            result = back / max(front, vec3(0.00001));
        }
        case 20u: {
            result = set_lum(set_sat(front, blend_sat(back)), blend_lum(back));
        }
        case 21u: {
            result = set_lum(set_sat(back, blend_sat(front)), blend_lum(back));
        }
        case 22u: {
            result = set_lum(front, blend_lum(back));
        }
        case 23u: {
            result = set_lum(back, blend_lum(front));
        }
        case 24u: {
            result = select(front, back, blend_lum(back) < blend_lum(front));
        }
        case 25u: {
            result = select(front, back, blend_lum(back) > blend_lum(front));
        }
        // --- Signature modes. Destructive by design; they are instruments,
        // --- not correctness-preserving compositing operators.
        case 26u: {
            // Negation: difference that reflects off black instead of
            // reaching it, so overlapping darks stay luminous.
            result = vec3(1.0) - abs(vec3(1.0) - back - front);
        }
        case 27u: {
            // The layer's brightness drives how far the backdrop inverts,
            // so bright footage punches a photographic negative through.
            result = mix(back, vec3(1.0) - back, blend_lum(front));
        }
        case 28u: {
            result = min(vec3(1.0), back * back / max(vec3(1.0) - front, vec3(0.00001)));
        }
        case 29u: {
            result = min(vec3(1.0), front * front / max(vec3(1.0) - back, vec3(0.00001)));
        }
        case 30u: {
            result = min(back, front) - max(back, front) + vec3(1.0);
        }
        case 31u: {
            // Rotates the backdrop around the grey axis by the layer's own hue
            // angle: identity on greyscale layers, a spinning colour wheel on
            // saturated ones.
            result = hue_rotate(back, hue_angle(front));
        }
        case 32u: {
            // Triangle-wave folding. The layer sets the fold count per channel,
            // so a gradient becomes contour bands that march with the footage.
            let folds = vec3(1.0) + front * 7.0;
            result = abs(fract(back * folds * 0.5) * 2.0 - vec3(1.0));
        }
        case 33u: {
            // Both layers quantise to 5 bits and exclusive-or. Neighbouring
            // input values land far apart, which is the point.
            let levels = 31.0;
            let back_code = vec3<u32>(clamp(back, vec3(0.0), vec3(1.0)) * levels + vec3(0.5));
            let front_code = vec3<u32>(clamp(front, vec3(0.0), vec3(1.0)) * levels + vec3(0.5));
            result = vec3<f32>(back_code ^ front_code) / levels;
        }
        case 34u: {
            // Solarisation with a per-channel threshold supplied by the layer.
            result = select(back, vec3(1.0) - back, back > front);
        }
        default: {
            result = front;
        }
    }
    return clamp(result, vec3(0.0), vec3(1.0));
}

fn composite(back: vec4<f32>, straight_front: vec4<f32>, level: f32, mode: u32) -> vec4<f32> {
    let front_alpha = clamp(straight_front.a * level, 0.0, 1.0);
    let back_alpha = clamp(back.a, 0.0, 1.0);
    let back_color = back.rgb / max(back_alpha, 0.00001);
    let blended = blend_color(back_color, straight_front.rgb, mode);
    let rgb = (1.0 - front_alpha) * back.rgb
        + front_alpha * ((1.0 - back_alpha) * straight_front.rgb + back_alpha * blended);
    let alpha = front_alpha + back_alpha * (1.0 - front_alpha);
    return vec4(rgb, alpha);
}

fn layer_uv(
    input_uv: vec2<f32>,
    position: vec2<f32>,
    scale: f32,
    rotation: f32,
    flip_horizontal: u32,
    flip_vertical: u32,
    source_aspect: f32,
    crop: vec4<f32>,
    source_mode: u32,
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
    var uv = local + vec2(0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) {
        return vec3(uv, 0.0);
    }

    let crop_min = vec2(crop.x, crop.z);
    let crop_size = max(vec2(1.0 - crop.x - crop.y, 1.0 - crop.z - crop.w), vec2(0.02));
    let cropped_aspect = source_aspect * crop_size.x / crop_size.y;
    if source_mode == 0u {
        var content_scale = vec2(1.0);
        if globals.output_aspect > cropped_aspect {
            content_scale.x = cropped_aspect / globals.output_aspect;
        } else {
            content_scale.y = globals.output_aspect / cropped_aspect;
        }
        uv = (uv - vec2(0.5)) / content_scale + vec2(0.5);
        if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) {
            return vec3(uv, 0.0);
        }
    } else if source_mode == 1u {
        var sample_scale = vec2(1.0);
        if globals.output_aspect > cropped_aspect {
            sample_scale.y = cropped_aspect / globals.output_aspect;
        } else {
            sample_scale.x = globals.output_aspect / cropped_aspect;
        }
        uv = (uv - vec2(0.5)) * sample_scale + vec2(0.5);
    }
    return vec3(crop_min + uv * crop_size, 1.0);
}

fn geometry_effect_uv(input_uv: vec2<f32>, effect: EffectConfig) -> vec2<f32> {
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

fn effect_uv_slot(input_uv: vec2<f32>, effect: EffectConfig, slot: u32) -> vec2<f32> {
    if effect.slot_enabled[slot] == 0u || effect.slot_groups[slot] != 0u {
        return input_uv;
    }
    let effected = geometry_effect_uv(input_uv, effect);
    return mix(input_uv, effected, effect.slot_mix[slot]);
}

fn effect_uv(input_uv: vec2<f32>, effect: EffectConfig) -> vec2<f32> {
    var uv = effect_uv_slot(input_uv, effect, 0u);
    uv = effect_uv_slot(uv, effect, 1u);
    return effect_uv_slot(uv, effect, 2u);
}

// Bloom is gathered inside the composite pass rather than as a separate
// bright-pass/blur pipeline: the mixer samples every deck once already, and a
// per-deck ping-pong chain would cost four more render targets at composition
// resolution. A 16-tap golden-angle disc is a coarser blur than a true
// separable Gaussian, but at these radii the difference is invisible against
// moving footage.
const BLOOM_TAPS: u32 = 16u;
const GOLDEN_ANGLE: f32 = 2.39996323;
const BLOOM_FALLOFF: f32 = 2.5;

/// Isolate the light above the threshold with a quadratic knee, so bloom fades
/// in as footage brightens instead of popping on at a hard cutoff.
fn bright_pass(color: vec3<f32>, threshold: f32) -> vec3<f32> {
    let knee = max(threshold * 0.5, 0.0001);
    let luma = dot(max(color, vec3(0.0)), vec3(0.2126, 0.7152, 0.0722));
    let soft = clamp(luma - threshold + knee, 0.0, 2.0 * knee);
    let contribution = max(soft * soft / (4.0 * knee), luma - threshold);
    return max(color, vec3(0.0)) * (max(contribution, 0.0) / max(luma, 0.0001));
}

fn bloom_light(
    primary: texture_2d<f32>,
    alpha_texture: texture_2d<f32>,
    uv: vec2<f32>,
    kind: u32,
    effect: EffectConfig,
) -> vec3<f32> {
    let dimensions = max(vec2<f32>(textureDimensions(primary)), vec2(1.0));
    let texel = 1.0 / dimensions;
    // Scale the radius off the smaller dimension so a given setting spreads the
    // same distance whatever resolution the clip happens to be.
    let radius = effect.bloom_radius * 0.12 * min(dimensions.x, dimensions.y);

    var accumulated = vec3(0.0);
    var weights = vec3(0.0);
    for (var tap = 0u; tap < BLOOM_TAPS; tap = tap + 1u) {
        let step = (f32(tap) + 0.5) / f32(BLOOM_TAPS);
        let angle = f32(tap) * GOLDEN_ANGLE;
        // sqrt distributes taps evenly across the disc; without it they crowd
        // the centre and the bloom develops a hard core.
        let offset = vec2(cos(angle), sin(angle)) * sqrt(step) * radius * texel;
        let sampled = sample_source(
            primary,
            alpha_texture,
            clamp(uv + offset, vec2(0.0), vec2(1.0)),
            kind,
        );
        // Longer red falloff than blue reproduces how real diffusion spreads
        // wavelengths unevenly. Free: it only reweights taps already taken.
        let spread = BLOOM_FALLOFF * vec3(
            1.0 - 0.55 * effect.bloom_chroma,
            1.0,
            1.0 + 0.55 * effect.bloom_chroma,
        );
        let weight = exp(-step * spread);
        accumulated += bright_pass(sampled.rgb, effect.bloom_threshold) * weight * sampled.a;
        weights += weight;
    }
    return accumulated / max(weights, vec3(0.0001));
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

fn apply_color_group(color: vec4<f32>, effect: EffectConfig) -> vec4<f32> {
    var rgb = max(color.rgb, vec3(0.0));
    let luma = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3(luma), rgb, effect.saturation);
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
    return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), color.a);
}

fn apply_stylize_group(
    color: vec4<f32>,
    edge: f32,
    bloom: vec3<f32>,
    source_luma: f32,
    effect: EffectConfig,
) -> vec4<f32> {
    var rgb = max(color.rgb, vec3(0.0));
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
    if effect.bloom > 0.0001 {
        // Additive: bloom is light the lens scattered, not a colour choice.
        rgb += bloom * effect.bloom * 2.5;
    }

    var alpha = color.a;
    if effect.luma_key > 0.0001 {
        alpha *= smoothstep(effect.luma_key - 0.05, effect.luma_key + 0.05, source_luma);
    }
    return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), alpha);
}

fn apply_effect_slot(
    color: vec4<f32>,
    edge: f32,
    bloom: vec3<f32>,
    source_luma: f32,
    effect: EffectConfig,
    slot: u32,
) -> vec4<f32> {
    if effect.slot_enabled[slot] == 0u {
        return color;
    }
    var effected = color;
    if effect.slot_groups[slot] == 1u {
        effected = apply_color_group(color, effect);
    } else if effect.slot_groups[slot] == 2u {
        effected = apply_stylize_group(color, edge, bloom, source_luma, effect);
    }
    return mix(color, effected, effect.slot_mix[slot]);
}

fn apply_color_effects(
    color: vec4<f32>,
    edge: f32,
    bloom: vec3<f32>,
    effect: EffectConfig,
) -> vec4<f32> {
    let source_luma = dot(max(color.rgb, vec3(0.0)), vec3(0.2126, 0.7152, 0.0722));
    var resolved = apply_effect_slot(color, edge, bloom, source_luma, effect, 0u);
    resolved = apply_effect_slot(resolved, edge, bloom, source_luma, effect, 1u);
    return apply_effect_slot(resolved, edge, bloom, source_luma, effect, 2u);
}

fn stylize_slot_active(effect: EffectConfig, slot: u32) -> bool {
    return effect.slot_enabled[slot] != 0u
        && effect.slot_groups[slot] == 2u
        && effect.slot_mix[slot] > 0.0001;
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
    let stylize_active = stylize_slot_active(effect, 0u)
        || stylize_slot_active(effect, 1u)
        || stylize_slot_active(effect, 2u);
    if stylize_active && (effect.neon > 0.0001 || effect.find_edges > 0.0001) {
        edge = edge_strength(primary, alpha_texture, uv, kind);
    }
    var bloom = vec3(0.0);
    if stylize_active && effect.bloom > 0.0001 {
        bloom = bloom_light(primary, alpha_texture, uv, kind, effect);
    }
    return apply_color_effects(color, edge, bloom, effect);
}

fn process_layer(
    primary: texture_2d<f32>,
    alpha_texture: texture_2d<f32>,
    transformed_uv: vec3<f32>,
    kind: u32,
    level: f32,
    effect: EffectConfig,
) -> vec4<f32> {
    // A zero level cannot reach the output: `composite` scales the layer's
    // alpha by it, and the CPU already folds bypass and solo exclusion into
    // the same value. Bailing here skips the whole per-layer effect chain,
    // which matters most for the multi-tap gathers - bloom alone is 16
    // samples per deck per pixel whether or not the deck is being seen.
    if transformed_uv.z == 0.0 || level <= 0.0 {
        return vec4(0.0);
    }
    return process_source(primary, alpha_texture, transformed_uv.xy, kind, effect);
}

fn effect_config(index: u32) -> EffectConfig {
    return EffectConfig(
        globals.contrast[index], globals.saturation[index], globals.hue[index],
        globals.black_level[index], globals.white_level[index], globals.gamma[index],
        globals.pixelate[index], globals.luma_key[index], globals.neon[index],
        globals.fractal[index], globals.jitter[index], globals.find_edges[index],
        globals.bit_reduction[index], globals.blacklight[index], globals.bloom[index],
        globals.bloom_threshold[index], globals.bloom_radius[index], globals.bloom_chroma[index],
        globals.mirror[index],
        vec4(globals.effect_slot_groups_0[index], globals.effect_slot_groups_1[index], globals.effect_slot_groups_2[index], 0u),
        vec4(globals.effect_slot_enabled_0[index], globals.effect_slot_enabled_1[index], globals.effect_slot_enabled_2[index], 0u),
        vec4(globals.effect_slot_mix_0[index], globals.effect_slot_mix_1[index], globals.effect_slot_mix_2[index], 0.0),
    );
}

fn transformed_layer_uv(input_uv: vec2<f32>, index: u32, source_aspect: f32) -> vec3<f32> {
    return layer_uv(
        input_uv,
        vec2(globals.position_x[index], globals.position_y[index]),
        globals.scale[index],
        globals.rotation[index],
        globals.flip_horizontal[index],
        globals.flip_vertical[index],
        source_aspect,
        vec4(globals.crop_left[index], globals.crop_right[index], globals.crop_top[index], globals.crop_bottom[index]),
        globals.source_modes[index],
    );
}

fn process_deck_a(input_uv: vec2<f32>) -> vec4<f32> {
    let dimensions = textureDimensions(source_a);
    let aspect = f32(dimensions.x) / max(f32(dimensions.y), 1.0);
    return process_layer(source_a, alpha_a, transformed_layer_uv(input_uv, 0u, aspect), globals.source_kinds.x, globals.levels.x, effect_config(0u));
}

fn process_deck_b(input_uv: vec2<f32>) -> vec4<f32> {
    let dimensions = textureDimensions(source_b);
    let aspect = f32(dimensions.x) / max(f32(dimensions.y), 1.0);
    return process_layer(source_b, alpha_b, transformed_layer_uv(input_uv, 1u, aspect), globals.source_kinds.y, globals.levels.y, effect_config(1u));
}

fn process_deck_c(input_uv: vec2<f32>) -> vec4<f32> {
    let dimensions = textureDimensions(source_c);
    let aspect = f32(dimensions.x) / max(f32(dimensions.y), 1.0);
    return process_layer(source_c, alpha_c, transformed_layer_uv(input_uv, 2u, aspect), globals.source_kinds.z, globals.levels.z, effect_config(2u));
}

fn process_deck_d(input_uv: vec2<f32>) -> vec4<f32> {
    let dimensions = textureDimensions(source_d);
    let aspect = f32(dimensions.x) / max(f32(dimensions.y), 1.0);
    return process_layer(source_d, alpha_d, transformed_layer_uv(input_uv, 3u, aspect), globals.source_kinds.w, globals.levels.w, effect_config(3u));
}

fn composite_layers(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>) -> vec4<f32> {
    var bus_a = vec4(0.0);
    var bus_b = vec4(0.0);
    if globals.bus_assignments.x == 0u {
        bus_a = composite(bus_a, a, globals.levels.x, globals.blend_modes.x);
    } else {
        bus_b = composite(bus_b, a, globals.levels.x, globals.blend_modes.x);
    }
    if globals.bus_assignments.y == 0u {
        bus_a = composite(bus_a, b, globals.levels.y, globals.blend_modes.y);
    } else {
        bus_b = composite(bus_b, b, globals.levels.y, globals.blend_modes.y);
    }
    if globals.bus_assignments.z == 0u {
        bus_a = composite(bus_a, c, globals.levels.z, globals.blend_modes.z);
    } else {
        bus_b = composite(bus_b, c, globals.levels.z, globals.blend_modes.z);
    }
    if globals.bus_assignments.w == 0u {
        bus_a = composite(bus_a, d, globals.levels.w, globals.blend_modes.w);
    } else {
        bus_b = composite(bus_b, d, globals.levels.w, globals.blend_modes.w);
    }
    var mixed = bus_a * globals.crossfade_gains.x
        + bus_b * globals.crossfade_gains.y;
    mixed *= globals.master_opacity;
    return vec4(mixed.rgb, 1.0);
}

@fragment fn fs_deck_a(input: VertexOutput) -> @location(0) vec4<f32> { return process_deck_a(input.uv); }
@fragment fn fs_deck_b(input: VertexOutput) -> @location(0) vec4<f32> { return process_deck_b(input.uv); }
@fragment fn fs_deck_c(input: VertexOutput) -> @location(0) vec4<f32> { return process_deck_c(input.uv); }
@fragment fn fs_deck_d(input: VertexOutput) -> @location(0) vec4<f32> { return process_deck_d(input.uv); }

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if globals.blackout != 0u {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    return composite_layers(
        process_deck_a(input.uv),
        process_deck_b(input.uv),
        process_deck_c(input.uv),
        process_deck_d(input.uv),
    );
}

@fragment
fn fs_main_with_deck_overrides(input: VertexOutput) -> @location(0) vec4<f32> {
    if globals.blackout != 0u {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    var a: vec4<f32>;
    var b: vec4<f32>;
    var c: vec4<f32>;
    var d: vec4<f32>;
    if globals.deck_override_mask.x != 0u { a = textureSample(deck_override_a, source_sampler, input.uv); } else { a = process_deck_a(input.uv); }
    if globals.deck_override_mask.y != 0u { b = textureSample(deck_override_b, source_sampler, input.uv); } else { b = process_deck_b(input.uv); }
    if globals.deck_override_mask.z != 0u { c = textureSample(deck_override_c, source_sampler, input.uv); } else { c = process_deck_c(input.uv); }
    if globals.deck_override_mask.w != 0u { d = textureSample(deck_override_d, source_sampler, input.uv); } else { d = process_deck_d(input.uv); }
    return composite_layers(a, b, c, d);
}
