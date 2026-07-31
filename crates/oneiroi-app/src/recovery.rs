//! Operator-facing discovery and restoration of crash-safe session journals.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use oneiroi_core::Quantization;
use oneiroi_graph::ParameterValue;
use oneiroi_media::{ClipAddress, DeckId};
use oneiroi_session::{SessionState, ShowTime, control_parameter_path, recover_journal};

use super::{State, performance_control_snapshot, structural};

#[derive(Clone, Debug)]
pub(crate) struct RecoveryEntry {
    pub journal_path: PathBuf,
    pub take_name: String,
    pub command_count: u64,
    pub checkpointed: bool,
    pub ignored_partial_tail: bool,
    pub latest_time: ShowTime,
    pub project_linked: bool,
    state: SessionState,
}

impl RecoveryEntry {
    pub(crate) fn file_name(&self) -> String {
        self.journal_path.file_name().map_or_else(
            || self.journal_path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

impl State {
    pub(crate) fn refresh_session_recoveries(&mut self) {
        let directory = self.workspace.join(".oneiroi/session");
        match discover_recoveries(
            &directory,
            self.performance_runtime.journal_path(),
            &self.project_id,
        ) {
            Ok((entries, rejected, foreign)) => {
                let count = entries.len();
                self.session_recoveries = entries;
                self.ui.session_recovery_selected = self
                    .ui
                    .session_recovery_selected
                    .min(count.saturating_sub(1));
                self.session_recovery_status = if rejected == 0 && foreign == 0 {
                    format!("Found {count} project/legacy session(s)")
                } else {
                    format!(
                        "Found {count} project/legacy session(s) · {foreign} other project · {rejected} invalid"
                    )
                };
            }
            Err(error) => {
                self.session_recoveries.clear();
                self.session_recovery_status = format!("Session scan failed: {error:#}");
            }
        }
    }

    pub(crate) fn restore_session_recovery(&mut self, index: usize, now: Instant) {
        let Some(entry) = self.session_recoveries.get(index).cloned() else {
            self.session_recovery_status = "Select a recoverable session first".to_owned();
            return;
        };
        match self.apply_recovered_session(&entry, now) {
            Ok(()) => {
                self.refresh_session_recoveries();
                self.session_recovery_status = format!(
                    "Restored {} · continuing in a fresh journal",
                    entry.take_name
                );
            }
            Err(error) => {
                self.session_recovery_status = format!("Session restore failed: {error:#}");
            }
        }
    }

    fn apply_recovered_session(&mut self, entry: &RecoveryEntry, now: Instant) -> Result<()> {
        let state = &entry.state;
        let previous_extent = self.performance_runtime.render_plan().extent();
        self.performance_runtime
            .set_composition_extent(state.output_extent)
            .context("validate recovered composition extent")?;
        self.remember_active_take();
        if let Err(error) = self.performance_runtime.restore_baseline(
            state.clone(),
            &entry.take_name,
            self.show_time_at(now),
        ) {
            if let Err(rollback_error) = self
                .performance_runtime
                .set_composition_extent(previous_extent)
            {
                log::error!("recovery graph rollback failed: {rollback_error:#}");
            }
            return Err(error).context("start recovered take");
        }
        self.performance_started = now;

        structural::apply_session_parameters(&mut self.ui, &mut self.mixer, &state.parameters);
        if let Some(ParameterValue::Text(value)) = state.parameters.get("launch.quantization") {
            self.ui.quantization = match value.as_str() {
                "immediate" => Quantization::Immediate,
                "beat" => Quantization::Beat,
                "bar" => Quantization::Bar,
                _ => self.ui.quantization,
            };
        }
        if let Some(ParameterValue::Text(value)) = state.parameters.get("output.display_id") {
            self.ui.output_display_id.clone_from(value);
        }
        let targets = performance_control_snapshot(&self.ui, &self.mixer, &self.transports);
        for target in targets.keys().copied() {
            let Some(ParameterValue::Scalar(value)) =
                state.parameters.get(&control_parameter_path(target))
            else {
                continue;
            };
            if value.is_finite() && *value >= f64::from(f32::MIN) && *value <= f64::from(f32::MAX) {
                self.apply_control_update_unrecorded(
                    oneiroi_core::ControlUpdate {
                        target,
                        value: *value as f32,
                    },
                    now,
                );
            }
        }

        self.ui.bpm = state.bpm;
        self.tempo.set_bpm(
            state.bpm,
            now.saturating_duration_since(self.performance_started)
                .as_secs_f64(),
        );
        self.ui.blackout = state.blackout;
        self.ui.output_enabled = state.output_enabled;
        self.ui.output_fullscreen = state.output_fullscreen;
        self.ui.composition_extent = state.output_extent;
        self.ui.custom_composition_extent = state.output_extent;
        let _ = self.apply_output_settings();

        for deck in DeckId::ALL {
            let index = deck.index();
            match state.active_clips[index] {
                Some(slot)
                    if self
                        .clips
                        .movie(ClipAddress {
                            deck,
                            slot: usize::from(slot),
                        })
                        .is_some() =>
                {
                    self.launch_clip(ClipAddress {
                        deck,
                        slot: usize::from(slot),
                    });
                }
                Some(_) => {}
                None => {
                    self.mixer.eject(deck);
                    self.live_configs[index] = None;
                    self.clips.deactivate(deck);
                    self.launches.cancel(deck);
                    let generation = self.mixer.deck(deck).generation;
                    self.reset_playback(deck, generation);
                }
            }
            self.transports[index].position = state.deck_positions[index];
            if state.deck_positions[index] > 0.0 && self.clips.active(deck).is_some() {
                self.seek_deck(deck);
            }
        }
        self.master_effect_processor.reset_history();
        Ok(())
    }

    pub(crate) fn session_state_snapshot(&self) -> SessionState {
        let mut parameters = structural::session_parameters(&self.ui, &self.mixer);
        for (target, value) in performance_control_snapshot(&self.ui, &self.mixer, &self.transports)
        {
            parameters.insert(
                control_parameter_path(target),
                ParameterValue::Scalar(f64::from(value)),
            );
        }
        parameters.insert(
            "launch.quantization".to_owned(),
            ParameterValue::Text(
                match self.ui.quantization {
                    Quantization::Immediate => "immediate",
                    Quantization::Beat => "beat",
                    Quantization::Bar => "bar",
                }
                .to_owned(),
            ),
        );
        parameters.insert(
            "output.display_id".to_owned(),
            ParameterValue::Text(self.ui.output_display_id.clone()),
        );
        SessionState {
            bpm: self.ui.bpm,
            output_enabled: self.ui.output_enabled,
            output_fullscreen: self.ui.output_fullscreen,
            output_extent: self.ui.composition_extent,
            blackout: self.ui.blackout,
            active_clips: std::array::from_fn(|index| {
                self.clips
                    .active(DeckId::ALL[index])
                    .and_then(|slot| u8::try_from(slot).ok())
            }),
            deck_positions: self.transports.map(|transport| transport.position),
            parameters,
            ..SessionState::default()
        }
    }

    pub(crate) fn remember_active_take(&mut self) {
        let Some(active) = self.performance_runtime.take_metadata() else {
            return;
        };
        if let Some(existing) = self
            .project_takes
            .iter_mut()
            .find(|take| take.take_id == active.take_id)
        {
            *existing = active;
        } else {
            self.project_takes.push(active);
        }
        if self.project_takes.len() > 256 {
            let remove = self.project_takes.len() - 256;
            self.project_takes.drain(..remove);
        }
    }
}

fn discover_recoveries(
    directory: &Path,
    active_journal: Option<&Path>,
    project_id: &str,
) -> Result<(Vec<RecoveryEntry>, usize, usize)> {
    if !directory.exists() {
        return Ok((Vec::new(), 0, 0));
    }
    let mut paths = fs::read_dir(directory)
        .with_context(|| format!("read {}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .filter(|path| active_journal != Some(path.as_path()))
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| {
        std::cmp::Reverse(
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .ok(),
        )
    });
    let mut entries = Vec::new();
    let mut rejected = 0;
    let mut foreign = 0;
    for journal_path in paths {
        let checkpoint_path = journal_path.with_extension("checkpoint.json");
        let recovery = match recover_journal(&journal_path, &checkpoint_path) {
            Ok(recovery) => recovery,
            Err(_) => {
                rejected += 1;
                continue;
            }
        };
        let state = match recovery.replay_state() {
            Ok(state) => state,
            Err(_) => {
                rejected += 1;
                continue;
            }
        };
        if recovery
            .project_id
            .as_deref()
            .is_some_and(|identity| identity != project_id)
        {
            foreign += 1;
            continue;
        }
        let latest_time = recovery.latest_time();
        entries.push(RecoveryEntry {
            journal_path,
            take_name: recovery.take_name,
            command_count: state
                .last_sequence
                .map_or(0, |sequence| sequence.saturating_add(1)),
            checkpointed: recovery.checkpoint.is_some(),
            ignored_partial_tail: recovery.ignored_partial_tail,
            latest_time,
            project_linked: recovery.project_id.as_deref() == Some(project_id),
            state,
        });
    }
    Ok((entries, rejected, foreign))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use oneiroi_io::new_project_id;
    use oneiroi_session::{CommandOperation, CommandOrigin, JournalWriter};

    use super::*;

    #[test]
    fn catalog_excludes_active_journal_and_replays_valid_entries() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("oneiroi-recovery-catalog-{unique}"));
        let old_path = directory.join("old.jsonl");
        let old_checkpoint = directory.join("old.checkpoint.json");
        let writer = JournalWriter::open(&old_path, old_checkpoint, "Old take", 4).unwrap();
        writer
            .try_append(oneiroi_session::ShowCommand {
                sequence: 0,
                command_id: 1,
                origin: CommandOrigin::Operator,
                execute_at: ShowTime::default(),
                operation: CommandOperation::SetTempo { bpm: 132.0 },
            })
            .unwrap();
        writer.flush().unwrap();
        drop(writer);
        let active_path = directory.join("active.jsonl");
        let active = JournalWriter::open(
            &active_path,
            directory.join("active.checkpoint.json"),
            "Live",
            4,
        )
        .unwrap();
        active.flush().unwrap();

        let project_id = new_project_id();
        let linked_path = directory.join("linked.jsonl");
        let linked = JournalWriter::open_linked(
            &linked_path,
            directory.join("linked.checkpoint.json"),
            "Linked",
            Some(project_id.clone()),
            Some(new_project_id()),
            4,
        )
        .unwrap();
        linked.flush().unwrap();
        drop(linked);
        let foreign = JournalWriter::open_linked(
            directory.join("foreign.jsonl"),
            directory.join("foreign.checkpoint.json"),
            "Foreign",
            Some(new_project_id()),
            Some(new_project_id()),
            4,
        )
        .unwrap();
        foreign.flush().unwrap();
        drop(foreign);

        let (entries, rejected, foreign) =
            discover_recoveries(&directory, Some(&active_path), &project_id).unwrap();
        assert_eq!(rejected, 0);
        assert_eq!(foreign, 1);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| {
            entry.take_name == "Old take" && !entry.project_linked && entry.state.bpm == 132.0
        }));
        assert!(
            entries
                .iter()
                .any(|entry| entry.take_name == "Linked" && entry.project_linked)
        );
    }
}
