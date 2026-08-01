//! Deterministic command capture for editor-owned structures that are not
//! represented by the continuous control gateway.

use std::collections::BTreeMap;

use oneiroi_graph::ParameterValue;
use oneiroi_media::{CrossfadeBus, DeckId, FourDeckMixer};
use oneiroi_render::{
    DeckLfos, DeckTransform, EffectGroup, EffectTarget, LayerBlendMode, LfoWaveform,
    MasterEffectChain, MasterEffectKind, MasterModulation, SourceMode,
};
use oneiroi_session::CommandOperation;

use crate::ui::UiState;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StructuralSnapshot {
    buses: [CrossfadeBus; 4],
    equal_power: bool,
    transforms: [DeckTransform; 4],
    blend_modes: [LayerBlendMode; 4],
    solo: [bool; 4],
    bypassed: [bool; 4],
    effect_slots: [[oneiroi_render::EffectSlot; 3]; 4],
    mirror: [bool; 4],
    lfos: [DeckLfos; 4],
    master_effects: MasterEffectChain,
    master_modulation: MasterModulation,
}

impl StructuralSnapshot {
    pub(crate) fn capture(ui: &UiState, mixer: &FourDeckMixer) -> Self {
        Self {
            buses: std::array::from_fn(|index| mixer.deck(DeckId::ALL[index]).bus),
            equal_power: ui.equal_power,
            transforms: ui.transforms,
            blend_modes: ui.blend_modes,
            solo: ui.solo,
            bypassed: ui.bypassed,
            effect_slots: ui.effects.map(|effects| effects.slots),
            mirror: ui.effects.map(|effects| effects.mirror),
            lfos: ui.lfos,
            master_effects: ui.master_effects.clone(),
            master_modulation: ui.master_modulation,
        }
    }

    /// Restore the captured values so commands can be journaled before the
    /// accepted editor state is applied.
    pub(crate) fn apply(&self, ui: &mut UiState, mixer: &mut FourDeckMixer) {
        for (index, deck) in DeckId::ALL.into_iter().enumerate() {
            mixer.deck_mut(deck).bus = self.buses[index];
            ui.effects[index].slots = self.effect_slots[index];
            ui.effects[index].mirror = self.mirror[index];
        }
        ui.equal_power = self.equal_power;
        ui.transforms = self.transforms;
        ui.blend_modes = self.blend_modes;
        ui.solo = self.solo;
        ui.bypassed = self.bypassed;
        ui.lfos = self.lfos;
        ui.master_effects = self.master_effects.clone();
        ui.master_modulation = self.master_modulation;
    }

    pub(crate) fn commands_to(&self, next: &Self) -> Vec<CommandOperation> {
        let mut commands = Vec::new();
        changed_bool(
            &mut commands,
            "mixer.equal_power",
            self.equal_power,
            next.equal_power,
        );
        for deck in 0..4 {
            let root = format!("deck.{deck}");
            changed_text(
                &mut commands,
                format!("{root}.bus"),
                bus_name(self.buses[deck]),
                bus_name(next.buses[deck]),
            );
            diff_transform(
                &mut commands,
                &root,
                self.transforms[deck],
                next.transforms[deck],
            );
            changed_text(
                &mut commands,
                format!("{root}.blend_mode"),
                blend_name(self.blend_modes[deck]),
                blend_name(next.blend_modes[deck]),
            );
            changed_bool(
                &mut commands,
                format!("{root}.solo"),
                self.solo[deck],
                next.solo[deck],
            );
            changed_bool(
                &mut commands,
                format!("{root}.bypassed"),
                self.bypassed[deck],
                next.bypassed[deck],
            );
            changed_bool(
                &mut commands,
                format!("{root}.effects.mirror"),
                self.mirror[deck],
                next.mirror[deck],
            );
            diff_deck_effect_structure(
                &mut commands,
                &root,
                self.effect_slots[deck],
                next.effect_slots[deck],
                self.lfos[deck],
                next.lfos[deck],
            );
        }
        diff_master_effects(&mut commands, &self.master_effects, &next.master_effects);
        diff_master_modulation(
            &mut commands,
            self.master_modulation,
            next.master_modulation,
        );
        commands
    }
}

