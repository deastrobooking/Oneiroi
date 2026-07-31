use oneiroi_core::{
    AudioAnalysisSettings, ControlTarget, MappingMode, MidiBinding, MidiMapper, MidiMessage,
    MidiMessageKind, Quantization,
};
use oneiroi_io::{
    AudioAnalysisProject, BlendModeProject, CameraProject, ClipLaunchModeProject,
    ClipPlaybackProject, ControlTargetProject, CrossfadeBusProject, DeckProject,
    EffectGroupProject, EffectParameterValueProject, EffectProject, EffectSlotProject,
    EffectTargetProject, EndModeProject, LfoProject, LfoWaveformProject, MappingModeProject,
    MasterEffectKindProject, MasterEffectSlotProject, MasterEffectsProject, MasterLfoProject,
    MasterModulationProject, MasterModulationRouteProject, MidiMappingProject, MidiMessageProject,
    ModRouteProject, OutputProject, ProjectFile, ProjectSettings, QuantizationProject,
    SourceModeProject, TransformProject, TransportProject,
};
use oneiroi_media::{
    CLIPS_PER_DECK, CameraConfig, CameraDevice, ClipAddress, ClipBank, ClipLaunchMode,
    ClipPlayback, CrossfadeBus, DeckId, DeckTransport, EndMode, FourDeckMixer,
};
use oneiroi_render::{
    DeckEffects, DeckLfos, DeckTransform, EffectGroup, EffectLfo, EffectParameterValue, EffectSlot,
    EffectTarget, LayerBlendMode, LfoWaveform, MasterEffectChain, MasterEffectKind,
    MasterEffectSlot, MasterLfo, MasterModulation, MasterModulationRoute, ModulationRoute,
    SourceMode,
};

use crate::ui::UiState;

pub struct ProjectSessionMetadata<'a> {
    pub project_id: &'a str,
    pub takes: Vec<oneiroi_io::TakeMetadataProject>,
}

