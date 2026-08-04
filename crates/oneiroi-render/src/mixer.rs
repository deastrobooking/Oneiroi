//! Four-source GPU compositor for unified media frames.

use std::path::PathBuf;

use bytemuck::{Pod, Zeroable};
use oneiroi_hap::CompressedPlaneFormat;
use oneiroi_media::{RgbaFrame, VideoFramePayload};
use thiserror::Error;

use crate::deck_effect::{DeckEffectPass, DeckEffectRuntime};
use crate::{CompressedTexture, DeckPackageSlot, UploadError};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MixerGlobals {
    levels: [f32; 4],
    source_kinds: [u32; 4],
    contrast: [f32; 4],
    saturation: [f32; 4],
    hue: [f32; 4],
    black_level: [f32; 4],
    white_level: [f32; 4],
    gamma: [f32; 4],
    pixelate: [f32; 4],
    luma_key: [f32; 4],
    neon: [f32; 4],
    fractal: [f32; 4],
    jitter: [f32; 4],
    find_edges: [f32; 4],
    bit_reduction: [f32; 4],
    blacklight: [f32; 4],
    bloom: [f32; 4],
    bloom_threshold: [f32; 4],
    bloom_radius: [f32; 4],
    bloom_chroma: [f32; 4],
    mirror: [u32; 4],
    effect_slot_groups_0: [u32; 4],
    effect_slot_groups_1: [u32; 4],
    effect_slot_groups_2: [u32; 4],
    effect_slot_enabled_0: [u32; 4],
    effect_slot_enabled_1: [u32; 4],
    effect_slot_enabled_2: [u32; 4],
    effect_slot_mix_0: [f32; 4],
    effect_slot_mix_1: [f32; 4],
    effect_slot_mix_2: [f32; 4],
    position_x: [f32; 4],
    position_y: [f32; 4],
    scale: [f32; 4],
    rotation: [f32; 4],
    flip_horizontal: [u32; 4],
    flip_vertical: [u32; 4],
    crop_left: [f32; 4],
    crop_right: [f32; 4],
    crop_top: [f32; 4],
    crop_bottom: [f32; 4],
    source_modes: [u32; 4],
    blend_modes: [u32; 4],
    bus_assignments: [u32; 4],
    crossfade_gains: [f32; 2],
    master_opacity: f32,
    time_seconds: f32,
    output_aspect: f32,
    blackout: u32,
    _padding_a: u32,
    _padding_b: u32,
    deck_override_mask: [u32; 4],
}

pub const EFFECT_SLOTS_PER_DECK: usize = 3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EffectGroup {
    #[default]
    Color,
    Geometry,
    Stylize,
}

impl EffectGroup {
    pub const ALL: [Self; EFFECT_SLOTS_PER_DECK] = [Self::Color, Self::Geometry, Self::Stylize];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Color => "Color + levels",
            Self::Geometry => "Geometry",
            Self::Stylize => "Stylize + key",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectSlot {
    pub group: EffectGroup,
    pub bypassed: bool,
    pub mix: f32,
}

impl EffectSlot {
    pub const fn new(group: EffectGroup) -> Self {
        Self {
            group,
            bypassed: false,
            mix: 1.0,
        }
    }

