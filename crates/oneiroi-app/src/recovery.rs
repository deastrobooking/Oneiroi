//! Operator-facing discovery and restoration of crash-safe session journals.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use oneiroi_core::Quantization;
use oneiroi_graph::ParameterValue;
use oneiroi_media::{ClipAddress, DeckId};
use oneiroi_session::{
    JournalRecovery, SessionState, ShowTime, control_parameter_path, recover_journal,
};

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
    timeline: JournalRecovery,
}

impl RecoveryEntry {
    pub(crate) fn file_name(&self) -> String {
        self.journal_path.file_name().map_or_else(
            || self.journal_path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    pub(crate) fn markers(&self) -> &[oneiroi_session::TimelineMarker] {
        &self.timeline.markers
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
        let target = self
            .session_recoveries
            .get(index)
            .map_or(0, |entry| entry.latest_time.monotonic_ns);
        self.restore_session_recovery_at(index, target, now);
    }

    pub(crate) fn restore_session_recovery_at(
        &mut self,
        index: usize,
        monotonic_ns: u64,
        now: Instant,
    ) {
        let Some(entry) = self.session_recoveries.get(index).cloned() else {
            self.session_recovery_status = "Select a recoverable session first".to_owned();
            return;
        };
        let state = match entry.timeline.replay_at(ShowTime {
            monotonic_ns,
            ..entry.latest_time
        }) {
            Ok(state) => state,
            Err(error) => {
                self.session_recovery_status = format!("Timeline replay failed: {error}");
                return;
            }
        };
        let branch_name = valid_take_name(&self.ui.take_name_input).map(ToOwned::to_owned);
        match self.apply_recovered_session(&entry, &state, branch_name.as_deref(), now) {
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

    fn apply_recovered_session(
        &mut self,
        entry: &RecoveryEntry,
        state: &SessionState,
        branch_name: Option<&str>,
        now: Instant,
    ) -> Result<()> {
        let previous_extent = self.performance_runtime.render_plan().extent();
        self.performance_runtime
            .set_composition_extent(state.output_extent)
            .context("validate recovered composition extent")?;
        self.remember_active_take();
        let restore = if let Some(name) = branch_name {
            self.performance_runtime.start_named_baseline(
                state.clone(),
                name,
                self.show_time_at(now),
            )
        } else {
            self.performance_runtime.restore_baseline(
                state.clone(),
                &entry.take_name,
                self.show_time_at(now),
            )
        };
        if let Err(error) = restore {
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

    pub(crate) fn start_named_take(&mut self, now: Instant) {
        let Some(name) = valid_take_name(&self.ui.take_name_input).map(ToOwned::to_owned) else {
            self.session_recovery_status =
                "Take name must be 1–128 printable characters".to_owned();
            return;
        };
        self.remember_active_take();
        let state = self.session_state_snapshot();
        match self
            .performance_runtime
            .start_named_baseline(state, &name, self.show_time_at(now))
        {
            Ok(()) => {
                self.refresh_session_recoveries();
                self.session_recovery_status = format!("Recording named take · {name}");
            }
            Err(error) => {
                self.session_recovery_status = format!("Start take failed: {error:#}");
            }
        }
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
            random_seeds: self.performance_runtime.random_seeds().clone(),
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

    pub(crate) fn rename_project_take(&mut self, index: usize) {
        let Some(name) = valid_take_name(&self.ui.take_name_input).map(ToOwned::to_owned) else {
            self.session_recovery_status = "Enter a valid take name first".to_owned();
            return;
        };
        let Some(previous) = rename_take_metadata(&mut self.project_takes, index, &name) else {
            self.session_recovery_status = "Select project take metadata first".to_owned();
            return;
        };
        self.session_recovery_status = format!("Renamed {previous} to {name} · save project");
    }

    pub(crate) fn remove_project_take(&mut self, index: usize) {
        let Some(removed) = remove_take_metadata(&mut self.project_takes, index) else {
            self.session_recovery_status = "Select project take metadata first".to_owned();
            return;
        };
        self.ui.project_take_selected = self
            .ui
            .project_take_selected
            .min(self.project_takes.len().saturating_sub(1));
        self.session_recovery_status = format!(
            "Removed {} from project metadata · journal file retained",
            removed.name
        );
    }

    pub(crate) fn add_timeline_marker(&mut self, now: Instant) {
        let Some(label) = valid_take_name(&self.ui.timeline_marker_input).map(ToOwned::to_owned)
        else {
            self.session_recovery_status = "Marker must be 1–128 printable characters".to_owned();
            return;
        };
        match self
            .performance_runtime
            .add_timeline_marker(self.show_time_at(now), label.clone())
        {
            Ok(()) => {
                self.session_recovery_status = format!("Added timeline marker · {label}");
                self.ui.timeline_marker_input.clear();
            }
            Err(error) => {
                self.session_recovery_status = format!("Add marker failed: {error:#}");
            }
        }
    }

    pub(crate) fn export_project_take(&mut self, index: usize, archive: bool) {
        let Some(take) = self.project_takes.get(index) else {
            self.session_recovery_status = "Select project take metadata first".to_owned();
            return;
        };
        let destination_root = if archive {
            self.workspace.join(".oneiroi/archive")
        } else {
            let value = self.ui.take_export_directory.trim();
            if value.is_empty() {
                self.session_recovery_status = "Enter an export directory first".to_owned();
                return;
            }
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                self.workspace.join(path)
            }
        };
        match copy_take_bundle(
            take,
            &self.workspace.join(".oneiroi/session"),
            &destination_root,
        ) {
            Ok(destination) => {
                self.session_recovery_status = format!(
                    "{} copy created at {}",
                    if archive { "Archive" } else { "Export" },
                    destination.display()
                );
            }
            Err(error) => {
                self.session_recovery_status = format!(
                    "{} failed: {error:#}",
                    if archive { "Archive" } else { "Export" }
                );
            }
        }
    }
}

fn copy_take_bundle(
    take: &oneiroi_io::TakeMetadataProject,
    source_directory: &Path,
    destination_root: &Path,
) -> Result<PathBuf> {
    let journal_source = source_directory.join(&take.journal_file);
    if !journal_source.is_file() {
        anyhow::bail!("journal is missing: {}", journal_source.display());
    }
    fs::create_dir_all(destination_root)
        .with_context(|| format!("create {}", destination_root.display()))?;
    let destination =
        destination_root.join(format!("{}-{}", take.take_id, oneiroi_io::new_project_id()));
    fs::create_dir(&destination).with_context(|| format!("create {}", destination.display()))?;
    let journal_destination = destination.join(&take.journal_file);
    fs::copy(&journal_source, &journal_destination).with_context(|| {
        format!(
            "copy {} to {}",
            journal_source.display(),
            journal_destination.display()
        )
    })?;
    let checkpoint_source = journal_source.with_extension("checkpoint.json");
    if checkpoint_source.is_file() {
        let checkpoint_name = checkpoint_source
            .file_name()
            .context("checkpoint has no filename")?;
        fs::copy(&checkpoint_source, destination.join(checkpoint_name))
            .with_context(|| format!("copy checkpoint {}", checkpoint_source.display()))?;
    }
    Ok(destination)
}

fn valid_take_name(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control))
        .then_some(value)
}

fn rename_take_metadata(
    takes: &mut [oneiroi_io::TakeMetadataProject],
    index: usize,
    name: &str,
) -> Option<String> {
    let take = takes.get_mut(index)?;
    let previous = std::mem::replace(&mut take.name, name.to_owned());
    Some(previous)
}

fn remove_take_metadata(
    takes: &mut Vec<oneiroi_io::TakeMetadataProject>,
    index: usize,
) -> Option<oneiroi_io::TakeMetadataProject> {
    (index < takes.len()).then(|| takes.remove(index))
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
            take_name: recovery.take_name.clone(),
            command_count: state
                .last_sequence
                .map_or(0, |sequence| sequence.saturating_add(1)),
            checkpointed: recovery.checkpoint.is_some(),
            ignored_partial_tail: recovery.ignored_partial_tail,
            latest_time,
            project_linked: recovery.project_id.as_deref() == Some(project_id),
            timeline: recovery,
        });
    }
    Ok((entries, rejected, foreign))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use oneiroi_io::new_project_id;
    use oneiroi_session::{CommandOperation, CommandOrigin, JournalWriter, StateCheckpoint};

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
            entry.take_name == "Old take"
                && !entry.project_linked
                && entry.timeline.replay_state().unwrap().bpm == 132.0
        }));
        assert!(
            entries
                .iter()
                .any(|entry| entry.take_name == "Linked" && entry.project_linked)
        );
    }

    #[test]
    fn take_names_reject_empty_control_or_oversized_values() {
        assert_eq!(valid_take_name("  Finale  "), Some("Finale"));
        assert_eq!(valid_take_name(""), None);
        assert_eq!(valid_take_name("bad\nname"), None);
        assert_eq!(valid_take_name(&"x".repeat(129)), None);
    }

    #[test]
    fn take_metadata_can_be_renamed_and_unlinked_without_deleting_files() {
        let mut takes = vec![oneiroi_io::TakeMetadataProject {
            take_id: new_project_id(),
            name: "Old".to_owned(),
            journal_file: "old.jsonl".to_owned(),
            created_unix_ms: 1,
        }];
        assert_eq!(
            rename_take_metadata(&mut takes, 0, "Finale"),
            Some("Old".to_owned())
        );
        let removed = remove_take_metadata(&mut takes, 0).unwrap();
        assert_eq!(removed.name, "Finale");
        assert_eq!(removed.journal_file, "old.jsonl");
        assert!(takes.is_empty());
    }

    #[test]
    fn take_exports_are_unique_copies_and_preserve_the_source_bundle() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("oneiroi-take-export-{unique}"));
        let source = root.join("session");
        let exports = root.join("exports");
        let journal = source.join("finale.jsonl");
        let checkpoint = source.join("finale.checkpoint.json");
        let writer = JournalWriter::open(&journal, &checkpoint, "Finale", 4).unwrap();
        writer
            .try_checkpoint(StateCheckpoint {
                after_sequence: None,
                at: ShowTime::default(),
                state: SessionState::default(),
            })
            .unwrap();
        writer.flush().unwrap();
        drop(writer);
        let take = oneiroi_io::TakeMetadataProject {
            take_id: new_project_id(),
            name: "Finale".to_owned(),
            journal_file: "finale.jsonl".to_owned(),
            created_unix_ms: 1,
        };

        let first = copy_take_bundle(&take, &source, &exports).unwrap();
        let second = copy_take_bundle(&take, &source, &exports).unwrap();

        assert_ne!(first, second);
        for destination in [&first, &second] {
            assert!(destination.join("finale.jsonl").is_file());
            assert!(destination.join("finale.checkpoint.json").is_file());
        }
        assert!(journal.is_file());
        assert!(checkpoint.is_file());
    }
}