/// Apply the stable structural parameter paths produced by `commands_to`.
/// Unknown paths and values are ignored so newer journals remain recoverable
/// by older builds without corrupting known state.
pub(crate) fn apply_session_parameters(
    ui: &mut UiState,
    mixer: &mut FourDeckMixer,
    parameters: &BTreeMap<String, ParameterValue>,
) {
    if let Some(value) = bool_at(parameters, "mixer.equal_power") {
        ui.equal_power = value;
    }
    for deck in 0..4 {
        let root = format!("deck.{deck}");
        if let Some(value) = text_at(parameters, &format!("{root}.bus")) {
            mixer.deck_mut(DeckId::ALL[deck]).bus = match value {
                "left" => CrossfadeBus::Left,
                "right" => CrossfadeBus::Right,
                _ => mixer.deck(DeckId::ALL[deck]).bus,
            };
        }
        let transform = &mut ui.transforms[deck];
        for axis in 0..2 {
            assign_f32(
                &mut transform.position[axis],
                parameters,
                &format!("{root}.transform.position.{axis}"),
            );
        }
        assign_f32(
            &mut transform.scale,
            parameters,
            &format!("{root}.transform.scale"),
        );
        assign_f32(
            &mut transform.rotation,
            parameters,
            &format!("{root}.transform.rotation"),
        );
        assign_bool(
            &mut transform.flip_horizontal,
            parameters,
            &format!("{root}.transform.flip_horizontal"),
        );
        assign_bool(
            &mut transform.flip_vertical,
            parameters,
            &format!("{root}.transform.flip_vertical"),
        );
        for edge in 0..4 {
            assign_f32(
                &mut transform.crop[edge],
                parameters,
                &format!("{root}.transform.crop.{edge}"),
            );
        }
        if let Some(value) = text_at(parameters, &format!("{root}.transform.source_mode")) {
            transform.source_mode = match value {
                "fit" => SourceMode::Fit,
                "fill" => SourceMode::Fill,
                "stretch" => SourceMode::Stretch,
                _ => transform.source_mode,
            };
        }
        *transform = transform.sanitized();
        if let Some(value) = text_at(parameters, &format!("{root}.blend_mode")) {
            ui.blend_modes[deck] = LayerBlendMode::from_name(value).unwrap_or(ui.blend_modes[deck]);
        }
        assign_bool(&mut ui.solo[deck], parameters, &format!("{root}.solo"));
        assign_bool(
            &mut ui.bypassed[deck],
            parameters,
            &format!("{root}.bypassed"),
        );
        assign_bool(
            &mut ui.effects[deck].mirror,
            parameters,
            &format!("{root}.effects.mirror"),
        );
        apply_deck_effect_structure(ui, parameters, deck, &root);
    }
    apply_master_effect_structure(ui, parameters);
    apply_master_modulation_structure(ui, parameters);
}

pub(crate) fn session_parameters(
    ui: &UiState,
    mixer: &FourDeckMixer,
) -> BTreeMap<String, ParameterValue> {
    let defaults_ui = UiState::default();
    let defaults_mixer = FourDeckMixer::default();
    let defaults = StructuralSnapshot::capture(&defaults_ui, &defaults_mixer);
    let current = StructuralSnapshot::capture(ui, mixer);
    defaults
        .commands_to(&current)
        .into_iter()
        .filter_map(|command| match command {
            CommandOperation::SetParameter { path, value } => Some((path, value)),
            _ => None,
        })
        .collect()
}