    pub fn sanitized(mut self) -> Self {
        self.mix = if self.mix.is_finite() {
            self.mix.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self
    }
}

impl Default for EffectSlot {
    fn default() -> Self {
        Self::new(EffectGroup::Color)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EffectPreset {
    #[default]
    Neutral,
    NeonNight,
    Blacklight,
    Glitch,
    Halation,
}

impl EffectPreset {
    pub const ALL: [Self; 5] = [
        Self::Neutral,
        Self::NeonNight,
        Self::Blacklight,
        Self::Glitch,
        Self::Halation,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Neutral => "Neutral",
            Self::NeonNight => "Neon night",
            Self::Blacklight => "Blacklight",
            Self::Glitch => "Glitch",
            Self::Halation => "Halation",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckEffects {
    pub slots: [EffectSlot; EFFECT_SLOTS_PER_DECK],
    /// 1.0 is neutral.
    pub contrast: f32,
    /// 1.0 is neutral, 0.0 is monochrome.
    pub saturation: f32,
    /// Hue rotation in turns.
    pub hue: f32,
    pub black_level: f32,
    pub white_level: f32,
    /// 1.0 is neutral.
    pub gamma: f32,
    /// Normalized block size. Zero disables pixelation.
    pub pixelate: f32,
    /// Luma threshold. Zero disables the key.
    pub luma_key: f32,
    pub neon: f32,
    pub fractal: f32,
    pub jitter: f32,
    pub find_edges: f32,
    pub bit_reduction: f32,
    pub blacklight: f32,
    /// Bloom intensity. Zero disables the bright-pass entirely.
    pub bloom: f32,
    /// Luminance above which a pixel contributes light to the bloom.
    pub bloom_threshold: f32,
    /// Bloom spread as a fraction of the source's smaller dimension.
    pub bloom_radius: f32,
    /// Chromatic spread. Red carries further than blue as this rises.
    pub bloom_chroma: f32,
    pub mirror: bool,
}

impl Default for DeckEffects {
    fn default() -> Self {
        Self {
            slots: [
                EffectSlot::new(EffectGroup::Geometry),
                EffectSlot::new(EffectGroup::Color),
                EffectSlot::new(EffectGroup::Stylize),
            ],
            contrast: 1.0,
            saturation: 1.0,
            hue: 0.0,
            black_level: 0.0,
            white_level: 1.0,
            gamma: 1.0,
            pixelate: 0.0,
            luma_key: 0.0,
            neon: 0.0,
            fractal: 0.0,
            jitter: 0.0,
            find_edges: 0.0,
            bit_reduction: 0.0,
            blacklight: 0.0,
            bloom: 0.0,
            bloom_threshold: 0.65,
            bloom_radius: 0.35,
            bloom_chroma: 0.0,
            mirror: false,
        }
    }
}

impl DeckEffects {
    pub fn preset(preset: EffectPreset) -> Self {
        let mut effects = Self::default();
        match preset {
            EffectPreset::Neutral => {}
            EffectPreset::NeonNight => {
                effects.contrast = 1.2;
                effects.saturation = 1.4;
                effects.neon = 0.85;
                effects.find_edges = 0.2;
            }
            EffectPreset::Blacklight => {
                effects.contrast = 1.15;
                effects.saturation = 1.35;
                effects.blacklight = 1.0;
            }
            EffectPreset::Glitch => {
                effects.fractal = 0.3;
                effects.jitter = 0.65;
                effects.bit_reduction = 0.55;
                effects.blacklight = 0.15;
            }
            // Wide, warm diffusion off the highlights only: the look of
            // bright footage through an anamorphic lens.
            EffectPreset::Halation => {
                effects.contrast = 1.1;
                effects.black_level = 0.04;
                effects.bloom = 0.8;
                effects.bloom_threshold = 0.55;
                effects.bloom_radius = 0.6;
                effects.bloom_chroma = 0.7;
            }
        }
        effects
    }

    pub fn sanitized(mut self) -> Self {
        self.slots = self.slots.map(EffectSlot::sanitized);
        self.contrast = self.contrast.clamp(0.0, 4.0);
        self.saturation = self.saturation.clamp(0.0, 4.0);
        self.hue = self.hue.clamp(-1.0, 1.0);
        self.black_level = self.black_level.clamp(0.0, 0.95);
        self.white_level = self.white_level.clamp(self.black_level + 0.01, 1.0);
        self.gamma = self.gamma.clamp(0.1, 4.0);
        self.pixelate = self.pixelate.clamp(0.0, 0.5);
        self.luma_key = self.luma_key.clamp(0.0, 1.0);
        self.neon = self.neon.clamp(0.0, 1.0);
        self.fractal = self.fractal.clamp(0.0, 1.0);
        self.jitter = self.jitter.clamp(0.0, 1.0);
        self.find_edges = self.find_edges.clamp(0.0, 1.0);
        self.bit_reduction = self.bit_reduction.clamp(0.0, 1.0);
        self.blacklight = self.blacklight.clamp(0.0, 1.0);
        self.bloom = self.bloom.clamp(0.0, 1.0);
        self.bloom_threshold = self.bloom_threshold.clamp(0.0, 1.0);
        // A zero radius would collapse every tap onto the centre pixel and
        // turn bloom into a flat brightness lift, so keep a floor.
        self.bloom_radius = self.bloom_radius.clamp(0.02, 1.0);
        self.bloom_chroma = self.bloom_chroma.clamp(0.0, 1.0);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    Saw,
    SawDown,
    Square,
}

impl LfoWaveform {
    pub fn sample(self, phase: f32) -> f32 {
        let phase = phase.rem_euclid(1.0);
        match self {
            Self::Sine => (phase * std::f32::consts::TAU).sin(),
            Self::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Self::Saw => phase * 2.0 - 1.0,
            Self::SawDown => 1.0 - phase * 2.0,
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EffectTarget {
    #[default]
    Hue,
    Contrast,
    Saturation,
    BlackLevel,
    WhiteLevel,
    Gamma,
    Pixelate,
    LumaKey,
    Neon,
    Fractal,
    Jitter,
    FindEdges,
    BitReduction,
    Blacklight,
    Bloom,
    BloomThreshold,
    BloomRadius,
    BloomChroma,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectLfo {
    pub enabled: bool,
    pub direct_enabled: bool,
    pub target: EffectTarget,
    pub waveform: LfoWaveform,
    pub rate_hz: f32,
    pub tempo_sync: bool,
    pub beats_per_cycle: f32,
    pub depth: f32,
    pub phase: f32,
}

impl Default for EffectLfo {
    fn default() -> Self {
        Self {
            enabled: false,
            direct_enabled: true,
            target: EffectTarget::Hue,
            waveform: LfoWaveform::Sine,
            rate_hz: 0.25,
            tempo_sync: false,
            beats_per_cycle: 1.0,
            depth: 0.5,
            phase: 0.0,
        }
    }
}

pub const MOD_ROUTES_PER_DECK: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModulationRoute {
    pub enabled: bool,
    pub source: u8,
    pub target: EffectTarget,
    /// Bipolar route amount. Negative values invert the source.
    pub amount: f32,
}

impl Default for ModulationRoute {
    fn default() -> Self {
        Self {
            enabled: false,
            source: 0,
            target: EffectTarget::Hue,
            amount: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeckLfos {
    pub lanes: [EffectLfo; 3],
    pub routes: [ModulationRoute; MOD_ROUTES_PER_DECK],
}

impl DeckLfos {
    pub fn apply(self, effects: DeckEffects, time_seconds: f32, beat_position: f32) -> DeckEffects {
        self.apply_with_audio(effects, time_seconds, beat_position, [0.0; 5])
    }

    pub fn apply_with_audio(
        self,
        mut effects: DeckEffects,
        time_seconds: f32,
        beat_position: f32,
        audio_sources: [f32; 5],
    ) -> DeckEffects {
        let mut source_values = [0.0; 10];
        source_values[3..8].copy_from_slice(&audio_sources.map(|value| value.clamp(0.0, 1.0)));
        source_values[8] = beat_position.rem_euclid(1.0);
        source_values[9] = (beat_position / 4.0).rem_euclid(1.0);
        for (index, lfo) in self.lanes.into_iter().enumerate() {
            if !lfo.enabled {
                continue;
            }
            let cycle = if lfo.tempo_sync {
                beat_position / lfo.beats_per_cycle.clamp(0.0625, 8.0)
            } else {
                time_seconds * lfo.rate_hz.clamp(0.01, 20.0)
            };
            let value = lfo.waveform.sample(cycle + lfo.phase) * lfo.depth.clamp(0.0, 1.0);
            source_values[index] = value;
            if lfo.direct_enabled {
                modulate(&mut effects, lfo.target, value);
            }
        }
        for route in self.routes {
            if !route.enabled {
                continue;
            }
            let Some(source) = source_values.get(usize::from(route.source)) else {
                continue;
            };
            modulate(
                &mut effects,
                route.target,
                *source * route.amount.clamp(-1.0, 1.0),
            );
        }
        effects.sanitized()
    }
}

fn modulate(effects: &mut DeckEffects, target: EffectTarget, value: f32) {
    match target {
        EffectTarget::Hue => effects.hue += value * 0.5,
        EffectTarget::Contrast => effects.contrast += value * 2.0,
        EffectTarget::Saturation => effects.saturation += value * 2.0,
        EffectTarget::BlackLevel => effects.black_level += value * 0.45,
        EffectTarget::WhiteLevel => effects.white_level += value * 0.45,
        EffectTarget::Gamma => effects.gamma += value * 1.5,
        EffectTarget::Pixelate => effects.pixelate += value * 0.1,
        EffectTarget::LumaKey => effects.luma_key += value,
        EffectTarget::Neon => effects.neon += value,
        EffectTarget::Fractal => effects.fractal += value,
        EffectTarget::Jitter => effects.jitter += value,
        EffectTarget::FindEdges => effects.find_edges += value,
        EffectTarget::BitReduction => effects.bit_reduction += value,
        EffectTarget::Blacklight => effects.blacklight += value,
        EffectTarget::Bloom => effects.bloom += value,
        EffectTarget::BloomThreshold => effects.bloom_threshold += value * 0.5,
        EffectTarget::BloomRadius => effects.bloom_radius += value * 0.5,
        EffectTarget::BloomChroma => effects.bloom_chroma += value,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MixerParams {
    pub levels: [f32; 4],
    pub solo: [bool; 4],
    pub bypassed: [bool; 4],
    pub buses: [MixerBus; 4],
    pub crossfade_gains: [f32; 2],
    pub transforms: [DeckTransform; 4],
    pub blend_modes: [LayerBlendMode; 4],
    pub output_aspect: f32,
    pub effects: [DeckEffects; 4],
    pub master_opacity: f32,
    pub time_seconds: f32,
    pub blackout: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckTransform {
    /// Position in normalized output coordinates. `1.0` moves the layer by
    /// half the output width or height.
    pub position: [f32; 2],
    pub scale: f32,
    /// Clockwise rotation in turns.
    pub rotation: f32,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    /// Normalized left, right, top and bottom crop amounts.
    pub crop: [f32; 4],
    pub source_mode: SourceMode,
}

impl Default for DeckTransform {
    fn default() -> Self {
        Self {
            position: [0.0; 2],
            scale: 1.0,
            rotation: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            crop: [0.0; 4],
            source_mode: SourceMode::Stretch,
        }
    }
}

impl DeckTransform {
    pub fn sanitized(mut self) -> Self {
        self.position = self.position.map(|value| {
            if value.is_finite() {
                value.clamp(-2.0, 2.0)
            } else {
                0.0
            }
        });
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(0.05, 4.0)
        } else {
            1.0
        };
        self.rotation = if self.rotation.is_finite() {
            self.rotation.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.crop = self.crop.map(|value| {
            if value.is_finite() {
                value.clamp(0.0, 0.95)
            } else {
                0.0
            }
        });
        normalize_crop_pair(&mut self.crop, 0, 1);
        normalize_crop_pair(&mut self.crop, 2, 3);
        self
    }
}

fn normalize_crop_pair(crop: &mut [f32; 4], first: usize, second: usize) {
    let sum = crop[first] + crop[second];
    if sum > 0.98 {
        let scale = 0.98 / sum;
        crop[first] *= scale;
        crop[second] *= scale;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceMode {
    Fit,
    Fill,
    #[default]
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayerBlendMode {
    #[default]
    Normal,
    Add,
    Screen,
    Multiply,
    Difference,
    Lighten,
    Darken,
    Overlay,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Exclusion,
    LinearBurn,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    DarkerColor,
    LighterColor,
    Negation,
    Invert,
    Reflect,
    Glow,
    Phoenix,
    HueShift,
    FractalFold,
    XorCrush,
    Solarize,
}

/// How a blend mode is presented to the operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendModeGroup {
    /// Darken/lighten families every compositing tool shares.
    Standard,
    /// Contrast modes that pivot around mid grey.
    Contrast,
    /// Non-separable modes that transplant one colour component.
    Component,
    /// Destructive modes specific to this engine.
    Signature,
}

impl BlendModeGroup {
    pub const ALL: [Self; 4] = [
        Self::Standard,
        Self::Contrast,
        Self::Component,
        Self::Signature,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Contrast => "Contrast",
            Self::Component => "Component",
            Self::Signature => "Oneiroi",
        }
    }
}

impl LayerBlendMode {
    pub const ALL: [Self; 35] = [
        Self::Normal,
        Self::Add,
        Self::Screen,
        Self::Multiply,
        Self::Difference,
        Self::Lighten,
        Self::Darken,
        Self::Overlay,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::HardLight,
        Self::SoftLight,
        Self::Exclusion,
        Self::LinearBurn,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::HardMix,
        Self::Subtract,
        Self::Divide,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
        Self::DarkerColor,
        Self::LighterColor,
        Self::Negation,
        Self::Invert,
        Self::Reflect,
        Self::Glow,
        Self::Phoenix,
        Self::HueShift,
        Self::FractalFold,
        Self::XorCrush,
        Self::Solarize,
    ];

    /// Shader selector. These values are also the persisted numeric identity of
    /// a mode, so existing values must never be renumbered.
    pub const fn code(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Add => 1,
            Self::Screen => 2,
            Self::Multiply => 3,
            Self::Difference => 4,
            Self::Lighten => 5,
            Self::Darken => 6,
            Self::Overlay => 7,
            Self::ColorDodge => 8,
            Self::ColorBurn => 9,
            Self::HardLight => 10,
            Self::SoftLight => 11,
            Self::Exclusion => 12,
            Self::LinearBurn => 13,
            Self::VividLight => 14,
            Self::LinearLight => 15,
            Self::PinLight => 16,
            Self::HardMix => 17,
            Self::Subtract => 18,
            Self::Divide => 19,
            Self::Hue => 20,
            Self::Saturation => 21,
            Self::Color => 22,
            Self::Luminosity => 23,
            Self::DarkerColor => 24,
            Self::LighterColor => 25,
            Self::Negation => 26,
            Self::Invert => 27,
            Self::Reflect => 28,
            Self::Glow => 29,
            Self::Phoenix => 30,
            Self::HueShift => 31,
            Self::FractalFold => 32,
            Self::XorCrush => 33,
            Self::Solarize => 34,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Add => "Add",
            Self::Screen => "Screen",
            Self::Multiply => "Multiply",
            Self::Difference => "Difference",
            Self::Lighten => "Lighten",
            Self::Darken => "Darken",
            Self::Overlay => "Overlay",
            Self::ColorDodge => "Color Dodge",
            Self::ColorBurn => "Color Burn",
            Self::HardLight => "Hard Light",
            Self::SoftLight => "Soft Light",
            Self::Exclusion => "Exclusion",
            Self::LinearBurn => "Linear Burn",
            Self::VividLight => "Vivid Light",
            Self::LinearLight => "Linear Light",
            Self::PinLight => "Pin Light",
            Self::HardMix => "Hard Mix",
            Self::Subtract => "Subtract",
            Self::Divide => "Divide",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
            Self::DarkerColor => "Darker Color",
            Self::LighterColor => "Lighter Color",
            Self::Negation => "Negation",
            Self::Invert => "Invert",
            Self::Reflect => "Reflect",
            Self::Glow => "Glow",
            Self::Phoenix => "Phoenix",
            Self::HueShift => "Hue Shift",
            Self::FractalFold => "Fractal Fold",
            Self::XorCrush => "Xor Crush",
            Self::Solarize => "Solarize",
        }
    }

    /// Stable identifier used by the typed project graph. Never rename
    /// these; they are written into saved documents.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Add => "add",
            Self::Screen => "screen",
            Self::Multiply => "multiply",
            Self::Difference => "difference",
            Self::Lighten => "lighten",
            Self::Darken => "darken",
            Self::Overlay => "overlay",
            Self::ColorDodge => "color_dodge",
            Self::ColorBurn => "color_burn",
            Self::HardLight => "hard_light",
            Self::SoftLight => "soft_light",
            Self::Exclusion => "exclusion",
            Self::LinearBurn => "linear_burn",
            Self::VividLight => "vivid_light",
            Self::LinearLight => "linear_light",
            Self::PinLight => "pin_light",
            Self::HardMix => "hard_mix",
            Self::Subtract => "subtract",
            Self::Divide => "divide",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Color => "color",
            Self::Luminosity => "luminosity",
            Self::DarkerColor => "darker_color",
            Self::LighterColor => "lighter_color",
            Self::Negation => "negation",
            Self::Invert => "invert",
            Self::Reflect => "reflect",
            Self::Glow => "glow",
            Self::Phoenix => "phoenix",
            Self::HueShift => "hue_shift",
            Self::FractalFold => "fractal_fold",
            Self::XorCrush => "xor_crush",
            Self::Solarize => "solarize",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "normal" => Some(Self::Normal),
            "add" => Some(Self::Add),
            "screen" => Some(Self::Screen),
            "multiply" => Some(Self::Multiply),
            "difference" => Some(Self::Difference),
            "lighten" => Some(Self::Lighten),
            "darken" => Some(Self::Darken),
            "overlay" => Some(Self::Overlay),
            "color_dodge" => Some(Self::ColorDodge),
            "color_burn" => Some(Self::ColorBurn),
            "hard_light" => Some(Self::HardLight),
            "soft_light" => Some(Self::SoftLight),
            "exclusion" => Some(Self::Exclusion),
            "linear_burn" => Some(Self::LinearBurn),
            "vivid_light" => Some(Self::VividLight),
            "linear_light" => Some(Self::LinearLight),
            "pin_light" => Some(Self::PinLight),
            "hard_mix" => Some(Self::HardMix),
            "subtract" => Some(Self::Subtract),
            "divide" => Some(Self::Divide),
            "hue" => Some(Self::Hue),
            "saturation" => Some(Self::Saturation),
            "color" => Some(Self::Color),
            "luminosity" => Some(Self::Luminosity),
            "darker_color" => Some(Self::DarkerColor),
            "lighter_color" => Some(Self::LighterColor),
            "negation" => Some(Self::Negation),
            "invert" => Some(Self::Invert),
            "reflect" => Some(Self::Reflect),
            "glow" => Some(Self::Glow),
            "phoenix" => Some(Self::Phoenix),
            "hue_shift" => Some(Self::HueShift),
            "fractal_fold" => Some(Self::FractalFold),
            "xor_crush" => Some(Self::XorCrush),
            "solarize" => Some(Self::Solarize),
            _ => None,
        }
    }

    pub const fn group(self) -> BlendModeGroup {
        match self {
            Self::Normal
            | Self::Add
            | Self::Screen
            | Self::Multiply
            | Self::Lighten
            | Self::Darken
            | Self::ColorDodge
            | Self::ColorBurn
            | Self::LinearBurn
            | Self::Subtract
            | Self::Divide
            | Self::DarkerColor
            | Self::LighterColor => BlendModeGroup::Standard,
            Self::Difference
            | Self::Overlay
            | Self::HardLight
            | Self::SoftLight
            | Self::Exclusion
            | Self::VividLight
            | Self::LinearLight
            | Self::PinLight
            | Self::HardMix => BlendModeGroup::Contrast,
            Self::Hue | Self::Saturation | Self::Color | Self::Luminosity => {
                BlendModeGroup::Component
            }
            Self::Negation
            | Self::Invert
            | Self::Reflect
            | Self::Glow
            | Self::Phoenix
            | Self::HueShift
            | Self::FractalFold
            | Self::XorCrush
            | Self::Solarize => BlendModeGroup::Signature,
        }
    }

    /// One-line description of what the mode does, shown as a tooltip.
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Normal => "Replaces the backdrop.",
            Self::Add => "Sums light. Clips to white.",
            Self::Screen => "Inverse multiply. Lightens without clipping.",
            Self::Multiply => "Darkens by multiplying light.",
            Self::Difference => "Absolute channel distance.",
            Self::Lighten => "Keeps the brighter channel.",
            Self::Darken => "Keeps the darker channel.",
            Self::Overlay => "Multiply in shadows, screen in highlights.",
            Self::ColorDodge => "Brightens the backdrop toward white.",
            Self::ColorBurn => "Darkens the backdrop toward black.",
            Self::HardLight => "Overlay judged by the layer instead of the backdrop.",
            Self::SoftLight => "Gentle dodge and burn.",
            Self::Exclusion => "Difference with a soft mid-tone rolloff.",
            Self::LinearBurn => "Sums and subtracts white. Crushes shadows.",
            Self::VividLight => "Burns below mid grey, dodges above.",
            Self::LinearLight => "Linear burn below mid grey, linear dodge above.",
            Self::PinLight => "Replaces by distance from mid grey.",
            Self::HardMix => "Snaps every channel to zero or one.",
            Self::Subtract => "Removes the layer's light.",
            Self::Divide => "Divides the backdrop. Blows out fast.",
            Self::Hue => "Layer hue, backdrop saturation and luminosity.",
            Self::Saturation => "Layer saturation, backdrop hue and luminosity.",
            Self::Color => "Layer hue and saturation, backdrop luminosity.",
            Self::Luminosity => "Layer luminosity, backdrop hue and saturation.",
            Self::DarkerColor => "Keeps whichever whole colour is darker.",
            Self::LighterColor => "Keeps whichever whole colour is brighter.",
            Self::Negation => "Difference inverted. Never reaches black.",
            Self::Invert => "Layer brightness inverts the backdrop.",
            Self::Reflect => "Backdrop reflects the layer into blown highlights.",
            Self::Glow => "Reflect with the operands swapped.",
            Self::Phoenix => "Channel range inverted. Flat pastel separation.",
            Self::HueShift => "Rotates backdrop hue by the layer's hue angle.",
            Self::FractalFold => "Folds channels through a triangle wave the layer drives.",
            Self::XorCrush => "Quantises both layers and exclusive-ors the codes.",
            Self::Solarize => "Inverts wherever the backdrop outshines the layer.",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MixerBus {
    #[default]
    A,
    B,
}

impl Default for MixerParams {
    fn default() -> Self {
        Self {
            levels: [1.0; 4],
            solo: [false; 4],
            bypassed: [false; 4],
            buses: [MixerBus::A; 4],
            crossfade_gains: [1.0, 0.0],
            transforms: [DeckTransform::default(); 4],
            blend_modes: [LayerBlendMode::Normal; 4],
            output_aspect: 1.0,
            effects: [DeckEffects::default(); 4],
            master_opacity: 1.0,
            time_seconds: 0.0,
            blackout: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum MixerUploadError {
    #[error("deck index {0} is outside the four-deck mixer")]
    InvalidDeck(usize),
    #[error("RGBA frame data has {actual} bytes; expected {expected}")]
    RgbaSize { actual: usize, expected: usize },
    #[error("RGBA frame extent is invalid")]
    InvalidRgbaExtent,
    #[error("upload HAP texture: {0}")]
    Compressed(#[from] UploadError),
    #[error("unsupported HAP plane combination")]
    UnsupportedHapPlanes,
}

enum TextureResource {
    Rgba {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        extent: [u32; 2],
    },
    Compressed(CompressedTexture),
}

impl TextureResource {
    fn view(&self) -> &wgpu::TextureView {
        match self {
            Self::Rgba { view, .. } => view,
            Self::Compressed(texture) => &texture.view,
        }
    }

    fn extent(&self) -> [u32; 2] {
        match self {
            Self::Rgba { extent, .. } => *extent,
            Self::Compressed(texture) => texture.visible_extent,
        }
    }
}

struct DeckSource {
    primary: TextureResource,
    secondary: Option<TextureResource>,
    kind: u32,
}

struct DeckPackageTargets {
    extent: [u32; 2],
    _input_texture: wgpu::Texture,
    input_view: wgpu::TextureView,
    _output_textures: [wgpu::Texture; 4],
    output_views: [wgpu::TextureView; 4],
    override_bind_group: wgpu::BindGroup,
    passes: [DeckEffectPass; 4],
}

pub struct FourDeckCompositor {
    pipeline: wgpu::RenderPipeline,
    deck_override_pipeline: wgpu::RenderPipeline,
    deck_layer_pipelines: [wgpu::RenderPipeline; 4],
    layout: wgpu::BindGroupLayout,
    deck_override_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    transparent: TextureResource,
    opaque_alpha: TextureResource,
    sources: [Option<DeckSource>; 4],
    bind_group: Option<wgpu::BindGroup>,
    deck_effects: DeckEffectRuntime,
    deck_package_targets: Option<DeckPackageTargets>,
    output_format: wgpu::TextureFormat,
}

impl FourDeckCompositor {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("four-deck-mixer"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mixer.wgsl").into()),
        });
        let mut entries = Vec::with_capacity(10);
        entries.push(sampler_layout_entry(0));
        entries.extend((1..=8).map(texture_layout_entry));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 9,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("four-deck-mixer-layout"),
            entries: &entries,
        });
        let deck_override_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("four-deck-mixer-override-layout"),
                entries: &(0..4).map(texture_layout_entry).collect::<Vec<_>>(),
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("four-deck-mixer-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let override_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("four-deck-mixer-override-pipeline-layout"),
                bind_group_layouts: &[Some(&layout), Some(&deck_override_layout)],
                immediate_size: 0,
            });
        let pipeline = create_mixer_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "fs_main",
            output_format,
            "four-deck-mixer-pipeline",
        );
        let deck_override_pipeline = create_mixer_pipeline(
            device,
            &override_pipeline_layout,
            &shader,
            "fs_main_with_deck_overrides",
            output_format,
            "four-deck-mixer-override-pipeline",
        );
        let deck_layer_pipelines =
            ["fs_deck_a", "fs_deck_b", "fs_deck_c", "fs_deck_d"].map(|entry| {
                create_mixer_pipeline(
                    device,
                    &pipeline_layout,
                    &shader,
                    entry,
                    output_format,
                    "oneiroi-deck-precomposition-pipeline",
                )
            });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("four-deck-mixer-globals"),
            size: size_of::<MixerGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("four-deck-mixer-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let transparent = solid_texture(device, queue, [0, 0, 0, 0], "transparent-source");
        let opaque_alpha = solid_texture(device, queue, [255; 4], "opaque-alpha");
        let deck_effects = DeckEffectRuntime::new(device, output_format);

        Self {
            pipeline,
            deck_override_pipeline,
            deck_layer_pipelines,
            layout,
            deck_override_layout,
            sampler,
            globals,
            transparent,
            opaque_alpha,
            sources: std::array::from_fn(|_| None),
            bind_group: None,
            deck_effects,
            deck_package_targets: None,
            output_format,
        }
    }

    /// Allocate the fixed one-input/four-output deck package budget at a
    /// composition resize boundary. No package texture is created in `draw`.
    pub fn set_output_extent(&mut self, device: &wgpu::Device, extent: [u32; 2]) {
        let extent = [extent[0].max(1), extent[1].max(1)];
        if self
            .deck_package_targets
            .as_ref()
            .is_some_and(|targets| targets.extent == extent)
        {
            return;
        }
        let make_texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.output_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let input_texture = make_texture("oneiroi-deck-package-input");
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let output_textures = std::array::from_fn(|index| {
            make_texture(match index {
                0 => "oneiroi-deck-a-package-output",
                1 => "oneiroi-deck-b-package-output",
                2 => "oneiroi-deck-c-package-output",
                _ => "oneiroi-deck-d-package-output",
            })
        });
        let output_views = output_textures
            .each_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let override_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("four-deck-mixer-override-bind-group"),
            layout: &self.deck_override_layout,
            entries: &[
                texture_entry(0, &output_views[0]),
                texture_entry(1, &output_views[1]),
                texture_entry(2, &output_views[2]),
                texture_entry(3, &output_views[3]),
            ],
        });
        let passes = self.deck_effects.create_passes(device, &input_view);
        self.deck_package_targets = Some(DeckPackageTargets {
            extent,
            _input_texture: input_texture,
            input_view,
            _output_textures: output_textures,
            output_views,
            override_bind_group,
            passes,
        });
    }

    pub fn watch_deck_effect_manifests(&mut self, paths: Vec<PathBuf>) {
        self.deck_effects.watch_manifests(paths);
    }

    pub fn poll_deck_effect_reload(&mut self) -> bool {
        self.deck_effects.poll_reload()
    }

    pub fn deck_effect_reload_status(&self) -> &str {
        self.deck_effects.status()
    }

    pub fn deck_effect_loaded(&self, id: &str) -> bool {
        self.deck_effects.is_loaded(id)
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        deck: usize,
        payload: &VideoFramePayload,
    ) -> Result<(), MixerUploadError> {
        if deck >= 4 {
            return Err(MixerUploadError::InvalidDeck(deck));
        }
        if let VideoFramePayload::Rgba8(frame) = payload
            && let Some(DeckSource {
                primary:
                    TextureResource::Rgba {
                        texture, extent, ..
                    },
                secondary: None,
                kind: 0,
            }) = self.sources[deck].as_mut()
            && *extent == frame.extent
        {
            validate_rgba(frame)?;
            write_rgba(queue, texture, frame);
            return Ok(());
        }
        if let VideoFramePayload::BlockCompressed(frame) = payload
            && let Some(source) = self.sources[deck].as_mut()
            && update_hap_source(queue, source, frame)?
        {
            return Ok(());
        }
        let source = match payload {
            VideoFramePayload::Rgba8(frame) => DeckSource {
                primary: upload_rgba(device, queue, frame)?,
                secondary: None,
                kind: 0,
            },
            VideoFramePayload::BlockCompressed(frame) => {
                let color = frame
                    .planes
                    .iter()
                    .find(|plane| plane.format != CompressedPlaneFormat::Bc4Alpha)
                    .or_else(|| frame.planes.first())
                    .ok_or(MixerUploadError::UnsupportedHapPlanes)?;
                let alpha = frame
                    .planes
                    .iter()
                    .find(|plane| plane.format == CompressedPlaneFormat::Bc4Alpha);
                let kind = match (color.format, alpha) {
                    (CompressedPlaneFormat::Bc3ScaledYCoCg, Some(_)) => 3,
                    (CompressedPlaneFormat::Bc3ScaledYCoCg, None) => 1,
                    (CompressedPlaneFormat::Bc4Alpha, _) => 2,
                    (_, None) if frame.planes.len() == 1 => 0,
                    _ => return Err(MixerUploadError::UnsupportedHapPlanes),
                };
                DeckSource {
                    primary: TextureResource::Compressed(CompressedTexture::upload(
                        device,
                        queue,
                        color,
                        Some("hap-primary"),
                    )?),
                    secondary: alpha
                        .map(|plane| {
                            CompressedTexture::upload(device, queue, plane, Some("hap-alpha"))
                                .map(TextureResource::Compressed)
                        })
                        .transpose()?,
                    kind,
                }
            }
        };
        self.sources[deck] = Some(source);
        self.bind_group = None;
        Ok(())
    }

    pub fn clear_deck(&mut self, deck: usize) {
        if deck < 4 {
            self.sources[deck] = None;
            self.bind_group = None;
        }
    }

    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        params: MixerParams,
    ) {
        let deck_packages = std::array::from_fn(|_| DeckPackageSlot::default());
        self.draw_with_deck_packages(device, queue, encoder, target, params, &deck_packages);
    }

    pub fn draw_with_deck_packages(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        params: MixerParams,
        deck_packages: &[DeckPackageSlot; 4],
    ) {
        let kinds = std::array::from_fn(|index| {
            self.sources[index].as_ref().map_or(0, |source| source.kind)
        });
        let effects = params.effects.map(DeckEffects::sanitized);
        let transforms = params.transforms.map(DeckTransform::sanitized);
        let any_solo = params.solo.into_iter().any(|solo| solo);
        let levels = std::array::from_fn(|index| {
            if params.bypassed[index] || (any_solo && !params.solo[index]) {
                0.0
            } else {
                params.levels[index]
            }
        });
        let targets_ready = self.deck_package_targets.is_some();
        let deck_override_mask = std::array::from_fn(|index| {
            u32::from(
                targets_ready
                    && !params.blackout
                    && levels[index] > 0.0
                    && self.sources[index].is_some()
                    && deck_packages[index].active()
                    && self
                        .deck_effects
                        .is_loaded(&deck_packages[index].package_id),
            )
        });
        let float_values = |read: fn(DeckEffects) -> f32| effects.map(read);
        let slot_groups = |slot: usize| {
            effects.map(|effect| match effect.slots[slot].group {
                EffectGroup::Geometry => 0,
                EffectGroup::Color => 1,
                EffectGroup::Stylize => 2,
            })
        };
        let slot_enabled =
            |slot: usize| effects.map(|effect| u32::from(!effect.slots[slot].bypassed));
        let slot_mix = |slot: usize| effects.map(|effect| effect.slots[slot].mix);
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&MixerGlobals {
                levels,
                source_kinds: kinds,
                contrast: float_values(|effect| effect.contrast),
                saturation: float_values(|effect| effect.saturation),
                hue: float_values(|effect| effect.hue),
                black_level: float_values(|effect| effect.black_level),
                white_level: float_values(|effect| effect.white_level),
                gamma: float_values(|effect| effect.gamma),
                pixelate: float_values(|effect| effect.pixelate),
                luma_key: float_values(|effect| effect.luma_key),
                neon: float_values(|effect| effect.neon),
                fractal: float_values(|effect| effect.fractal),
                jitter: float_values(|effect| effect.jitter),
                find_edges: float_values(|effect| effect.find_edges),
                bit_reduction: float_values(|effect| effect.bit_reduction),
                blacklight: float_values(|effect| effect.blacklight),
                bloom: float_values(|effect| effect.bloom),
                bloom_threshold: float_values(|effect| effect.bloom_threshold),
                bloom_radius: float_values(|effect| effect.bloom_radius),
                bloom_chroma: float_values(|effect| effect.bloom_chroma),
                mirror: effects.map(|effect| u32::from(effect.mirror)),
                effect_slot_groups_0: slot_groups(0),
                effect_slot_groups_1: slot_groups(1),
                effect_slot_groups_2: slot_groups(2),
                effect_slot_enabled_0: slot_enabled(0),
                effect_slot_enabled_1: slot_enabled(1),
                effect_slot_enabled_2: slot_enabled(2),
                effect_slot_mix_0: slot_mix(0),
                effect_slot_mix_1: slot_mix(1),
                effect_slot_mix_2: slot_mix(2),
                position_x: transforms.map(|transform| transform.position[0]),
                position_y: transforms.map(|transform| transform.position[1]),
                scale: transforms.map(|transform| transform.scale),
                rotation: transforms.map(|transform| transform.rotation),
                flip_horizontal: transforms.map(|transform| u32::from(transform.flip_horizontal)),
                flip_vertical: transforms.map(|transform| u32::from(transform.flip_vertical)),
                crop_left: transforms.map(|transform| transform.crop[0]),
                crop_right: transforms.map(|transform| transform.crop[1]),
                crop_top: transforms.map(|transform| transform.crop[2]),
                crop_bottom: transforms.map(|transform| transform.crop[3]),
                source_modes: transforms.map(|transform| match transform.source_mode {
                    SourceMode::Fit => 0,
                    SourceMode::Fill => 1,
                    SourceMode::Stretch => 2,
                }),
                blend_modes: params.blend_modes.map(LayerBlendMode::code),
                bus_assignments: params.buses.map(|bus| match bus {
                    MixerBus::A => 0,
                    MixerBus::B => 1,
                }),
                crossfade_gains: params.crossfade_gains.map(|gain| gain.clamp(0.0, 1.0)),
                master_opacity: params.master_opacity.clamp(0.0, 1.0),
                time_seconds: params.time_seconds,
                output_aspect: if params.output_aspect.is_finite() {
                    params.output_aspect.clamp(0.01, 100.0)
                } else {
                    1.0
                },
                blackout: u32::from(params.blackout),
                _padding_a: 0,
                _padding_b: 0,
                deck_override_mask,
            }),
        );
        if self.bind_group.is_none() {
            self.bind_group = Some(self.create_bind_group(device));
        }
        if deck_override_mask.into_iter().any(|active| active != 0) {
            let source_bind_group = self.bind_group.as_ref().unwrap();
            let targets = self.deck_package_targets.as_ref().unwrap();
            for index in 0..4 {
                if deck_override_mask[index] == 0 {
                    continue;
                }
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("oneiroi-deck-precomposition-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &targets.input_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&self.deck_layer_pipelines[index]);
                    pass.set_bind_group(0, source_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                let source_extent = self.sources[index]
                    .as_ref()
                    .map_or([1, 1], |source| source.primary.extent());
                let package_executed = self.deck_effects.draw(
                    queue,
                    encoder,
                    &targets.passes[index],
                    &targets.output_views[index],
                    &deck_packages[index],
                    index,
                    source_extent,
                    targets.extent,
                    params.time_seconds,
                );
                debug_assert!(package_executed);
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("four-deck-mixer-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let uses_deck_overrides = deck_override_mask.into_iter().any(|active| active != 0);
        pass.set_pipeline(if uses_deck_overrides {
            &self.deck_override_pipeline
        } else {
            &self.pipeline
        });
        pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
        if uses_deck_overrides {
            pass.set_bind_group(
                1,
                &self
                    .deck_package_targets
                    .as_ref()
                    .unwrap()
                    .override_bind_group,
                &[],
            );
        }
        pass.draw(0..3, 0..1);
    }

    fn create_bind_group(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        let primary = std::array::from_fn::<_, 4, _>(|i| self.primary_view(i));
        let alpha = std::array::from_fn::<_, 4, _>(|i| self.alpha_view(i));
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("four-deck-mixer-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                texture_entry(1, primary[0]),
                texture_entry(2, primary[1]),
                texture_entry(3, primary[2]),
                texture_entry(4, primary[3]),
                texture_entry(5, alpha[0]),
                texture_entry(6, alpha[1]),
                texture_entry(7, alpha[2]),
                texture_entry(8, alpha[3]),
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: self.globals.as_entire_binding(),
                },
            ],
        })
    }

    fn primary_view(&self, index: usize) -> &wgpu::TextureView {
        self.sources[index]
            .as_ref()
            .map_or(self.transparent.view(), |source| source.primary.view())
    }

    fn alpha_view(&self, index: usize) -> &wgpu::TextureView {
        self.sources[index]
            .as_ref()
            .and_then(|source| source.secondary.as_ref())
            .map_or(self.opaque_alpha.view(), TextureResource::view)
    }
}

fn create_mixer_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &str,
    output_format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn update_hap_source(
    queue: &wgpu::Queue,
    source: &mut DeckSource,
    frame: &oneiroi_hap::DecodedFrame,
) -> Result<bool, MixerUploadError> {
    let color = frame
        .planes
        .iter()
        .find(|plane| plane.format != CompressedPlaneFormat::Bc4Alpha)
        .or_else(|| frame.planes.first())
        .ok_or(MixerUploadError::UnsupportedHapPlanes)?;
    let alpha = frame
        .planes
        .iter()
        .find(|plane| plane.format == CompressedPlaneFormat::Bc4Alpha);
    let kind = match (color.format, alpha) {
        (CompressedPlaneFormat::Bc3ScaledYCoCg, Some(_)) => 3,
        (CompressedPlaneFormat::Bc3ScaledYCoCg, None) => 1,
        (CompressedPlaneFormat::Bc4Alpha, _) => 2,
        (_, None) if frame.planes.len() == 1 => 0,
        _ => return Err(MixerUploadError::UnsupportedHapPlanes),
    };
    if source.kind != kind {
        return Ok(false);
    }
    let TextureResource::Compressed(primary) = &mut source.primary else {
        return Ok(false);
    };
    if !primary.update(queue, color)? {
        return Ok(false);
    }
    match (alpha, source.secondary.as_mut()) {
        (None, None) => Ok(true),
        (Some(plane), Some(TextureResource::Compressed(secondary))) => {
            secondary.update(queue, plane).map_err(Into::into)
        }
        _ => Ok(false),
    }
}

fn sampler_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn solid_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixel: [u8; 4],
    label: &str,
) -> TextureResource {
    upload_rgba(
        device,
        queue,
        &RgbaFrame {
            extent: [1, 1],
            data: pixel.to_vec().into(),
        },
    )
    .unwrap_or_else(|_| panic!("create {label}"))
}

fn upload_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &RgbaFrame,
) -> Result<TextureResource, MixerUploadError> {
    validate_rgba(frame)?;
    let [width, height] = frame.extent;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rgba-deck-source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_rgba(queue, &texture, frame);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(TextureResource::Rgba {
        texture,
        view,
        extent: frame.extent,
    })
}

fn validate_rgba(frame: &RgbaFrame) -> Result<(), MixerUploadError> {
    let [width, height] = frame.extent;
    let expected = usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(4))
        .and_then(|row| usize::try_from(height).ok()?.checked_mul(row))
        .ok_or(MixerUploadError::InvalidRgbaExtent)?;
    if width == 0 || height == 0 {
        return Err(MixerUploadError::InvalidRgbaExtent);
    }
    if frame.data.len() != expected {
        return Err(MixerUploadError::RgbaSize {
            actual: frame.data.len(),
            expected,
        });
    }
    Ok(())
}

fn write_rgba(queue: &wgpu::Queue, texture: &wgpu::Texture, frame: &RgbaFrame) {
    let [width, height] = frame.extent;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &frame.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixer_shader_parses_and_validates_without_a_gpu_adapter() {
        let module = naga::front::wgsl::parse_str(include_str!("../shaders/mixer.wgsl"))
            .expect("parse mixer shader");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("validate mixer shader");
    }

    #[test]
    fn effect_chain_defaults_presets_and_mix_sanitize() {
        let default = DeckEffects::default();
        assert_eq!(
            default.slots.map(|slot| slot.group),
            [
                EffectGroup::Geometry,
                EffectGroup::Color,
                EffectGroup::Stylize
            ]
        );

        let neon = DeckEffects::preset(EffectPreset::NeonNight);
        assert!(neon.neon > 0.0);
        assert!(neon.saturation > 1.0);

        let mut invalid = default;
        invalid.slots[0].mix = f32::NAN;
        invalid.slots[1].mix = -1.0;
        invalid.slots[2].mix = 2.0;
        let sanitized = invalid.sanitized();
        assert_eq!(sanitized.slots[0].mix, 1.0);
        assert_eq!(sanitized.slots[1].mix, 0.0);
        assert_eq!(sanitized.slots[2].mix, 1.0);
    }

    #[test]
    fn lfo_waveforms_have_expected_quarter_cycle_values() {
        assert!((LfoWaveform::Sine.sample(0.25) - 1.0).abs() < 0.0001);
        assert_eq!(LfoWaveform::Triangle.sample(0.25), 0.0);
        assert_eq!(LfoWaveform::Saw.sample(0.25), -0.5);
        assert_eq!(LfoWaveform::SawDown.sample(0.25), 0.5);
        assert_eq!(LfoWaveform::Square.sample(0.25), 1.0);
        assert_eq!(LfoWaveform::Square.sample(0.75), -1.0);
    }

    #[test]
    fn enabled_lfo_modulates_a_copy_and_clamps_the_result() {
        let base = DeckEffects::default();
        let lfos = DeckLfos {
            lanes: [
                EffectLfo {
                    enabled: true,
                    direct_enabled: true,
                    target: EffectTarget::Hue,
                    waveform: LfoWaveform::Sine,
                    rate_hz: 1.0,
                    tempo_sync: false,
                    beats_per_cycle: 1.0,
                    depth: 1.0,
                    phase: 0.0,
                },
                EffectLfo {
                    enabled: true,
                    direct_enabled: true,
                    target: EffectTarget::Neon,
                    waveform: LfoWaveform::Square,
                    rate_hz: 1.0,
                    tempo_sync: false,
                    beats_per_cycle: 1.0,
                    depth: 2.0,
                    phase: 0.0,
                },
                EffectLfo::default(),
            ],
            ..Default::default()
        };
        let resolved = lfos.apply(base, 0.25, 0.0);
        assert!((resolved.hue - 0.5).abs() < 0.0001);
        assert_eq!(resolved.neon, 1.0);
        assert_eq!(base, DeckEffects::default());
    }

    #[test]
    fn matrix_routes_one_source_to_multiple_bipolar_destinations() {
        let mut lfos = DeckLfos::default();
        lfos.lanes[0] = EffectLfo {
            enabled: true,
            direct_enabled: false,
            target: EffectTarget::Hue,
            waveform: LfoWaveform::Square,
            rate_hz: 1.0,
            tempo_sync: false,
            beats_per_cycle: 1.0,
            depth: 1.0,
            phase: 0.0,
        };
        lfos.routes[0] = ModulationRoute {
            enabled: true,
            source: 0,
            target: EffectTarget::Contrast,
            amount: 0.5,
        };
        lfos.routes[1] = ModulationRoute {
            enabled: true,
            source: 0,
            target: EffectTarget::Saturation,
            amount: -0.25,
        };

        let resolved = lfos.apply(DeckEffects::default(), 0.25, 0.0);
        assert_eq!(resolved.hue, 0.0);
        assert_eq!(resolved.contrast, 2.0);
        assert_eq!(resolved.saturation, 0.5);
    }

    #[test]
    fn audio_sources_use_the_same_bipolar_route_amounts() {
        let mut lfos = DeckLfos::default();
        lfos.routes[0] = ModulationRoute {
            enabled: true,
            source: 4,
            target: EffectTarget::Neon,
            amount: 0.75,
        };
        lfos.routes[1] = ModulationRoute {
            enabled: true,
            source: 7,
            target: EffectTarget::Hue,
            amount: -0.5,
        };
        let resolved =
            lfos.apply_with_audio(DeckEffects::default(), 0.0, 0.0, [0.2, 0.8, 0.0, 0.0, 1.0]);
        assert!((resolved.neon - 0.6).abs() < 0.0001);
        assert!((resolved.hue + 0.25).abs() < 0.0001);
    }

    #[test]
    fn beat_and_bar_phase_are_matrix_sources() {
        let mut lfos = DeckLfos::default();
        lfos.routes[0] = ModulationRoute {
            enabled: true,
            source: 8,
            target: EffectTarget::Contrast,
            amount: 1.0,
        };
        lfos.routes[1] = ModulationRoute {
            enabled: true,
            source: 9,
            target: EffectTarget::Saturation,
            amount: 1.0,
        };
        let resolved = lfos.apply_with_audio(DeckEffects::default(), 0.0, 5.25, [0.0; 5]);
        assert!((resolved.contrast - 1.5).abs() < 0.0001);
        assert!((resolved.saturation - 1.625).abs() < 0.0001);
    }

    #[test]
    fn tempo_synced_lfo_uses_musical_position_not_wall_time() {
        let mut lfos = DeckLfos::default();
        lfos.lanes[0] = EffectLfo {
            enabled: true,
            direct_enabled: true,
            target: EffectTarget::Hue,
            waveform: LfoWaveform::Sine,
            rate_hz: 20.0,
            tempo_sync: true,
            beats_per_cycle: 2.0,
            depth: 1.0,
            phase: 0.0,
        };
        let first = lfos.apply(DeckEffects::default(), 100.0, 0.5);
        let later_wall_time = lfos.apply(DeckEffects::default(), 900.0, 0.5);
        assert!((first.hue - 0.5).abs() < 0.0001);
        assert_eq!(first, later_wall_time);
    }
}
