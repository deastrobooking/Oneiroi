use oneiroi_core::{
    ControlTarget, MappingMode, MidiBinding, MidiMapper, MidiMessage, MidiMessageKind, Quantization,
};
use oneiroi_io::{
    CameraProject, ControlTargetProject, CrossfadeBusProject, DeckProject, EffectProject,
    EffectTargetProject, EndModeProject, LfoProject, LfoWaveformProject, MappingModeProject,
    MidiMappingProject, MidiMessageProject, ModRouteProject, OutputProject, ProjectFile,
    ProjectSettings, QuantizationProject, TransportProject,
};
use oneiroi_media::{
    CLIPS_PER_DECK, CameraConfig, CameraDevice, ClipAddress, ClipBank, CrossfadeBus, DeckId,
    DeckTransport, EndMode, FourDeckMixer,
};
use oneiroi_render::{
    DeckEffects, DeckLfos, EffectLfo, EffectTarget, LfoWaveform, ModulationRoute,
};

use crate::ui::UiState;

pub fn snapshot(
    ui: &UiState,
    mixer: &FourDeckMixer,
    clips: &ClipBank,
    transports: &[DeckTransport; 4],
    midi: &MidiMapper,
    live_configs: &[Option<CameraConfig>; 4],
) -> ProjectFile {
    ProjectFile {
        settings: ProjectSettings {
            bpm: ui.bpm,
            quantization: quantization_to_project(ui.quantization),
            crossfader: ui.crossfader,
            equal_power: ui.equal_power,
            master_opacity: ui.master_opacity,
            output: OutputProject {
                enabled: ui.output_enabled,
                fullscreen: ui.output_fullscreen,
                display_id: ui.output_display_id.clone(),
                test_card: ui.output_test_card,
                identify: ui.output_identify,
                composition_extent: ui.composition_extent,
            },
        },
        decks: DeckId::ALL
            .into_iter()
            .map(|deck| {
                let live = mixer.deck(deck);
                let transport = transports[deck.index()];
                DeckProject {
                    clips: (0..CLIPS_PER_DECK)
                        .map(|slot| {
                            clips
                                .path(ClipAddress { deck, slot })
                                .map(ToOwned::to_owned)
                        })
                        .collect(),
                    selected_slot: clips.selected(deck),
                    active_slot: clips.active(deck),
                    level: live.level,
                    bus: match live.bus {
                        CrossfadeBus::Left => CrossfadeBusProject::Left,
                        CrossfadeBus::Right => CrossfadeBusProject::Right,
                    },
                    transport: TransportProject {
                        playing: transport.playing,
                        frozen: transport.frozen,
                        end_mode: match transport.end_mode {
                            EndMode::Loop => EndModeProject::Loop,
                            EndMode::OneShot => EndModeProject::OneShot,
                        },
                        speed: transport.speed,
                        position: transport.position,
                    },
                    effects: effect_to_project(ui.effects[deck.index()]),
                    lfos: ui.lfos[deck.index()]
                        .lanes
                        .into_iter()
                        .map(lfo_to_project)
                        .collect(),
                    mod_routes: ui.lfos[deck.index()]
                        .routes
                        .into_iter()
                        .map(route_to_project)
                        .collect(),
                    camera: live_configs[deck.index()]
                        .as_ref()
                        .map(|config| CameraProject {
                            backend: config.device.backend.clone(),
                            device_id: config.device.id.clone(),
                            label: config.device.label.clone(),
                            requested_extent: config.requested_extent,
                            requested_fps: config.requested_fps,
                        }),
                }
            })
            .collect(),
        midi_mappings: midi.bindings.iter().map(midi_to_project).collect(),
        ..ProjectFile::default()
    }
}

pub fn camera_from_project(camera: &CameraProject) -> CameraConfig {
    CameraConfig {
        device: CameraDevice {
            id: camera.device_id.clone(),
            label: camera.label.clone(),
            backend: camera.backend.clone(),
        },
        requested_extent: camera.requested_extent,
        requested_fps: camera.requested_fps,
    }
}

pub fn apply_midi(project: &ProjectFile) -> MidiMapper {
    let mut mapper = MidiMapper::default();
    mapper.bindings = project
        .midi_mappings
        .iter()
        .map(midi_from_project)
        .collect();
    mapper
}