pub fn snapshot(
    ui: &UiState,
    mixer: &FourDeckMixer,
    clips: &ClipBank,
    transports: &[DeckTransport; 4],
    midi: &MidiMapper,
    live_configs: &[Option<CameraConfig>; 4],
    session: ProjectSessionMetadata<'_>,
) -> ProjectFile {
    ProjectFile {
        project_id: session.project_id.to_owned(),
        takes: session.takes,
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
            audio_analysis: audio_analysis_to_project(ui.audio_analysis),
            master_effects: master_effects_to_project(&ui.master_effects),
            master_modulation: master_modulation_to_project(ui.master_modulation),
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
                    clip_playback: (0..CLIPS_PER_DECK)
                        .map(|slot| {
                            clip_playback_to_project(
                                clips
                                    .playback(ClipAddress { deck, slot })
                                    .unwrap_or_default(),
                            )
                        })
                        .collect(),
                    selected_slot: clips.selected(deck),
                    active_slot: clips.active(deck),
                    level: live.level,
                    bus: match live.bus {
                        CrossfadeBus::Left => CrossfadeBusProject::Left,
                        CrossfadeBus::Right => CrossfadeBusProject::Right,
                    },
                    solo: ui.solo[deck.index()],
                    bypassed: ui.bypassed[deck.index()],
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
                    transform: transform_to_project(ui.transforms[deck.index()]),
                    blend_mode: blend_mode_to_project(ui.blend_modes[deck.index()]),
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

pub fn clip_playback_from_project(playback: ClipPlaybackProject) -> ClipPlayback {
    ClipPlayback {
        in_point: playback.in_point,
        out_point: playback.out_point,
        launch_mode: match playback.launch_mode {
            ClipLaunchModeProject::Restart => ClipLaunchMode::Restart,
            ClipLaunchModeProject::Resume => ClipLaunchMode::Resume,
        },
        beat_duration: playback.beat_duration,
    }
}

fn clip_playback_to_project(playback: ClipPlayback) -> ClipPlaybackProject {
    ClipPlaybackProject {
        in_point: playback.in_point,
        out_point: playback.out_point,
        launch_mode: match playback.launch_mode {
            ClipLaunchMode::Restart => ClipLaunchModeProject::Restart,
            ClipLaunchMode::Resume => ClipLaunchModeProject::Resume,
        },
        beat_duration: playback.beat_duration,
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
        let mut current = semantic(current.clone());
        current.project_id.clear();
        current.takes.clear();
        let mut baseline = semantic(ProjectFile::default());
        baseline.project_id.clear();
        baseline.takes.clear();
        return current != baseline;
    };
    semantic(current.clone()) != semantic(saved.clone())
}

fn semantic(mut project: ProjectFile) -> ProjectFile {
    // Take catalog changes are operational history. They are folded into the
    // next explicit/autosave write but do not make an otherwise unchanged
    // project appear edited on every new run.
    project.takes.clear();
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
    ui.audio_analysis = audio_analysis_from_project(project.settings.audio_analysis);
    ui.master_effects = master_effects_from_project(&project.settings.master_effects);
    ui.master_modulation = master_modulation_from_project(&project.settings.master_modulation);
    ui.blackout = false;
    ui.master_freeze = false;
}

fn master_effects_to_project(effects: &MasterEffectChain) -> MasterEffectsProject {
    MasterEffectsProject {
        slots: effects
            .slots
            .iter()
            .map(|slot| MasterEffectSlotProject {
                kind: match slot.kind {
                    MasterEffectKind::None => MasterEffectKindProject::None,
                    MasterEffectKind::Blur => MasterEffectKindProject::Blur,
                    MasterEffectKind::Feedback => MasterEffectKindProject::Feedback,
                    MasterEffectKind::Custom => MasterEffectKindProject::Custom,
                },
                bypassed: slot.bypassed,
                mix: slot.mix,
                amount: slot.amount,
                feedback: slot.feedback,
                package_id: slot.package_id.clone(),
                parameters: slot
                    .parameters
                    .iter()
                    .map(|parameter| EffectParameterValueProject {
                        id: parameter.id.clone(),
                        value: parameter.value,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn master_effects_from_project(effects: &MasterEffectsProject) -> MasterEffectChain {
    let mut result = MasterEffectChain::default();
    for (destination, source) in result.slots.iter_mut().zip(&effects.slots) {
        *destination = MasterEffectSlot {
            kind: match source.kind {
                MasterEffectKindProject::None => MasterEffectKind::None,
                MasterEffectKindProject::Blur => MasterEffectKind::Blur,
                MasterEffectKindProject::Feedback => MasterEffectKind::Feedback,
                MasterEffectKindProject::Custom => MasterEffectKind::Custom,
            },
            bypassed: source.bypassed,
            mix: source.mix,
            amount: source.amount,
            feedback: source.feedback,
            package_id: source.package_id.clone(),
            parameters: source
                .parameters
                .iter()
                .map(|parameter| EffectParameterValue {
                    id: parameter.id.clone(),
                    value: parameter.value,
                })
                .collect(),
        };
    }
    result.sanitized()
}

fn master_modulation_to_project(modulation: MasterModulation) -> MasterModulationProject {
    MasterModulationProject {
        lfos: modulation
            .lfos
            .into_iter()
            .map(|lfo| MasterLfoProject {
                enabled: lfo.enabled,
                waveform: waveform_to_project(lfo.waveform),
                rate_hz: lfo.rate_hz,
                tempo_sync: lfo.tempo_sync,
                beats_per_cycle: lfo.beats_per_cycle,
                depth: lfo.depth,
                phase: lfo.phase,
            })
            .collect(),
        routes: modulation
            .routes
            .into_iter()
            .map(|route| MasterModulationRouteProject {
                enabled: route.enabled,
                source: route.source,
                target_slot: route.target_slot,
                parameter_key: route.parameter_key,
                amount: route.amount,
            })
            .collect(),
    }
}

fn master_modulation_from_project(project: &MasterModulationProject) -> MasterModulation {
    let mut modulation = MasterModulation::default();
    for (destination, source) in modulation.lfos.iter_mut().zip(&project.lfos) {
        *destination = MasterLfo {
            enabled: source.enabled,
            waveform: waveform_from_project(source.waveform),
            rate_hz: source.rate_hz,
            tempo_sync: source.tempo_sync,
            beats_per_cycle: source.beats_per_cycle,
            depth: source.depth,
            phase: source.phase,
        };
    }
    for (destination, source) in modulation.routes.iter_mut().zip(&project.routes) {
        *destination = MasterModulationRoute {
            enabled: source.enabled,
            source: source.source,
            target_slot: source.target_slot,
            parameter_key: source.parameter_key,
            amount: source.amount,
        };
    }
    modulation
}

fn waveform_to_project(waveform: LfoWaveform) -> LfoWaveformProject {
    match waveform {
        LfoWaveform::Sine => LfoWaveformProject::Sine,
        LfoWaveform::Triangle => LfoWaveformProject::Triangle,
        LfoWaveform::Saw => LfoWaveformProject::Saw,
        LfoWaveform::SawDown => LfoWaveformProject::SawDown,
        LfoWaveform::Square => LfoWaveformProject::Square,
    }
}

fn waveform_from_project(waveform: LfoWaveformProject) -> LfoWaveform {
    match waveform {
        LfoWaveformProject::Sine => LfoWaveform::Sine,
        LfoWaveformProject::Triangle => LfoWaveform::Triangle,
        LfoWaveformProject::Saw => LfoWaveform::Saw,
        LfoWaveformProject::SawDown => LfoWaveform::SawDown,
        LfoWaveformProject::Square => LfoWaveform::Square,
    }
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
    ui.solo[deck.index()] = project.solo;
    ui.bypassed[deck.index()] = project.bypassed;
    ui.effects[deck.index()] = effect_from_project(&project.effects);
    ui.transforms[deck.index()] = transform_from_project(project.transform);
    ui.blend_modes[deck.index()] = blend_mode_from_project(project.blend_mode);
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
        in_point: 0.0,
    }
}

fn audio_analysis_to_project(settings: AudioAnalysisSettings) -> AudioAnalysisProject {
    let settings = settings.sanitized();
    AudioAnalysisProject {
        gain: settings.gain,
        noise_floor: settings.noise_floor,
        attack_ms: settings.attack_ms,
        release_ms: settings.release_ms,
        transient_sensitivity: settings.transient_sensitivity,
        normalization: settings.normalization,
        normalization_target: settings.normalization_target,
        normalization_speed_ms: settings.normalization_speed_ms,
    }
}

fn audio_analysis_from_project(settings: AudioAnalysisProject) -> AudioAnalysisSettings {
    AudioAnalysisSettings {
        gain: settings.gain,
        noise_floor: settings.noise_floor,
        attack_ms: settings.attack_ms,
        release_ms: settings.release_ms,
        transient_sensitivity: settings.transient_sensitivity,
        normalization: settings.normalization,
        normalization_target: settings.normalization_target,
        normalization_speed_ms: settings.normalization_speed_ms,
    }
    .sanitized()
}

fn blend_mode_to_project(mode: LayerBlendMode) -> BlendModeProject {
    match mode {
        LayerBlendMode::Normal => BlendModeProject::Normal,
        LayerBlendMode::Add => BlendModeProject::Add,
        LayerBlendMode::Screen => BlendModeProject::Screen,
        LayerBlendMode::Multiply => BlendModeProject::Multiply,
        LayerBlendMode::Difference => BlendModeProject::Difference,
        LayerBlendMode::Lighten => BlendModeProject::Lighten,
        LayerBlendMode::Darken => BlendModeProject::Darken,
        LayerBlendMode::Overlay => BlendModeProject::Overlay,
    }
}

fn blend_mode_from_project(mode: BlendModeProject) -> LayerBlendMode {
    match mode {
        BlendModeProject::Normal => LayerBlendMode::Normal,
        BlendModeProject::Add => LayerBlendMode::Add,
        BlendModeProject::Screen => LayerBlendMode::Screen,
        BlendModeProject::Multiply => LayerBlendMode::Multiply,
        BlendModeProject::Difference => LayerBlendMode::Difference,
        BlendModeProject::Lighten => LayerBlendMode::Lighten,
        BlendModeProject::Darken => LayerBlendMode::Darken,
        BlendModeProject::Overlay => LayerBlendMode::Overlay,
    }
}

fn transform_to_project(transform: DeckTransform) -> TransformProject {
    TransformProject {
        position: transform.position,
        scale: transform.scale,
        rotation: transform.rotation,
        flip_horizontal: transform.flip_horizontal,
        flip_vertical: transform.flip_vertical,
        crop: transform.crop,
        source_mode: match transform.source_mode {
            SourceMode::Fit => SourceModeProject::Fit,
            SourceMode::Fill => SourceModeProject::Fill,
            SourceMode::Stretch => SourceModeProject::Stretch,
        },
    }
}

fn transform_from_project(transform: TransformProject) -> DeckTransform {
    DeckTransform {
        position: transform.position,
        scale: transform.scale,
        rotation: transform.rotation,
        flip_horizontal: transform.flip_horizontal,
        flip_vertical: transform.flip_vertical,
        crop: transform.crop,
        source_mode: match transform.source_mode {
            SourceModeProject::Fit => SourceMode::Fit,
            SourceModeProject::Fill => SourceMode::Fill,
            SourceModeProject::Stretch => SourceMode::Stretch,
        },
    }
    .sanitized()
}

fn effect_to_project(effect: DeckEffects) -> EffectProject {
    EffectProject {
        slots: effect
            .slots
            .into_iter()
            .map(|slot| EffectSlotProject {
                group: match slot.group {
                    EffectGroup::Geometry => EffectGroupProject::Geometry,
                    EffectGroup::Color => EffectGroupProject::Color,
                    EffectGroup::Stylize => EffectGroupProject::Stylize,
                },
                bypassed: slot.bypassed,
                mix: slot.mix,
            })
            .collect(),
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
    let mut result = DeckEffects {
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
        ..DeckEffects::default()
    };
    for (destination, source) in result.slots.iter_mut().zip(&effect.slots) {
        *destination = EffectSlot {
            group: match source.group {
                EffectGroupProject::Geometry => EffectGroup::Geometry,
                EffectGroupProject::Color => EffectGroup::Color,
                EffectGroupProject::Stylize => EffectGroup::Stylize,
            },
            bypassed: source.bypassed,
            mix: source.mix,
        };
    }
    result.sanitized()
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
            MappingMode::RelativeBinaryOffset => MappingModeProject::RelativeBinaryOffset,
            MappingMode::RelativeTwosComplement => MappingModeProject::RelativeTwosComplement,
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
        MappingModeProject::RelativeBinaryOffset => MappingMode::RelativeBinaryOffset,
        MappingModeProject::RelativeTwosComplement => MappingMode::RelativeTwosComplement,
    };
    binding.soft_takeover = mapping.soft_takeover;
    binding
}

fn target_to_project(target: ControlTarget) -> ControlTargetProject {
    match target {
        ControlTarget::Crossfader => ControlTargetProject::Crossfader,
        ControlTarget::MasterOpacity => ControlTargetProject::MasterOpacity,
        ControlTarget::MasterBlackout => ControlTargetProject::MasterBlackout,
        ControlTarget::MasterFreeze => ControlTargetProject::MasterFreeze,
        ControlTarget::TapTempo => ControlTargetProject::TapTempo,
        ControlTarget::DeckLevel(deck) => ControlTargetProject::DeckLevel { deck },
        ControlTarget::DeckPlay(deck) => ControlTargetProject::DeckPlay { deck },
        ControlTarget::DeckFreeze(deck) => ControlTargetProject::DeckFreeze { deck },
        ControlTarget::DeckSpeed(deck) => ControlTargetProject::DeckSpeed { deck },
        ControlTarget::DeckSelect(deck) => ControlTargetProject::DeckSelect { deck },
        ControlTarget::DeckRestart(deck) => ControlTargetProject::DeckRestart { deck },
        ControlTarget::ClipLaunch { deck, slot } => ControlTargetProject::ClipLaunch { deck, slot },
        ControlTarget::SceneLaunch(slot) => ControlTargetProject::SceneLaunch { slot },
        ControlTarget::EffectParameter {
            deck,
            effect,
            parameter,
        } => ControlTargetProject::EffectParameter {
            deck,
            effect,
            parameter,
        },
        ControlTarget::LfoParameter {
            deck,
            lfo,
            parameter,
        } => ControlTargetProject::LfoParameter {
            deck,
            lfo,
            parameter,
        },
        ControlTarget::ModRouteParameter {
            deck,
            route,
            parameter,
        } => ControlTargetProject::ModRouteParameter {
            deck,
            route,
            parameter,
        },
        ControlTarget::MasterEffectParameter {
            slot,
            parameter_key,
        } => ControlTargetProject::MasterEffectParameter {
            slot,
            parameter_key,
        },
    }
}

fn target_from_project(target: ControlTargetProject) -> ControlTarget {
    match target {
        ControlTargetProject::Crossfader => ControlTarget::Crossfader,
        ControlTargetProject::MasterOpacity => ControlTarget::MasterOpacity,
        ControlTargetProject::MasterBlackout => ControlTarget::MasterBlackout,
        ControlTargetProject::MasterFreeze => ControlTarget::MasterFreeze,
        ControlTargetProject::TapTempo => ControlTarget::TapTempo,
        ControlTargetProject::DeckLevel { deck } => ControlTarget::DeckLevel(deck),
        ControlTargetProject::DeckPlay { deck } => ControlTarget::DeckPlay(deck),
        ControlTargetProject::DeckFreeze { deck } => ControlTarget::DeckFreeze(deck),
        ControlTargetProject::DeckSpeed { deck } => ControlTarget::DeckSpeed(deck),
        ControlTargetProject::DeckSelect { deck } => ControlTarget::DeckSelect(deck),
        ControlTargetProject::DeckRestart { deck } => ControlTarget::DeckRestart(deck),
        ControlTargetProject::ClipLaunch { deck, slot } => ControlTarget::ClipLaunch { deck, slot },
        ControlTargetProject::SceneLaunch { slot } => ControlTarget::SceneLaunch(slot),
        ControlTargetProject::EffectParameter {
            deck,
            effect,
            parameter,
        } => ControlTarget::EffectParameter {
            deck,
            effect,
            parameter,
        },
        ControlTargetProject::LfoParameter {
            deck,
            lfo,
            parameter,
        } => ControlTarget::LfoParameter {
            deck,
            lfo,
            parameter,
        },
        ControlTargetProject::ModRouteParameter {
            deck,
            route,
            parameter,
        } => ControlTarget::ModRouteParameter {
            deck,
            route,
            parameter,
        },
        ControlTargetProject::MasterEffectParameter {
            slot,
            parameter_key,
        } => ControlTarget::MasterEffectParameter {
            slot,
            parameter_key,
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

    #[test]
    fn extended_midi_target_and_relative_mode_round_trip() {
        let mut binding = MidiBinding::learned(
            "controller",
            MidiMessage::ControlChange {
                channel: 3,
                controller: 21,
                value: 65,
            },
            ControlTarget::ModRouteParameter {
                deck: 1,
                route: 6,
                parameter: 1,
            },
        );
        binding.mode = MappingMode::RelativeBinaryOffset;
        binding.output_range = [-1.0, 1.0];
        binding.soft_takeover = true;
        assert_eq!(midi_from_project(&midi_to_project(&binding)), binding);
    }

    #[test]
    fn clip_playback_project_conversion_preserves_trim_and_launch_mode() {
        let playback = ClipPlayback {
            in_point: 2.5,
            out_point: Some(14.0),
            launch_mode: ClipLaunchMode::Resume,
            beat_duration: Some(16.0),
        };
        assert_eq!(
            clip_playback_from_project(clip_playback_to_project(playback)),
            playback
        );
    }

    #[test]
    fn custom_master_effect_conversion_preserves_named_parameters() {
        let chain = MasterEffectChain {
            slots: [
                MasterEffectSlot {
                    kind: MasterEffectKind::Custom,
                    package_id: "chromatic-split".to_owned(),
                    parameters: vec![EffectParameterValue {
                        id: "amount".to_owned(),
                        value: 0.025,
                    }],
                    ..MasterEffectSlot::default()
                },
                MasterEffectSlot::default(),
            ],
        };
        assert_eq!(
            master_effects_from_project(&master_effects_to_project(&chain)),
            chain
        );

        let mut modulation = MasterModulation::default();
        modulation.lfos[0] = MasterLfo {
            enabled: true,
            tempo_sync: true,
            beats_per_cycle: 2.0,
            ..MasterLfo::default()
        };
        modulation.routes[0] = MasterModulationRoute {
            enabled: true,
            source: 0,
            target_slot: 0,
            parameter_key: oneiroi_core::effect_parameter_key("chromatic-split", "amount"),
            amount: -0.5,
        };
        assert_eq!(
            master_modulation_from_project(&master_modulation_to_project(modulation)),
            modulation
        );
    }
}