fn apply_deck_effect_structure(
    ui: &mut UiState,
    parameters: &BTreeMap<String, ParameterValue>,
    deck: usize,
    root: &str,
) {
    for slot in 0..3 {
        let path = format!("{root}.effects.slot.{slot}");
        let effect_slot = &mut ui.effects[deck].slots[slot];
        if let Some(value) = text_at(parameters, &format!("{path}.group")) {
            effect_slot.group = match value {
                "color" => EffectGroup::Color,
                "geometry" => EffectGroup::Geometry,
                "stylize" => EffectGroup::Stylize,
                _ => effect_slot.group,
            };
        }
        assign_bool(
            &mut effect_slot.bypassed,
            parameters,
            &format!("{path}.bypassed"),
        );
        assign_f32(&mut effect_slot.mix, parameters, &format!("{path}.mix"));
        *effect_slot = effect_slot.sanitized();
        let lfo = &mut ui.lfos[deck].lanes[slot];
        let path = format!("{root}.lfo.{slot}");
        assign_bool(
            &mut lfo.direct_enabled,
            parameters,
            &format!("{path}.direct_enabled"),
        );
        if let Some(value) = text_at(parameters, &format!("{path}.target"))
            && let Some(value) = parse_target(value)
        {
            lfo.target = value;
        }
        if let Some(value) = text_at(parameters, &format!("{path}.waveform"))
            && let Some(value) = parse_waveform(value)
        {
            lfo.waveform = value;
        }
        assign_bool(
            &mut lfo.tempo_sync,
            parameters,
            &format!("{path}.tempo_sync"),
        );
        assign_f32(
            &mut lfo.beats_per_cycle,
            parameters,
            &format!("{path}.beats_per_cycle"),
        );
    }
    for route in 0..8 {
        let path = format!("{root}.modulation.route.{route}");
        if let Some(value) = integer_at(parameters, &format!("{path}.source"))
            && let Ok(value) = u8::try_from(value)
        {
            ui.lfos[deck].routes[route].source = value;
        }
        if let Some(value) = text_at(parameters, &format!("{path}.target"))
            && let Some(value) = parse_target(value)
        {
            ui.lfos[deck].routes[route].target = value;
        }
    }
}

fn apply_master_effect_structure(ui: &mut UiState, parameters: &BTreeMap<String, ParameterValue>) {
    for slot in 0..ui.master_effects.slots.len() {
        let root = format!("master.effect.{slot}");
        let effect = &mut ui.master_effects.slots[slot];
        if let Some(value) = text_at(parameters, &format!("{root}.kind")) {
            effect.kind = match value {
                "none" => MasterEffectKind::None,
                "blur" => MasterEffectKind::Blur,
                "feedback" => MasterEffectKind::Feedback,
                "custom" => MasterEffectKind::Custom,
                _ => effect.kind,
            };
        }
        assign_bool(
            &mut effect.bypassed,
            parameters,
            &format!("{root}.bypassed"),
        );
        assign_f32(&mut effect.mix, parameters, &format!("{root}.mix"));
        assign_f32(&mut effect.amount, parameters, &format!("{root}.amount"));
        assign_f32(
            &mut effect.feedback,
            parameters,
            &format!("{root}.feedback"),
        );
        if let Some(value) = text_at(parameters, &format!("{root}.package_id")) {
            effect.package_id = value.to_owned();
        }
        if let Some(ids) = text_at(parameters, &format!("{root}.parameter_ids")) {
            effect.parameters = ids
                .split('\u{1f}')
                .filter(|id| !id.is_empty())
                .map(|id| oneiroi_render::EffectParameterValue {
                    id: id.to_owned(),
                    value: scalar_at(parameters, &format!("{root}.parameter.{id}"))
                        .unwrap_or_default() as f32,
                })
                .collect();
        }
        effect.sanitize();
    }
}

fn apply_master_modulation_structure(
    ui: &mut UiState,
    parameters: &BTreeMap<String, ParameterValue>,
) {
    for index in 0..ui.master_modulation.lfos.len() {
        let lfo = &mut ui.master_modulation.lfos[index];
        let root = format!("master.lfo.{index}");
        assign_bool(&mut lfo.enabled, parameters, &format!("{root}.enabled"));
        if let Some(value) = text_at(parameters, &format!("{root}.waveform"))
            && let Some(value) = parse_waveform(value)
        {
            lfo.waveform = value;
        }
        assign_f32(&mut lfo.rate_hz, parameters, &format!("{root}.rate_hz"));
        assign_bool(
            &mut lfo.tempo_sync,
            parameters,
            &format!("{root}.tempo_sync"),
        );
        assign_f32(
            &mut lfo.beats_per_cycle,
            parameters,
            &format!("{root}.beats_per_cycle"),
        );
        assign_f32(&mut lfo.depth, parameters, &format!("{root}.depth"));
        assign_f32(&mut lfo.phase, parameters, &format!("{root}.phase"));
    }
    for index in 0..ui.master_modulation.routes.len() {
        let route = &mut ui.master_modulation.routes[index];
        let root = format!("master.modulation.route.{index}");
        assign_bool(&mut route.enabled, parameters, &format!("{root}.enabled"));
        if let Some(value) = integer_at(parameters, &format!("{root}.source"))
            && let Ok(value) = u8::try_from(value)
        {
            route.source = value;
        }
        if let Some(value) = integer_at(parameters, &format!("{root}.target_slot"))
            && let Ok(value) = u8::try_from(value)
        {
            route.target_slot = value;
        }
        if let Some(value) = text_at(parameters, &format!("{root}.parameter_key"))
            && let Ok(value) = value.parse()
        {
            route.parameter_key = value;
        }
        assign_f32(&mut route.amount, parameters, &format!("{root}.amount"));
    }
}