pub fn is_dirty(current: &ProjectFile, saved: Option<&ProjectFile>) -> bool {
    let Some(saved) = saved else {
        return current != &ProjectFile::default();
    };
    semantic(current.clone()) != semantic(saved.clone())
}

fn semantic(mut project: ProjectFile) -> ProjectFile {
    for deck in &mut project.decks {
        // The moving playhead is recovery state, not an edit.
        deck.transport.position = 0.0;
    }
    project
}

pub fn apply_master(project: &ProjectFile, ui: &mut UiState) {
    ui.bpm = project.settings.bpm;
    ui.quantization = quantization_from_project(project.settings.quantization);
    ui.crossfader = project.settings.crossfader;
    ui.equal_power = project.settings.equal_power;
    ui.master_opacity = project.settings.master_opacity;
    ui.output_enabled = project.settings.output.enabled;
    ui.output_fullscreen = project.settings.output.fullscreen;
    ui.output_display_id = project.settings.output.display_id.clone();
    ui.output_test_card = project.settings.output.test_card;
    ui.output_identify = project.settings.output.identify;
    ui.composition_extent = project.settings.output.composition_extent;
    ui.custom_composition_extent = project.settings.output.composition_extent;
    ui.blackout = false;
    ui.master_freeze = false;
}

pub fn apply_deck(
    deck: DeckId,
    project: &DeckProject,
    mixer: &mut FourDeckMixer,
    ui: &mut UiState,
) -> DeckTransport {
    let live = mixer.deck_mut(deck);
    live.level = project.level;
    live.bus = match project.bus {
        CrossfadeBusProject::Left => CrossfadeBus::Left,
        CrossfadeBusProject::Right => CrossfadeBus::Right,
    };
    ui.effects[deck.index()] = effect_from_project(&project.effects);
    let mut lfos = DeckLfos::default();
    for (destination, source) in lfos.lanes.iter_mut().zip(&project.lfos) {
        *destination = lfo_from_project(source);
    }
    for (destination, source) in lfos.routes.iter_mut().zip(&project.mod_routes) {
        *destination = route_from_project(source);
    }
    ui.lfos[deck.index()] = lfos;
    DeckTransport {
        playing: project.transport.playing,
        frozen: project.transport.frozen,
        end_mode: match project.transport.end_mode {
            EndModeProject::Loop => EndMode::Loop,
            EndModeProject::OneShot => EndMode::OneShot,
        },
        speed: project.transport.speed,
        position: project.transport.position.max(0.0),
        duration: None,
    }
}

fn effect_to_project(effect: DeckEffects) -> EffectProject {
    EffectProject {
        contrast: effect.contrast,
        saturation: effect.saturation,
        hue: effect.hue,
        black_level: effect.black_level,
        white_level: effect.white_level,
        gamma: effect.gamma,
        pixelate: effect.pixelate,
        luma_key: effect.luma_key,
        neon: effect.neon,
        fractal: effect.fractal,
        jitter: effect.jitter,
        find_edges: effect.find_edges,
        bit_reduction: effect.bit_reduction,
        blacklight: effect.blacklight,
        mirror: effect.mirror,
    }
}

fn effect_from_project(effect: &EffectProject) -> DeckEffects {
    DeckEffects {
        contrast: effect.contrast,
        saturation: effect.saturation,
        hue: effect.hue,
        black_level: effect.black_level,
        white_level: effect.white_level,
        gamma: effect.gamma,
        pixelate: effect.pixelate,
        luma_key: effect.luma_key,
        neon: effect.neon,
        fractal: effect.fractal,
        jitter: effect.jitter,
        find_edges: effect.find_edges,
        bit_reduction: effect.bit_reduction,
        blacklight: effect.blacklight,
        mirror: effect.mirror,
    }
}

fn lfo_to_project(lfo: EffectLfo) -> LfoProject {
    LfoProject {
        enabled: lfo.enabled,
        direct_enabled: lfo.direct_enabled,
        target: effect_target_to_project(lfo.target),
        waveform: match lfo.waveform {
            LfoWaveform::Sine => LfoWaveformProject::Sine,
            LfoWaveform::Triangle => LfoWaveformProject::Triangle,
            LfoWaveform::Saw => LfoWaveformProject::Saw,
            LfoWaveform::SawDown => LfoWaveformProject::SawDown,
            LfoWaveform::Square => LfoWaveformProject::Square,
        },
        rate_hz: lfo.rate_hz,
        tempo_sync: lfo.tempo_sync,
        beats_per_cycle: lfo.beats_per_cycle,
        depth: lfo.depth,
        phase: lfo.phase,
    }
}

