use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROJECT_FORMAT: &str = "oneiroi-project";
pub const PROJECT_VERSION: u32 = 4;
const MINIMUM_PROJECT_VERSION: u32 = 1;
pub const DECK_COUNT: usize = 4;
pub const CLIPS_PER_DECK: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub takes: Vec<TakeMetadataProject>,
    pub settings: ProjectSettings,
    pub decks: Vec<DeckProject>,
    #[serde(default)]
    pub midi_mappings: Vec<MidiMappingProject>,
}

impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            format: PROJECT_FORMAT.to_owned(),
            version: PROJECT_VERSION,
            project_id: new_project_id(),
            takes: Vec::new(),
            settings: ProjectSettings::default(),
            decks: (0..DECK_COUNT)
                .map(|index| DeckProject {
                    bus: if index.is_multiple_of(2) {
                        CrossfadeBusProject::Left
                    } else {
                        CrossfadeBusProject::Right
                    },
                    ..DeckProject::default()
                })
                .collect(),
            midi_mappings: Vec::new(),
        }
    }
}

impl ProjectFile {
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.format != PROJECT_FORMAT {
            return Err(ProjectError::WrongFormat(self.format.clone()));
        }
        if !(MINIMUM_PROJECT_VERSION..=PROJECT_VERSION).contains(&self.version) {
            return Err(ProjectError::UnsupportedVersion(self.version));
        }
        if (self.version >= 4 && !valid_identity(&self.project_id))
            || self.takes.len() > 256
            || self.takes.iter().any(|take| {
                !valid_identity(&take.take_id)
                    || take.name.is_empty()
                    || take.name.len() > 128
                    || take.journal_file.is_empty()
                    || take.journal_file.len() > 255
                    || Path::new(&take.journal_file)
                        .file_name()
                        .is_none_or(|file_name| file_name.to_string_lossy() != take.journal_file)
            })
        {
            return Err(ProjectError::InvalidValue(
                "project or take identity is invalid".to_owned(),
            ));
        }
        if self.decks.len() != DECK_COUNT {
            return Err(ProjectError::InvalidShape(format!(
                "expected {DECK_COUNT} decks, found {}",
                self.decks.len()
            )));
        }
        if !self.settings.bpm.is_finite()
            || !(20.0..=400.0).contains(&self.settings.bpm)
            || !unit(self.settings.crossfader)
            || !unit(self.settings.master_opacity)
            || !(320..=7680).contains(&self.settings.output.composition_extent[0])
            || !(180..=4320).contains(&self.settings.output.composition_extent[1])
            || !effect_value(self.settings.audio_analysis.gain, 0.0, 16.0)
            || !effect_value(self.settings.audio_analysis.noise_floor, 0.0, 0.5)
            || !effect_value(self.settings.audio_analysis.attack_ms, 1.0, 2_000.0)
            || !effect_value(self.settings.audio_analysis.release_ms, 1.0, 5_000.0)
            || !effect_value(
                self.settings.audio_analysis.transient_sensitivity,
                0.0,
                16.0,
            )
            || !effect_value(self.settings.audio_analysis.normalization_target, 0.05, 1.0)
            || !effect_value(
                self.settings.audio_analysis.normalization_speed_ms,
                10.0,
                10_000.0,
            )
            || !valid_master_effects(&self.settings.master_effects)
            || !valid_master_modulation(&self.settings.master_modulation)
        {
            return Err(ProjectError::InvalidValue(
                "master settings are outside supported ranges".to_owned(),
            ));
        }
        for (index, deck) in self.decks.iter().enumerate() {
            if deck.clips.len() != CLIPS_PER_DECK {
                return Err(ProjectError::InvalidShape(format!(
                    "deck {index} expected {CLIPS_PER_DECK} clips, found {}",
                    deck.clips.len()
                )));
            }
            if deck.clip_playback.len() != CLIPS_PER_DECK {
                return Err(ProjectError::InvalidShape(format!(
                    "deck {index} expected {CLIPS_PER_DECK} clip playback entries, found {}",
                    deck.clip_playback.len()
                )));
            }
            if deck.selected_slot >= CLIPS_PER_DECK
                || deck.active_slot.is_some_and(|slot| slot >= CLIPS_PER_DECK)
                || !unit(deck.level)
                || !deck.transport.speed.is_finite()
                || !(0.25..=4.0).contains(&deck.transport.speed)
                || !deck.transport.position.is_finite()
                || deck.transport.position < 0.0
                || deck
                    .transform
                    .position
                    .iter()
                    .any(|value| !effect_value(*value, -2.0, 2.0))
                || !effect_value(deck.transform.scale, 0.05, 4.0)
                || !effect_value(deck.transform.rotation, -1.0, 1.0)
                || deck
                    .transform
                    .crop
                    .iter()
                    .any(|value| !effect_value(*value, 0.0, 0.95))
                || deck.transform.crop[0] + deck.transform.crop[1] > 0.98
                || deck.transform.crop[2] + deck.transform.crop[3] > 0.98
                || !effect_value(deck.effects.contrast, 0.0, 4.0)
                || !effect_value(deck.effects.saturation, 0.0, 4.0)
                || !effect_value(deck.effects.hue, -1.0, 1.0)
                || !effect_value(deck.effects.black_level, 0.0, 0.95)
                || !effect_value(deck.effects.white_level, 0.01, 1.0)
                || deck.effects.white_level <= deck.effects.black_level
                || !effect_value(deck.effects.gamma, 0.1, 4.0)
                || !effect_value(deck.effects.pixelate, 0.0, 0.5)
                || !effect_value(deck.effects.luma_key, 0.0, 1.0)
                || !unit(deck.effects.neon)
                || !unit(deck.effects.fractal)
                || !unit(deck.effects.jitter)
                || !unit(deck.effects.find_edges)
                || !unit(deck.effects.bit_reduction)
                || !unit(deck.effects.blacklight)
                || !valid_effect_slots(&deck.effects.slots)
                || deck.lfos.len() > 3
                || deck.lfos.iter().any(|lfo| {
                    !effect_value(lfo.rate_hz, 0.01, 20.0)
                        || !effect_value(lfo.beats_per_cycle, 0.0625, 8.0)
                        || !unit(lfo.depth)
                        || !unit(lfo.phase)
                })
                || deck.mod_routes.len() > 8
                || deck
                    .mod_routes
                    .iter()
                    .any(|route| route.source >= 10 || !effect_value(route.amount, -1.0, 1.0))
                || deck.camera.as_ref().is_some_and(|camera| {
                    camera.device_id.is_empty()
                        || camera.requested_fps == Some(0)
                        || camera
                            .requested_extent
                            .is_some_and(|[width, height]| width == 0 || height == 0)
                })
                || (deck.camera.is_some() && deck.active_slot.is_some())
                || deck.clip_playback.iter().any(|playback| {
                    !playback.in_point.is_finite()
                        || playback.in_point < 0.0
                        || playback
                            .out_point
                            .is_some_and(|out| !out.is_finite() || out <= playback.in_point)
                        || playback.beat_duration.is_some_and(|beats| {
                            !beats.is_finite() || !(0.0625..=256.0).contains(&beats)
                        })
                })
            {
                return Err(ProjectError::InvalidValue(format!(
                    "deck {index} contains an unsupported value"
                )));
            }
        }
        for mapping in &self.midi_mappings {
            if mapping.channel > 15
                || mapping.number > 127
                || !valid_control_target(mapping.target)
                || mapping
                    .input_range
                    .iter()
                    .chain(mapping.output_range.iter())
                    .any(|value| !value.is_finite())
            {
                return Err(ProjectError::InvalidValue(
                    "MIDI mapping contains an unsupported value".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TakeMetadataProject {
    pub take_id: String,
    pub name: String,
    pub journal_file: String,
    pub created_unix_ms: u64,
}

static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(1);

pub fn new_project_id() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = u128::from(NEXT_PROJECT_ID.fetch_add(1, Ordering::Relaxed));
    let process = u128::from(std::process::id());
    format!("{:032x}", time ^ (process << 64) ^ sequence)
}

fn valid_identity(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn unit(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn effect_value(value: f32, minimum: f32, maximum: f32) -> bool {
    value.is_finite() && (minimum..=maximum).contains(&value)
}

fn valid_effect_slots(slots: &[EffectSlotProject]) -> bool {
    slots.len() == 3
        && slots.iter().all(|slot| unit(slot.mix))
        && EffectGroupProject::ALL
            .into_iter()
            .all(|group| slots.iter().filter(|slot| slot.group == group).count() == 1)
}

fn valid_master_effects(effects: &MasterEffectsProject) -> bool {
    effects.slots.len() == 2
        && effects.slots.iter().all(|slot| {
            unit(slot.mix)
                && effect_value(slot.amount, 0.0, 32.0)
                && effect_value(slot.feedback, 0.0, 0.99)
                && slot.parameters.len() <= 32
                && slot
                    .parameters
                    .iter()
                    .all(|parameter| valid_effect_id(&parameter.id) && parameter.value.is_finite())
                && {
                    let mut ids = std::collections::HashSet::new();
                    slot.parameters
                        .iter()
                        .all(|parameter| ids.insert(parameter.id.as_str()))
                }
                && (slot.kind != MasterEffectKindProject::Custom
                    || valid_effect_id(&slot.package_id))
        })
}

fn valid_master_modulation(modulation: &MasterModulationProject) -> bool {
    modulation.lfos.len() == 3
        && modulation.routes.len() == 8
        && modulation.lfos.iter().all(|lfo| {
            effect_value(lfo.rate_hz, 0.01, 20.0)
                && effect_value(lfo.beats_per_cycle, 0.0625, 8.0)
                && unit(lfo.depth)
                && unit(lfo.phase)
        })
        && modulation.routes.iter().all(|route| {
            route.source < 10
                && route.target_slot < 2
                && effect_value(route.amount, -1.0, 1.0)
                && (!route.enabled || route.parameter_key != 0)
        })
}

fn valid_effect_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_control_target(target: ControlTargetProject) -> bool {
    match target {
        ControlTargetProject::Crossfader
        | ControlTargetProject::MasterOpacity
        | ControlTargetProject::MasterBlackout
        | ControlTargetProject::MasterFreeze
        | ControlTargetProject::TapTempo => true,
        ControlTargetProject::DeckLevel { deck }
        | ControlTargetProject::DeckPlay { deck }
        | ControlTargetProject::DeckFreeze { deck }
        | ControlTargetProject::DeckSpeed { deck }
        | ControlTargetProject::DeckSelect { deck }
        | ControlTargetProject::DeckRestart { deck } => deck < 4,
        ControlTargetProject::ClipLaunch { deck, slot } => deck < 4 && slot < 8,
        ControlTargetProject::SceneLaunch { slot } => slot < 8,
        ControlTargetProject::EffectParameter {
            deck,
            effect,
            parameter,
        } => deck < 4 && effect < 14 && parameter == 0,
        ControlTargetProject::LfoParameter {
            deck,
            lfo,
            parameter,
        } => deck < 4 && lfo < 3 && parameter < 4,
        ControlTargetProject::ModRouteParameter {
            deck,
            route,
            parameter,
        } => deck < 4 && route < 8 && parameter < 2,
        ControlTargetProject::MasterEffectParameter {
            slot,
            parameter_key,
        } => slot < 2 && parameter_key != 0,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub bpm: f64,
    pub quantization: QuantizationProject,
    pub crossfader: f32,
    pub equal_power: bool,
    pub master_opacity: f32,
    #[serde(default)]
    pub output: OutputProject,
    #[serde(default)]
    pub audio_analysis: AudioAnalysisProject,
    #[serde(default)]
    pub master_effects: MasterEffectsProject,
    #[serde(default)]
    pub master_modulation: MasterModulationProject,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            quantization: QuantizationProject::Immediate,
            crossfader: 0.5,
            equal_power: true,
            master_opacity: 1.0,
            output: OutputProject::default(),
            audio_analysis: AudioAnalysisProject::default(),
            master_effects: MasterEffectsProject::default(),
            master_modulation: MasterModulationProject::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MasterEffectKindProject {
    #[default]
    None,
    Blur,
    Feedback,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterEffectSlotProject {
    pub kind: MasterEffectKindProject,
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default = "one")]
    pub mix: f32,
    #[serde(default = "default_blur_amount")]
    pub amount: f32,
    #[serde(default = "default_feedback_amount")]
    pub feedback: f32,
    #[serde(default)]
    pub package_id: String,
    #[serde(default)]
    pub parameters: Vec<EffectParameterValueProject>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectParameterValueProject {
    pub id: String,
    pub value: f32,
}

impl Default for MasterEffectSlotProject {
    fn default() -> Self {
        Self {
            kind: MasterEffectKindProject::None,
            bypassed: false,
            mix: 1.0,
            amount: default_blur_amount(),
            feedback: default_feedback_amount(),
            package_id: String::new(),
            parameters: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterEffectsProject {
    #[serde(default = "default_master_effect_slots")]
    pub slots: Vec<MasterEffectSlotProject>,
}

impl Default for MasterEffectsProject {
    fn default() -> Self {
        Self {
            slots: default_master_effect_slots(),
        }
    }
}

fn default_master_effect_slots() -> Vec<MasterEffectSlotProject> {
    vec![MasterEffectSlotProject::default(); 2]
}

fn default_blur_amount() -> f32 {
    8.0
}

fn default_feedback_amount() -> f32 {
    0.85
}

fn default_lfo_rate() -> f32 {
    0.25
}

fn default_lfo_depth() -> f32 {
    0.5
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterLfoProject {
    #[serde(default)]
    pub enabled: bool,
    pub waveform: LfoWaveformProject,
    #[serde(default = "default_lfo_rate")]
    pub rate_hz: f32,
    #[serde(default)]
    pub tempo_sync: bool,
    #[serde(default = "one")]
    pub beats_per_cycle: f32,
    #[serde(default = "default_lfo_depth")]
    pub depth: f32,
    #[serde(default)]
    pub phase: f32,
}

impl Default for MasterLfoProject {
    fn default() -> Self {
        Self {
            enabled: false,
            waveform: LfoWaveformProject::Sine,
            rate_hz: default_lfo_rate(),
            tempo_sync: false,
            beats_per_cycle: 1.0,
            depth: default_lfo_depth(),
            phase: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterModulationRouteProject {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub source: u8,
    #[serde(default)]
    pub target_slot: u8,
    #[serde(default)]
    pub parameter_key: u64,
    #[serde(default = "default_lfo_depth")]
    pub amount: f32,
}

impl Default for MasterModulationRouteProject {
    fn default() -> Self {
        Self {
            enabled: false,
            source: 0,
            target_slot: 0,
            parameter_key: 0,
            amount: default_lfo_depth(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterModulationProject {
    #[serde(default = "default_master_lfos")]
    pub lfos: Vec<MasterLfoProject>,
    #[serde(default = "default_master_modulation_routes")]
    pub routes: Vec<MasterModulationRouteProject>,
}

impl Default for MasterModulationProject {
    fn default() -> Self {
        Self {
            lfos: default_master_lfos(),
            routes: default_master_modulation_routes(),
        }
    }
}

fn default_master_lfos() -> Vec<MasterLfoProject> {
    vec![MasterLfoProject::default(); 3]
}

fn default_master_modulation_routes() -> Vec<MasterModulationRouteProject> {
    vec![MasterModulationRouteProject::default(); 8]
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioAnalysisProject {
    pub gain: f32,
    pub noise_floor: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub transient_sensitivity: f32,
    #[serde(default)]
    pub normalization: bool,
    #[serde(default = "default_normalization_target")]
    pub normalization_target: f32,
    #[serde(default = "default_normalization_speed")]
    pub normalization_speed_ms: f32,
}

impl Default for AudioAnalysisProject {
    fn default() -> Self {
        Self {
            gain: 1.0,
            noise_floor: 0.01,
            attack_ms: 20.0,
            release_ms: 180.0,
            transient_sensitivity: 2.0,
            normalization: false,
            normalization_target: default_normalization_target(),
            normalization_speed_ms: default_normalization_speed(),
        }
    }
}

fn default_normalization_target() -> f32 {
    0.5
}

fn default_normalization_speed() -> f32 {
    1_000.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputProject {
    pub enabled: bool,
    pub fullscreen: bool,
    #[serde(default)]
    pub display_id: String,
    #[serde(default)]
    pub test_card: bool,
    #[serde(default)]
    pub identify: bool,
    pub composition_extent: [u32; 2],
}

impl Default for OutputProject {
    fn default() -> Self {
        Self {
            enabled: true,
            fullscreen: false,
            display_id: String::new(),
            test_card: false,
            identify: false,
            composition_extent: [1920, 1080],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantizationProject {
    Immediate,
    Beat,
    Bar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeckProject {
    pub clips: Vec<Option<PathBuf>>,
    #[serde(default = "default_clip_playback")]
    pub clip_playback: Vec<ClipPlaybackProject>,
    pub selected_slot: usize,
    pub active_slot: Option<usize>,
    pub level: f32,
    pub bus: CrossfadeBusProject,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub bypassed: bool,
    pub transport: TransportProject,
    #[serde(default)]
    pub transform: TransformProject,
    #[serde(default)]
    pub blend_mode: BlendModeProject,
    pub effects: EffectProject,
    #[serde(default = "default_lfos")]
    pub lfos: Vec<LfoProject>,
    #[serde(default = "default_mod_routes")]
    pub mod_routes: Vec<ModRouteProject>,
    #[serde(default)]
    pub camera: Option<CameraProject>,
}

impl Default for DeckProject {
    fn default() -> Self {
        Self {
            clips: vec![None; CLIPS_PER_DECK],
            clip_playback: default_clip_playback(),
            selected_slot: 0,
            active_slot: None,
            level: 1.0,
            bus: CrossfadeBusProject::Left,
            solo: false,
            bypassed: false,
            transport: TransportProject::default(),
            transform: TransformProject::default(),
            blend_mode: BlendModeProject::Normal,
            effects: EffectProject::default(),
            lfos: default_lfos(),
            mod_routes: default_mod_routes(),
            camera: None,
        }
    }
}

fn default_clip_playback() -> Vec<ClipPlaybackProject> {
    vec![ClipPlaybackProject::default(); CLIPS_PER_DECK]
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipPlaybackProject {
    pub in_point: f64,
    pub out_point: Option<f64>,
    #[serde(default)]
    pub launch_mode: ClipLaunchModeProject,
    pub beat_duration: Option<f64>,
}

impl Default for ClipPlaybackProject {
    fn default() -> Self {
        Self {
            in_point: 0.0,
            out_point: None,
            launch_mode: ClipLaunchModeProject::Restart,
            beat_duration: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipLaunchModeProject {
    #[default]
    Restart,
    Resume,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossfadeBusProject {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformProject {
    pub position: [f32; 2],
    pub scale: f32,
    pub rotation: f32,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    #[serde(default)]
    pub crop: [f32; 4],
    #[serde(default)]
    pub source_mode: SourceModeProject,
}

impl Default for TransformProject {
    fn default() -> Self {
        Self {
            position: [0.0; 2],
            scale: 1.0,
            rotation: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            crop: [0.0; 4],
            source_mode: SourceModeProject::Stretch,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceModeProject {
    Fit,
    Fill,
    #[default]
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendModeProject {
    #[default]
    Normal,
    Add,
    Screen,
    Multiply,
    Difference,
    Lighten,
    Darken,
    Overlay,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransportProject {
    pub playing: bool,
    pub frozen: bool,
    pub end_mode: EndModeProject,
    pub speed: f32,
    pub position: f64,
}

impl Default for TransportProject {
    fn default() -> Self {
        Self {
            playing: true,
            frozen: false,
            end_mode: EndModeProject::Loop,
            speed: 1.0,
            position: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndModeProject {
    Loop,
    OneShot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectProject {
    #[serde(default = "default_effect_slots")]
    pub slots: Vec<EffectSlotProject>,
    pub contrast: f32,
    pub saturation: f32,
    #[serde(default)]
    pub hue: f32,
    #[serde(default)]
    pub black_level: f32,
    #[serde(default = "one")]
    pub white_level: f32,
    #[serde(default = "one")]
    pub gamma: f32,
    pub pixelate: f32,
    pub luma_key: f32,
    #[serde(default)]
    pub neon: f32,
    #[serde(default)]
    pub fractal: f32,
    #[serde(default)]
    pub jitter: f32,
    #[serde(default)]
    pub find_edges: f32,
    #[serde(default)]
    pub bit_reduction: f32,
    #[serde(default)]
    pub blacklight: f32,
    pub mirror: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectGroupProject {
    Geometry,
    Color,
    Stylize,
}

impl EffectGroupProject {
    const ALL: [Self; 3] = [Self::Geometry, Self::Color, Self::Stylize];
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectSlotProject {
    pub group: EffectGroupProject,
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default = "one")]
    pub mix: f32,
}

fn default_effect_slots() -> Vec<EffectSlotProject> {
    EffectGroupProject::ALL
        .into_iter()
        .map(|group| EffectSlotProject {
            group,
            bypassed: false,
            mix: 1.0,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LfoWaveformProject {
    Sine,
    Triangle,
    Saw,
    SawDown,
    Square,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectTargetProject {
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LfoProject {
    pub enabled: bool,
    #[serde(default = "yes")]
    pub direct_enabled: bool,
    pub target: EffectTargetProject,
    pub waveform: LfoWaveformProject,
    pub rate_hz: f32,
    #[serde(default)]
    pub tempo_sync: bool,
    #[serde(default = "one")]
    pub beats_per_cycle: f32,
    pub depth: f32,
    pub phase: f32,
}

impl Default for LfoProject {
    fn default() -> Self {
        Self {
            enabled: false,
            direct_enabled: true,
            target: EffectTargetProject::Hue,
            waveform: LfoWaveformProject::Sine,
            rate_hz: 0.25,
            tempo_sync: false,
            beats_per_cycle: 1.0,
            depth: 0.5,
            phase: 0.0,
        }
    }
}

fn default_lfos() -> Vec<LfoProject> {
    vec![LfoProject::default(); 3]
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModRouteProject {
    pub enabled: bool,
    pub source: u8,
    pub target: EffectTargetProject,
    pub amount: f32,
}

impl Default for ModRouteProject {
    fn default() -> Self {
        Self {
            enabled: false,
            source: 0,
            target: EffectTargetProject::Hue,
            amount: 0.5,
        }
    }
}

fn default_mod_routes() -> Vec<ModRouteProject> {
    vec![ModRouteProject::default(); 8]
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CameraProject {
    pub backend: String,
    pub device_id: String,
    pub label: String,
    pub requested_extent: Option<[u32; 2]>,
    pub requested_fps: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MidiMappingProject {
    pub device: String,
    pub channel: u8,
    pub message: MidiMessageProject,
    pub number: u8,
    pub target: ControlTargetProject,
    pub input_range: [f32; 2],
    pub output_range: [f32; 2],
    pub invert: bool,
    pub mode: MappingModeProject,
    pub soft_takeover: bool,
    pub feedback: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MidiMessageProject {
    Note,
    ControlChange,
    PitchBend,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingModeProject {
    Continuous,
    Momentary,
    Toggle,
    RelativeBinaryOffset,
    RelativeTwosComplement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlTargetProject {
    Crossfader,
    MasterOpacity,
    MasterBlackout,
    MasterFreeze,
    TapTempo,
    DeckLevel { deck: u8 },
    DeckPlay { deck: u8 },
    DeckFreeze { deck: u8 },
    DeckSpeed { deck: u8 },
    DeckSelect { deck: u8 },
    DeckRestart { deck: u8 },
    ClipLaunch { deck: u8, slot: u8 },
    SceneLaunch { slot: u8 },
    EffectParameter { deck: u8, effect: u8, parameter: u8 },
    LfoParameter { deck: u8, lfo: u8, parameter: u8 },
    ModRouteParameter { deck: u8, route: u8, parameter: u8 },
    MasterEffectParameter { slot: u8, parameter_key: u64 },
}

impl Default for EffectProject {
    fn default() -> Self {
        Self {
            slots: default_effect_slots(),
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
            mirror: false,
        }
    }
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("open project {path}: {source}")]
    Open {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("read project JSON: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("project format is {0:?}, not an Oneiroi project")]
    WrongFormat(String),
    #[error("project version {0} is not supported")]
    UnsupportedVersion(u32),
    #[error("invalid project shape: {0}")]
    InvalidShape(String),
    #[error("invalid project value: {0}")]
    InvalidValue(String),
    #[error("create project directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("write project {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn load_project(path: impl AsRef<Path>) -> Result<ProjectFile, ProjectError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| ProjectError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let mut project: ProjectFile = serde_json::from_reader(BufReader::new(file))?;
    project.validate()?;
    if project.project_id.is_empty() {
        project.project_id = new_project_id();
    }
    project.version = PROJECT_VERSION;
    Ok(project)
}

pub fn save_project_atomic(
    path: impl AsRef<Path>,
    project: &ProjectFile,
) -> Result<(), ProjectError> {
    project.validate()?;
    let path = path.as_ref();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ProjectError::CreateDirectory {
        path: parent.to_path_buf(),
        source,
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project.oneiroi");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| ProjectError::Write {
                path: temporary.clone(),
                source,
            })?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, project)?;
        writer.flush().map_err(|source| ProjectError::Write {
            path: temporary.clone(),
            source,
        })?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|source| ProjectError::Write {
                path: temporary.clone(),
                source,
            })?;
        fs::rename(&temporary, path).map_err(|source| ProjectError::Write {
            path: path.to_path_buf(),
            source,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn autosave_path(project_path: Option<&Path>, workspace: &Path) -> PathBuf {
    project_path.map_or_else(
        || workspace.join(".oneiroi-untitled.autosave"),
        |path| {
            let file = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project.oneiroi");
            path.with_file_name(format!(".{file}.autosave"))
        },
    )
}

pub fn recovery_is_newer(project_path: &Path, recovery_path: &Path) -> bool {
    let recovery_modified = modified(recovery_path);
    let project_modified = modified(project_path);
    match (recovery_modified, project_modified) {
        (Some(recovery), Some(project)) => recovery > project,
        (Some(_), None) => true,
        _ => false,
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn test_path(name: &str) -> PathBuf {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "oneiroi-project-{}-{id}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn atomically_round_trips_versioned_project() {
        let path = test_path("roundtrip.oneiroi");
        let mut project = ProjectFile::default();
        project.takes.push(TakeMetadataProject {
            take_id: new_project_id(),
            name: "Opening take".to_owned(),
            journal_file: "session-opening.jsonl".to_owned(),
            created_unix_ms: 123,
        });
        project.decks[3].clips[7] = Some(PathBuf::from("/show/clip.mov"));
        project.decks[3].active_slot = Some(7);
        project.decks[2].camera = Some(CameraProject {
            backend: "avfoundation".to_owned(),
            device_id: "0".to_owned(),
            label: "Camera".to_owned(),
            requested_extent: Some([1280, 720]),
            requested_fps: Some(30),
        });
        project.decks[1].lfos[0] = LfoProject {
            enabled: true,
            direct_enabled: false,
            target: EffectTargetProject::Neon,
            waveform: LfoWaveformProject::Triangle,
            rate_hz: 0.5,
            tempo_sync: true,
            beats_per_cycle: 2.0,
            depth: 0.75,
            phase: 0.25,
        };
        project.decks[1].mod_routes[0] = ModRouteProject {
            enabled: true,
            source: 9,
            target: EffectTargetProject::Jitter,
            amount: -0.6,
        };
        project.decks[0].transform = TransformProject {
            position: [0.25, -0.5],
            scale: 1.5,
            rotation: 0.125,
            flip_horizontal: true,
            flip_vertical: false,
            crop: [0.1, 0.2, 0.0, 0.15],
            source_mode: SourceModeProject::Fill,
        };
        project.decks[0].blend_mode = BlendModeProject::Screen;
        project.decks[0].effects.slots.swap(0, 2);
        project.decks[0].effects.slots[1].bypassed = true;
        project.decks[0].effects.slots[2].mix = 0.35;
        project.decks[0].solo = true;
        project.decks[2].bypassed = true;
        project.settings.output = OutputProject {
            enabled: false,
            fullscreen: true,
            display_id: "stage-left".to_owned(),
            test_card: true,
            identify: true,
            composition_extent: [3840, 2160],
        };
        project.settings.audio_analysis = AudioAnalysisProject {
            gain: 2.5,
            noise_floor: 0.03,
            attack_ms: 8.0,
            release_ms: 240.0,
            transient_sensitivity: 3.5,
            normalization: true,
            normalization_target: 0.6,
            normalization_speed_ms: 750.0,
        };
        project.settings.master_effects.slots[0] = MasterEffectSlotProject {
            kind: MasterEffectKindProject::Blur,
            bypassed: false,
            mix: 0.65,
            amount: 14.0,
            feedback: 0.85,
            ..MasterEffectSlotProject::default()
        };
        project.settings.master_effects.slots[1] = MasterEffectSlotProject {
            kind: MasterEffectKindProject::Feedback,
            bypassed: false,
            mix: 0.8,
            amount: 8.0,
            feedback: 0.92,
            ..MasterEffectSlotProject::default()
        };
        project.settings.master_effects.slots.swap(0, 1);
        project.settings.bpm = 128.0;
        project.decks[0].clip_playback[3] = ClipPlaybackProject {
            in_point: 1.25,
            out_point: Some(9.5),
            launch_mode: ClipLaunchModeProject::Resume,
            beat_duration: Some(8.0),
        };
        project.midi_mappings.push(MidiMappingProject {
            device: "controller".to_owned(),
            channel: 0,
            message: MidiMessageProject::ControlChange,
            number: 7,
            target: ControlTargetProject::LfoParameter {
                deck: 2,
                lfo: 1,
                parameter: 2,
            },
            input_range: [0.0, 1.0],
            output_range: [0.0, 1.0],
            invert: false,
            mode: MappingModeProject::RelativeTwosComplement,
            soft_takeover: true,
            feedback: Some("ring".to_owned()),
        });
        save_project_atomic(&path, &project).unwrap();
        assert_eq!(load_project(&path).unwrap(), project);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_unknown_versions_and_malformed_grid() {
        let mut project = ProjectFile {
            version: PROJECT_VERSION + 1,
            ..ProjectFile::default()
        };
        assert!(matches!(
            project.validate(),
            Err(ProjectError::UnsupportedVersion(_))
        ));
        project.version = PROJECT_VERSION;
        project.decks[0].clips.pop();
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidShape(_))
        ));
    }

    #[test]
    fn rejects_invalid_take_identity_and_non_file_journal_paths() {
        let mut project = ProjectFile::default();
        project.takes.push(TakeMetadataProject {
            take_id: "not-an-identity".to_owned(),
            name: "Take".to_owned(),
            journal_file: "nested/take.jsonl".to_owned(),
            created_unix_ms: 0,
        });
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));
    }

    #[test]
    fn rejects_out_of_range_midi_targets() {
        let mut project = ProjectFile::default();
        project.midi_mappings.push(MidiMappingProject {
            device: "controller".to_owned(),
            channel: 0,
            message: MidiMessageProject::Note,
            number: 1,
            target: ControlTargetProject::ClipLaunch { deck: 4, slot: 0 },
            input_range: [0.0, 1.0],
            output_range: [0.0, 1.0],
            invert: false,
            mode: MappingModeProject::Momentary,
            soft_takeover: false,
            feedback: None,
        });
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));
    }

    #[test]
    fn rejects_duplicate_or_invalid_effect_slots() {
        let mut project = ProjectFile::default();
        project.decks[0].effects.slots[1].group = project.decks[0].effects.slots[0].group;
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));

        project = ProjectFile::default();
        project.decks[0].effects.slots[0].mix = 1.5;
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));
    }

    #[test]
    fn rejects_invalid_master_effect_slots() {
        let mut project = ProjectFile::default();
        project.settings.master_effects.slots.pop();
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));

        project = ProjectFile::default();
        project.settings.master_effects.slots[0].amount = 64.0;
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));

        project = ProjectFile::default();
        project.settings.master_effects.slots[0].feedback = 1.0;
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));
    }

    #[test]
    fn custom_master_effect_values_round_trip_and_validate() {
        let mut project = ProjectFile::default();
        project.settings.master_effects.slots[0] = MasterEffectSlotProject {
            kind: MasterEffectKindProject::Custom,
            package_id: "chromatic-split".to_owned(),
            parameters: vec![
                EffectParameterValueProject {
                    id: "amount".to_owned(),
                    value: 0.02,
                },
                EffectParameterValueProject {
                    id: "angle".to_owned(),
                    value: 1.0,
                },
            ],
            ..MasterEffectSlotProject::default()
        };
        project.settings.master_modulation.lfos[0].enabled = true;
        project.settings.master_modulation.lfos[0].tempo_sync = true;
        project.settings.master_modulation.lfos[0].beats_per_cycle = 2.0;
        project.settings.master_modulation.routes[0] = MasterModulationRouteProject {
            enabled: true,
            source: 0,
            target_slot: 0,
            parameter_key: 42,
            amount: -0.75,
        };
        project.midi_mappings.push(MidiMappingProject {
            device: "controller".to_owned(),
            channel: 0,
            message: MidiMessageProject::ControlChange,
            number: 14,
            target: ControlTargetProject::MasterEffectParameter {
                slot: 0,
                parameter_key: 42,
            },
            input_range: [0.0, 1.0],
            output_range: [0.0, 0.08],
            invert: false,
            mode: MappingModeProject::Continuous,
            soft_takeover: true,
            feedback: None,
        });
        project.validate().unwrap();
        let encoded = serde_json::to_vec(&project).unwrap();
        let decoded: ProjectFile = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, project);

        project.settings.master_effects.slots[0].parameters[1].id = "amount".to_owned();
        assert!(matches!(
            project.validate(),
            Err(ProjectError::InvalidValue(_))
        ));
    }

    #[test]
    fn derives_saved_and_untitled_autosave_paths() {
        let workspace = Path::new("/shows");
        assert_eq!(
            autosave_path(None, workspace),
            PathBuf::from("/shows/.oneiroi-untitled.autosave")
        );
        assert_eq!(
            autosave_path(Some(Path::new("/shows/set.oneiroi")), workspace),
            PathBuf::from("/shows/.set.oneiroi.autosave")
        );
    }

    #[test]
    fn early_version_one_projects_remain_compatible() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        value["version"] = serde_json::json!(1);
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("output")
            .unwrap();
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("audio_analysis")
            .unwrap();
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("master_effects")
            .unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("midi_mappings")
            .unwrap();
        for deck in value["decks"].as_array_mut().unwrap() {
            let deck = deck.as_object_mut().unwrap();
            deck.remove("transform").unwrap();
            deck.remove("blend_mode").unwrap();
            deck.remove("solo").unwrap();
            deck.remove("bypassed").unwrap();
            deck.remove("lfos").unwrap();
            deck.remove("mod_routes").unwrap();
            deck.remove("clip_playback").unwrap();
            let effects = deck["effects"].as_object_mut().unwrap();
            effects.remove("slots").unwrap();
            for field in [
                "hue",
                "black_level",
                "white_level",
                "gamma",
                "neon",
                "fractal",
                "jitter",
                "find_edges",
                "bit_reduction",
                "blacklight",
            ] {
                effects.remove(field).unwrap();
            }
        }
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(project.midi_mappings.is_empty());
        assert!(
            project
                .decks
                .iter()
                .all(|deck| deck.clip_playback == default_clip_playback())
        );
        assert!(project.decks.iter().all(|deck| deck.lfos == default_lfos()));
        assert!(
            project
                .decks
                .iter()
                .all(|deck| deck.mod_routes == default_mod_routes())
        );
        assert!(
            project
                .decks
                .iter()
                .all(|deck| deck.effects == EffectProject::default())
        );
        project.validate().unwrap();
    }

    #[test]
    fn early_version_two_output_settings_receive_safe_defaults() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        let output = value["settings"]["output"].as_object_mut().unwrap();
        output.remove("display_id").unwrap();
        output.remove("test_card").unwrap();
        output.remove("identify").unwrap();
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(project.settings.output.display_id.is_empty());
        assert!(!project.settings.output.test_card);
        assert!(!project.settings.output.identify);
        project.validate().unwrap();
    }

    #[test]
    fn projects_saved_before_layer_transforms_receive_neutral_geometry() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        for deck in value["decks"].as_array_mut().unwrap() {
            deck.as_object_mut().unwrap().remove("transform").unwrap();
        }
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(
            project
                .decks
                .iter()
                .all(|deck| deck.transform == TransformProject::default())
        );
        project.validate().unwrap();
    }

    #[test]
    fn early_transform_projects_default_to_uncropped_stretch() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        for deck in value["decks"].as_array_mut().unwrap() {
            let transform = deck["transform"].as_object_mut().unwrap();
            transform.remove("crop").unwrap();
            transform.remove("source_mode").unwrap();
        }
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(project.decks.iter().all(|deck| {
            deck.transform.crop == [0.0; 4]
                && deck.transform.source_mode == SourceModeProject::Stretch
        }));
        project.validate().unwrap();
    }

    #[test]
    fn projects_saved_before_blend_modes_default_to_normal() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        for deck in value["decks"].as_array_mut().unwrap() {
            deck.as_object_mut().unwrap().remove("blend_mode").unwrap();
        }
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(
            project
                .decks
                .iter()
                .all(|deck| deck.blend_mode == BlendModeProject::Normal)
        );
        project.validate().unwrap();
    }

    #[test]
    fn projects_saved_before_solo_and_bypass_default_to_active() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        for deck in value["decks"].as_array_mut().unwrap() {
            let deck = deck.as_object_mut().unwrap();
            deck.remove("solo").unwrap();
            deck.remove("bypassed").unwrap();
        }
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(
            project
                .decks
                .iter()
                .all(|deck| !deck.solo && !deck.bypassed)
        );
        project.validate().unwrap();
    }

    #[test]
    fn projects_saved_before_audio_analysis_receive_safe_defaults() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("audio_analysis")
            .unwrap();
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert_eq!(
            project.settings.audio_analysis,
            AudioAnalysisProject::default()
        );
        project.validate().unwrap();
    }

    #[test]
    fn early_audio_settings_default_to_manual_gain() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        let audio = value["settings"]["audio_analysis"].as_object_mut().unwrap();
        audio.remove("normalization").unwrap();
        audio.remove("normalization_target").unwrap();
        audio.remove("normalization_speed_ms").unwrap();
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(!project.settings.audio_analysis.normalization);
        assert_eq!(project.settings.audio_analysis.normalization_target, 0.5);
        assert_eq!(
            project.settings.audio_analysis.normalization_speed_ms,
            1_000.0
        );
        project.validate().unwrap();
    }

    #[test]
    fn lfos_saved_before_direct_toggle_keep_their_direct_route() {
        let mut value = serde_json::to_value(ProjectFile::default()).unwrap();
        for deck in value["decks"].as_array_mut().unwrap() {
            for lfo in deck["lfos"].as_array_mut().unwrap() {
                lfo.as_object_mut()
                    .unwrap()
                    .remove("direct_enabled")
                    .unwrap();
            }
        }
        let project: ProjectFile = serde_json::from_value(value).unwrap();
        assert!(
            project
                .decks
                .iter()
                .flat_map(|deck| &deck.lfos)
                .all(|lfo| lfo.direct_enabled)
        );
    }

    #[test]
    fn loading_version_one_upgrades_it_to_the_current_schema() {
        let path = test_path("migrate-v1.oneiroi");
        let mut project = ProjectFile {
            version: 1,
            ..ProjectFile::default()
        };
        project.project_id.clear();
        project.takes.clear();
        project.settings.output = OutputProject::default();
        save_project_atomic(&path, &project).unwrap();
        let loaded = load_project(&path).unwrap();
        assert_eq!(loaded.version, PROJECT_VERSION);
        assert!(valid_identity(&loaded.project_id));
        assert_eq!(loaded.settings.output, OutputProject::default());
        fs::remove_file(path).unwrap();
    }
}