fn scalar_at(parameters: &BTreeMap<String, ParameterValue>, path: &str) -> Option<f64> {
    match parameters.get(path) {
        Some(ParameterValue::Scalar(value)) if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn bool_at(parameters: &BTreeMap<String, ParameterValue>, path: &str) -> Option<bool> {
    match parameters.get(path) {
        Some(ParameterValue::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn integer_at(parameters: &BTreeMap<String, ParameterValue>, path: &str) -> Option<i64> {
    match parameters.get(path) {
        Some(ParameterValue::Integer(value)) => Some(*value),
        _ => None,
    }
}

fn text_at<'a>(parameters: &'a BTreeMap<String, ParameterValue>, path: &str) -> Option<&'a str> {
    match parameters.get(path) {
        Some(ParameterValue::Text(value)) => Some(value),
        _ => None,
    }
}

fn assign_f32(value: &mut f32, parameters: &BTreeMap<String, ParameterValue>, path: &str) {
    if let Some(recovered) = scalar_at(parameters, path)
        && recovered >= f64::from(f32::MIN)
        && recovered <= f64::from(f32::MAX)
    {
        *value = recovered as f32;
    }
}

fn assign_bool(value: &mut bool, parameters: &BTreeMap<String, ParameterValue>, path: &str) {
    if let Some(recovered) = bool_at(parameters, path) {
        *value = recovered;
    }
}

fn parse_waveform(value: &str) -> Option<LfoWaveform> {
    Some(match value {
        "sine" => LfoWaveform::Sine,
        "triangle" => LfoWaveform::Triangle,
        "saw" => LfoWaveform::Saw,
        "saw_down" => LfoWaveform::SawDown,
        "square" => LfoWaveform::Square,
        _ => return None,
    })
}

fn parse_target(value: &str) -> Option<EffectTarget> {
    Some(match value {
        "hue" => EffectTarget::Hue,
        "contrast" => EffectTarget::Contrast,
        "saturation" => EffectTarget::Saturation,
        "black_level" => EffectTarget::BlackLevel,
        "white_level" => EffectTarget::WhiteLevel,
        "gamma" => EffectTarget::Gamma,
        "pixelate" => EffectTarget::Pixelate,
        "luma_key" => EffectTarget::LumaKey,
        "neon" => EffectTarget::Neon,
        "fractal" => EffectTarget::Fractal,
        "jitter" => EffectTarget::Jitter,
        "find_edges" => EffectTarget::FindEdges,
        "bit_reduction" => EffectTarget::BitReduction,
        "blacklight" => EffectTarget::Blacklight,
        "bloom" => EffectTarget::Bloom,
        "bloom_threshold" => EffectTarget::BloomThreshold,
        "bloom_radius" => EffectTarget::BloomRadius,
        "bloom_chroma" => EffectTarget::BloomChroma,
        _ => return None,
    })
}

fn command(path: impl Into<String>, value: ParameterValue) -> CommandOperation {
    CommandOperation::SetParameter {
        path: path.into(),
        value,
    }
}

fn changed_bool(
    commands: &mut Vec<CommandOperation>,
    path: impl Into<String>,
    old: bool,
    new: bool,
) {
    if old != new {
        commands.push(command(path, ParameterValue::Bool(new)));
    }
}

fn changed_i64(commands: &mut Vec<CommandOperation>, path: impl Into<String>, old: i64, new: i64) {
    if old != new {
        commands.push(command(path, ParameterValue::Integer(new)));
    }
}

fn changed_f32(commands: &mut Vec<CommandOperation>, path: impl Into<String>, old: f32, new: f32) {
    if old.to_bits() != new.to_bits() {
        commands.push(command(path, ParameterValue::Scalar(f64::from(new))));
    }
}

fn changed_text(
    commands: &mut Vec<CommandOperation>,
    path: impl Into<String>,
    old: &str,
    new: &str,
) {
    if old != new {
        commands.push(command(path, ParameterValue::Text(new.to_owned())));
    }
}

fn diff_transform(
    commands: &mut Vec<CommandOperation>,
    root: &str,
    old: DeckTransform,
    new: DeckTransform,
) {
    for axis in 0..2 {
        changed_f32(
            commands,
            format!("{root}.transform.position.{axis}"),
            old.position[axis],
            new.position[axis],
        );
    }
    changed_f32(
        commands,
        format!("{root}.transform.scale"),
        old.scale,
        new.scale,
    );
    changed_f32(
        commands,
        format!("{root}.transform.rotation"),
        old.rotation,
        new.rotation,
    );
    changed_bool(
        commands,
        format!("{root}.transform.flip_horizontal"),
        old.flip_horizontal,
        new.flip_horizontal,
    );
    changed_bool(
        commands,
        format!("{root}.transform.flip_vertical"),
        old.flip_vertical,
        new.flip_vertical,
    );
    for edge in 0..4 {
        changed_f32(
            commands,
            format!("{root}.transform.crop.{edge}"),
            old.crop[edge],
            new.crop[edge],
        );
    }
    changed_text(
        commands,
        format!("{root}.transform.source_mode"),
        source_name(old.source_mode),
        source_name(new.source_mode),
    );
}

fn diff_deck_effect_structure(
    commands: &mut Vec<CommandOperation>,
    root: &str,
    old_slots: [oneiroi_render::EffectSlot; 3],
    new_slots: [oneiroi_render::EffectSlot; 3],
    old_lfos: DeckLfos,
    new_lfos: DeckLfos,
) {
    for slot in 0..3 {
        let path = format!("{root}.effects.slot.{slot}");
        changed_text(
            commands,
            format!("{path}.group"),
            group_name(old_slots[slot].group),
            group_name(new_slots[slot].group),
        );
        changed_bool(
            commands,
            format!("{path}.bypassed"),
            old_slots[slot].bypassed,
            new_slots[slot].bypassed,
        );
        changed_f32(
            commands,
            format!("{path}.mix"),
            old_slots[slot].mix,
            new_slots[slot].mix,
        );
        let old = old_lfos.lanes[slot];
        let new = new_lfos.lanes[slot];
        let lfo = format!("{root}.lfo.{slot}");
        changed_bool(
            commands,
            format!("{lfo}.direct_enabled"),
            old.direct_enabled,
            new.direct_enabled,
        );
        changed_text(
            commands,
            format!("{lfo}.target"),
            target_name(old.target),
            target_name(new.target),
        );
        changed_text(
            commands,
            format!("{lfo}.waveform"),
            waveform_name(old.waveform),
            waveform_name(new.waveform),
        );
        changed_bool(
            commands,
            format!("{lfo}.tempo_sync"),
            old.tempo_sync,
            new.tempo_sync,
        );
        changed_f32(
            commands,
            format!("{lfo}.beats_per_cycle"),
            old.beats_per_cycle,
            new.beats_per_cycle,
        );
    }
    for route in 0..8 {
        let path = format!("{root}.modulation.route.{route}");
        changed_i64(
            commands,
            format!("{path}.source"),
            i64::from(old_lfos.routes[route].source),
            i64::from(new_lfos.routes[route].source),
        );
        changed_text(
            commands,
            format!("{path}.target"),
            target_name(old_lfos.routes[route].target),
            target_name(new_lfos.routes[route].target),
        );
    }
}

fn diff_master_effects(
    commands: &mut Vec<CommandOperation>,
    old: &MasterEffectChain,
    new: &MasterEffectChain,
) {
    for slot in 0..old.slots.len() {
        let old = &old.slots[slot];
        let new = &new.slots[slot];
        let root = format!("master.effect.{slot}");
        changed_text(
            commands,
            format!("{root}.kind"),
            master_kind_name(old.kind),
            master_kind_name(new.kind),
        );
        changed_bool(
            commands,
            format!("{root}.bypassed"),
            old.bypassed,
            new.bypassed,
        );
        changed_f32(commands, format!("{root}.mix"), old.mix, new.mix);
        changed_f32(commands, format!("{root}.amount"), old.amount, new.amount);
        changed_f32(
            commands,
            format!("{root}.feedback"),
            old.feedback,
            new.feedback,
        );
        changed_text(
            commands,
            format!("{root}.package_id"),
            &old.package_id,
            &new.package_id,
        );
        let old_ids = old
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}");
        let new_ids = new
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}");
        changed_text(
            commands,
            format!("{root}.parameter_ids"),
            &old_ids,
            &new_ids,
        );
        // Existing custom parameter values travel through the continuous
        // ControlTarget gateway. Emit their initial values here only when the
        // package/parameter identity changes and those targets did not exist
        // in the pre-frame snapshot.
        if old.package_id != new.package_id || old_ids != new_ids {
            for parameter in &new.parameters {
                commands.push(command(
                    format!("{root}.parameter.{}", parameter.id),
                    ParameterValue::Scalar(f64::from(parameter.value)),
                ));
            }
        }
    }
}

fn diff_master_modulation(
    commands: &mut Vec<CommandOperation>,
    old: MasterModulation,
    new: MasterModulation,
) {
    for index in 0..old.lfos.len() {
        let old = old.lfos[index];
        let new = new.lfos[index];
        let root = format!("master.lfo.{index}");
        changed_bool(
            commands,
            format!("{root}.enabled"),
            old.enabled,
            new.enabled,
        );
        changed_text(
            commands,
            format!("{root}.waveform"),
            waveform_name(old.waveform),
            waveform_name(new.waveform),
        );
        changed_f32(
            commands,
            format!("{root}.rate_hz"),
            old.rate_hz,
            new.rate_hz,
        );
        changed_bool(
            commands,
            format!("{root}.tempo_sync"),
            old.tempo_sync,
            new.tempo_sync,
        );
        changed_f32(
            commands,
            format!("{root}.beats_per_cycle"),
            old.beats_per_cycle,
            new.beats_per_cycle,
        );
        changed_f32(commands, format!("{root}.depth"), old.depth, new.depth);
        changed_f32(commands, format!("{root}.phase"), old.phase, new.phase);
    }
    for index in 0..old.routes.len() {
        let old = old.routes[index];
        let new = new.routes[index];
        let root = format!("master.modulation.route.{index}");
        changed_bool(
            commands,
            format!("{root}.enabled"),
            old.enabled,
            new.enabled,
        );
        changed_i64(
            commands,
            format!("{root}.source"),
            i64::from(old.source),
            i64::from(new.source),
        );
        changed_i64(
            commands,
            format!("{root}.target_slot"),
            i64::from(old.target_slot),
            i64::from(new.target_slot),
        );
        if old.parameter_key != new.parameter_key {
            commands.push(command(
                format!("{root}.parameter_key"),
                ParameterValue::Text(new.parameter_key.to_string()),
            ));
        }
        changed_f32(commands, format!("{root}.amount"), old.amount, new.amount);
    }
}

fn bus_name(value: CrossfadeBus) -> &'static str {
    match value {
        CrossfadeBus::Left => "left",
        CrossfadeBus::Right => "right",
    }
}
fn group_name(value: EffectGroup) -> &'static str {
    match value {
        EffectGroup::Color => "color",
        EffectGroup::Geometry => "geometry",
        EffectGroup::Stylize => "stylize",
    }
}
fn source_name(value: SourceMode) -> &'static str {
    match value {
        SourceMode::Fit => "fit",
        SourceMode::Fill => "fill",
        SourceMode::Stretch => "stretch",
    }
}
fn blend_name(value: LayerBlendMode) -> &'static str {
    value.name()
}
fn waveform_name(value: LfoWaveform) -> &'static str {
    match value {
        LfoWaveform::Sine => "sine",
        LfoWaveform::Triangle => "triangle",
        LfoWaveform::Saw => "saw",
        LfoWaveform::SawDown => "saw_down",
        LfoWaveform::Square => "square",
    }
}
fn master_kind_name(value: MasterEffectKind) -> &'static str {
    match value {
        MasterEffectKind::None => "none",
        MasterEffectKind::Blur => "blur",
        MasterEffectKind::Feedback => "feedback",
        MasterEffectKind::Custom => "custom",
    }
}
fn target_name(value: EffectTarget) -> &'static str {
    match value {
        EffectTarget::Hue => "hue",
        EffectTarget::Contrast => "contrast",
        EffectTarget::Saturation => "saturation",
        EffectTarget::BlackLevel => "black_level",
        EffectTarget::WhiteLevel => "white_level",
        EffectTarget::Gamma => "gamma",
        EffectTarget::Pixelate => "pixelate",
        EffectTarget::LumaKey => "luma_key",
        EffectTarget::Neon => "neon",
        EffectTarget::Fractal => "fractal",
        EffectTarget::Jitter => "jitter",
        EffectTarget::FindEdges => "find_edges",
        EffectTarget::BitReduction => "bit_reduction",
        EffectTarget::Blacklight => "blacklight",
        EffectTarget::Bloom => "bloom",
        EffectTarget::BloomThreshold => "bloom_threshold",
        EffectTarget::BloomRadius => "bloom_radius",
        EffectTarget::BloomChroma => "bloom_chroma",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_stable_field_commands_for_structural_changes() {
        let mixer = FourDeckMixer::default();
        let ui = UiState::default();
        let before = StructuralSnapshot::capture(&ui, &mixer);
        let mut after = before.clone();
        after.transforms[0].crop[2] = 0.25;
        after.blend_modes[1] = LayerBlendMode::Difference;
        after.lfos[2].lanes[1].waveform = LfoWaveform::Square;
        after.master_effects.slots[0].kind = MasterEffectKind::Blur;

        let commands = after_paths(before.commands_to(&after));
        assert!(
            commands
                .iter()
                .any(|path| path == "deck.0.transform.crop.2")
        );
        assert!(commands.iter().any(|path| path == "deck.1.blend_mode"));
        assert!(commands.iter().any(|path| path == "deck.2.lfo.1.waveform"));
        assert!(commands.iter().any(|path| path == "master.effect.0.kind"));
    }

    #[test]
    fn snapshot_apply_restores_editor_and_mixer_structure() {
        let mut mixer = FourDeckMixer::default();
        let mut ui = UiState::default();
        let snapshot = StructuralSnapshot::capture(&ui, &mixer);
        mixer.deck_mut(DeckId::A).bus = CrossfadeBus::Right;
        ui.transforms[0].scale = 2.0;
        ui.master_modulation.lfos[0].enabled = true;

        snapshot.apply(&mut ui, &mut mixer);
        assert_eq!(mixer.deck(DeckId::A).bus, CrossfadeBus::Left);
        assert_eq!(ui.transforms[0].scale, 1.0);
        assert!(!ui.master_modulation.lfos[0].enabled);
    }

    #[test]
    fn emitted_paths_restore_concrete_structural_state() {
        let mixer = FourDeckMixer::default();
        let ui = UiState::default();
        let before = StructuralSnapshot::capture(&ui, &mixer);
        let mut desired = before.clone();
        desired.buses[0] = CrossfadeBus::Right;
        desired.transforms[0].source_mode = SourceMode::Fill;
        desired.effect_slots[1][2].group = EffectGroup::Geometry;
        desired.lfos[2].lanes[1].waveform = LfoWaveform::Square;
        desired.lfos[2].routes[3].target = EffectTarget::Jitter;
        desired.master_effects.slots[0].kind = MasterEffectKind::Feedback;
        desired.master_modulation.routes[0].target_slot = 1;
        let parameters = before
            .commands_to(&desired)
            .into_iter()
            .filter_map(|command| match command {
                CommandOperation::SetParameter { path, value } => Some((path, value)),
                _ => None,
            })
            .collect();
        let mut restored_ui = UiState::default();
        let mut restored_mixer = FourDeckMixer::default();

        apply_session_parameters(&mut restored_ui, &mut restored_mixer, &parameters);

        assert_eq!(
            StructuralSnapshot::capture(&restored_ui, &restored_mixer),
            desired
        );
    }

    fn after_paths(commands: Vec<CommandOperation>) -> Vec<String> {
        commands
            .into_iter()
            .filter_map(|command| match command {
                CommandOperation::SetParameter { path, .. } => Some(path),
                _ => None,
            })
            .collect()
    }
}