fn lfo_from_project(lfo: &LfoProject) -> EffectLfo {
    EffectLfo {
        enabled: lfo.enabled,
        direct_enabled: lfo.direct_enabled,
        target: effect_target_from_project(lfo.target),
        waveform: match lfo.waveform {
            LfoWaveformProject::Sine => LfoWaveform::Sine,
            LfoWaveformProject::Triangle => LfoWaveform::Triangle,
            LfoWaveformProject::Saw => LfoWaveform::Saw,
            LfoWaveformProject::SawDown => LfoWaveform::SawDown,
            LfoWaveformProject::Square => LfoWaveform::Square,
        },
        rate_hz: lfo.rate_hz,
        tempo_sync: lfo.tempo_sync,
        beats_per_cycle: lfo.beats_per_cycle,
        depth: lfo.depth,
        phase: lfo.phase,
    }
}

fn route_to_project(route: ModulationRoute) -> ModRouteProject {
    ModRouteProject {
        enabled: route.enabled,
        source: route.source,
        target: effect_target_to_project(route.target),
        amount: route.amount,
    }
}

fn route_from_project(route: &ModRouteProject) -> ModulationRoute {
    ModulationRoute {
        enabled: route.enabled,
        source: route.source,
        target: effect_target_from_project(route.target),
        amount: route.amount,
    }
}

fn effect_target_to_project(target: EffectTarget) -> EffectTargetProject {
    match target {
        EffectTarget::Hue => EffectTargetProject::Hue,
        EffectTarget::Contrast => EffectTargetProject::Contrast,
        EffectTarget::Saturation => EffectTargetProject::Saturation,
        EffectTarget::BlackLevel => EffectTargetProject::BlackLevel,
        EffectTarget::WhiteLevel => EffectTargetProject::WhiteLevel,
        EffectTarget::Gamma => EffectTargetProject::Gamma,
        EffectTarget::Pixelate => EffectTargetProject::Pixelate,
        EffectTarget::LumaKey => EffectTargetProject::LumaKey,
        EffectTarget::Neon => EffectTargetProject::Neon,
        EffectTarget::Fractal => EffectTargetProject::Fractal,
        EffectTarget::Jitter => EffectTargetProject::Jitter,
        EffectTarget::FindEdges => EffectTargetProject::FindEdges,
        EffectTarget::BitReduction => EffectTargetProject::BitReduction,
        EffectTarget::Blacklight => EffectTargetProject::Blacklight,
    }
}

fn effect_target_from_project(target: EffectTargetProject) -> EffectTarget {
    match target {
        EffectTargetProject::Hue => EffectTarget::Hue,
        EffectTargetProject::Contrast => EffectTarget::Contrast,
        EffectTargetProject::Saturation => EffectTarget::Saturation,
        EffectTargetProject::BlackLevel => EffectTarget::BlackLevel,
        EffectTargetProject::WhiteLevel => EffectTarget::WhiteLevel,
        EffectTargetProject::Gamma => EffectTarget::Gamma,
        EffectTargetProject::Pixelate => EffectTarget::Pixelate,
        EffectTargetProject::LumaKey => EffectTarget::LumaKey,
        EffectTargetProject::Neon => EffectTarget::Neon,
        EffectTargetProject::Fractal => EffectTarget::Fractal,
        EffectTargetProject::Jitter => EffectTarget::Jitter,
        EffectTargetProject::FindEdges => EffectTarget::FindEdges,
        EffectTargetProject::BitReduction => EffectTarget::BitReduction,
        EffectTargetProject::Blacklight => EffectTarget::Blacklight,
    }
}

fn quantization_to_project(value: Quantization) -> QuantizationProject {
    match value {
        Quantization::Immediate => QuantizationProject::Immediate,
        Quantization::Beat => QuantizationProject::Beat,
        Quantization::Bar => QuantizationProject::Bar,
    }
}

fn quantization_from_project(value: QuantizationProject) -> Quantization {
    match value {
        QuantizationProject::Immediate => Quantization::Immediate,
        QuantizationProject::Beat => Quantization::Beat,
        QuantizationProject::Bar => Quantization::Bar,
    }
}

