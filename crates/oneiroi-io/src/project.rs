use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROJECT_FORMAT: &str = "oneiroi-project";
pub const PROJECT_VERSION: u32 = 2;
const MINIMUM_PROJECT_VERSION: u32 = 1;
pub const DECK_COUNT: usize = 4;
pub const CLIPS_PER_DECK: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    pub format: String,
    pub version: u32,
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
            if deck.selected_slot >= CLIPS_PER_DECK
                || deck.active_slot.is_some_and(|slot| slot >= CLIPS_PER_DECK)
                || !unit(deck.level)
                || !deck.transport.speed.is_finite()
                || !(0.25..=4.0).contains(&deck.transport.speed)
                || !deck.transport.position.is_finite()
                || deck.transport.position < 0.0
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
                    .any(|route| route.source >= 3 || !effect_value(route.amount, -1.0, 1.0))
                || deck.camera.as_ref().is_some_and(|camera| {
                    camera.device_id.is_empty()
                        || camera.requested_fps == Some(0)
                        || camera
                            .requested_extent
                            .is_some_and(|[width, height]| width == 0 || height == 0)
                })
                || (deck.camera.is_some() && deck.active_slot.is_some())
            {
                return Err(ProjectError::InvalidValue(format!(
                    "deck {index} contains an unsupported value"
                )));
            }
        }
        for mapping in &self.midi_mappings {
            if mapping.channel > 15
                || mapping.number > 127
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

fn unit(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn effect_value(value: f32, minimum: f32, maximum: f32) -> bool {
    value.is_finite() && (minimum..=maximum).contains(&value)
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
        }
    }
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
    pub selected_slot: usize,
    pub active_slot: Option<usize>,
    pub level: f32,
    pub bus: CrossfadeBusProject,
    pub transport: TransportProject,
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
            selected_slot: 0,
            active_slot: None,
            level: 1.0,
            bus: CrossfadeBusProject::Left,
            transport: TransportProject::default(),
            effects: EffectProject::default(),
            lfos: default_lfos(),
            mod_routes: default_mod_routes(),
            camera: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossfadeBusProject {
    Left,
    Right,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlTargetProject {
    Crossfader,
    MasterOpacity,
    MasterBlackout,
    DeckLevel { deck: u8 },
    DeckPlay { deck: u8 },
    DeckFreeze { deck: u8 },
    DeckSpeed { deck: u8 },
    EffectParameter { deck: u8, effect: u8, parameter: u8 },
}

impl Default for EffectProject {
    fn default() -> Self {
        Self {
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
            source: 0,
            target: EffectTargetProject::Jitter,
            amount: -0.6,
        };
        project.settings.output = OutputProject {
            enabled: false,
            fullscreen: true,
            display_id: "stage-left".to_owned(),
            test_card: true,
            identify: true,
            composition_extent: [3840, 2160],
        };
        project.settings.bpm = 128.0;
        project.midi_mappings.push(MidiMappingProject {
            device: "controller".to_owned(),
            channel: 0,
            message: MidiMessageProject::ControlChange,
            number: 7,
            target: ControlTargetProject::Crossfader,
            input_range: [0.0, 1.0],
            output_range: [0.0, 1.0],
            invert: false,
            mode: MappingModeProject::Continuous,
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
        value
            .as_object_mut()
            .unwrap()
            .remove("midi_mappings")
            .unwrap();
        for deck in value["decks"].as_array_mut().unwrap() {
            let deck = deck.as_object_mut().unwrap();
            deck.remove("lfos").unwrap();
            deck.remove("mod_routes").unwrap();
            let effects = deck["effects"].as_object_mut().unwrap();
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
        project.settings.output = OutputProject::default();
        save_project_atomic(&path, &project).unwrap();
        let loaded = load_project(&path).unwrap();
        assert_eq!(loaded.version, PROJECT_VERSION);
        assert_eq!(loaded.settings.output, OutputProject::default());
        fs::remove_file(path).unwrap();
    }
}