fn midi_to_project(binding: &MidiBinding) -> MidiMappingProject {
    MidiMappingProject {
        device: binding.device.clone(),
        channel: binding.channel,
        message: match binding.kind {
            MidiMessageKind::Note => MidiMessageProject::Note,
            MidiMessageKind::ControlChange => MidiMessageProject::ControlChange,
            MidiMessageKind::PitchBend => MidiMessageProject::PitchBend,
        },
        number: binding.number,
        target: target_to_project(binding.target),
        input_range: binding.input_range,
        output_range: binding.output_range,
        invert: binding.invert,
        mode: match binding.mode {
            MappingMode::Continuous => MappingModeProject::Continuous,
            MappingMode::Momentary => MappingModeProject::Momentary,
            MappingMode::Toggle => MappingModeProject::Toggle,
        },
        soft_takeover: binding.soft_takeover,
        feedback: None,
    }
}

fn midi_from_project(mapping: &MidiMappingProject) -> MidiBinding {
    let message = match mapping.message {
        MidiMessageProject::Note => MidiMessage::NoteOn {
            channel: mapping.channel,
            note: mapping.number,
            velocity: 0,
        },
        MidiMessageProject::ControlChange => MidiMessage::ControlChange {
            channel: mapping.channel,
            controller: mapping.number,
            value: 0,
        },
        MidiMessageProject::PitchBend => MidiMessage::PitchBend {
            channel: mapping.channel,
            value: 0,
        },
    };
    let mut binding = MidiBinding::learned(
        mapping.device.clone(),
        message,
        target_from_project(mapping.target),
    );
    binding.input_range = mapping.input_range;
    binding.output_range = mapping.output_range;
    binding.invert = mapping.invert;
    binding.mode = match mapping.mode {
        MappingModeProject::Continuous => MappingMode::Continuous,
        MappingModeProject::Momentary => MappingMode::Momentary,
        MappingModeProject::Toggle => MappingMode::Toggle,
    };
    binding.soft_takeover = mapping.soft_takeover;
    binding
}

fn target_to_project(target: ControlTarget) -> ControlTargetProject {
    match target {
        ControlTarget::Crossfader => ControlTargetProject::Crossfader,
        ControlTarget::MasterOpacity => ControlTargetProject::MasterOpacity,
        ControlTarget::MasterBlackout => ControlTargetProject::MasterBlackout,
        ControlTarget::DeckLevel(deck) => ControlTargetProject::DeckLevel { deck },
        ControlTarget::DeckPlay(deck) => ControlTargetProject::DeckPlay { deck },
        ControlTarget::DeckFreeze(deck) => ControlTargetProject::DeckFreeze { deck },
        ControlTarget::DeckSpeed(deck) => ControlTargetProject::DeckSpeed { deck },
        ControlTarget::EffectParameter {
            deck,
            effect,
            parameter,
        } => ControlTargetProject::EffectParameter {
            deck,
            effect,
            parameter,
        },
    }
}

fn target_from_project(target: ControlTargetProject) -> ControlTarget {
    match target {
        ControlTargetProject::Crossfader => ControlTarget::Crossfader,
        ControlTargetProject::MasterOpacity => ControlTarget::MasterOpacity,
        ControlTargetProject::MasterBlackout => ControlTarget::MasterBlackout,
        ControlTargetProject::DeckLevel { deck } => ControlTarget::DeckLevel(deck),
        ControlTargetProject::DeckPlay { deck } => ControlTarget::DeckPlay(deck),
        ControlTargetProject::DeckFreeze { deck } => ControlTarget::DeckFreeze(deck),
        ControlTargetProject::DeckSpeed { deck } => ControlTarget::DeckSpeed(deck),
        ControlTargetProject::EffectParameter {
            deck,
            effect,
            parameter,
        } => ControlTarget::EffectParameter {
            deck,
            effect,
            parameter,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_playhead_does_not_mark_saved_project_dirty() {
        let mut saved = ProjectFile::default();
        let mut current = saved.clone();
        saved.decks[0].transport.position = 2.0;
        current.decks[0].transport.position = 10.0;
        assert!(!is_dirty(&current, Some(&saved)));
        current.decks[0].level = 0.5;
        assert!(is_dirty(&current, Some(&saved)));
    }
}
