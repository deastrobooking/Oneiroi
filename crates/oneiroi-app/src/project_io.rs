//! Project snapshot, save/open, restore polling and autosave.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use oneiroi_core::MediaTime;
use oneiroi_io::{
    ProjectFile, autosave_path, load_project, recovery_is_newer, save_project_atomic,
};
use oneiroi_media::{ClipAddress, ClipBank, ClipRestoreRequest, DeckId, LaunchQueue};
use oneiroi_session::{CommandOperation, CommandOrigin};

use super::{State, display_path, project, resolve_project_paths};

impl State {
    pub(crate) fn project_snapshot(&self) -> ProjectFile {
        let mut takes = self.project_takes.clone();
        if let Some(active) = self.performance_runtime.take_metadata() {
            if let Some(existing) = takes.iter_mut().find(|take| take.take_id == active.take_id) {
                *existing = active;
            } else {
                takes.push(active);
            }
        }
        if takes.len() > 256 {
            takes.sort_by_key(|take| take.created_unix_ms);
            let remove = takes.len() - 256;
            takes.drain(..remove);
        }
        project::snapshot(
            &self.ui,
            &self.mixer,
            &self.clips,
            &self.transports,
            &self.midi,
            &self.live_configs,
            project::ProjectSessionMetadata {
                project_id: &self.project_id,
                takes,
            },
        )
    }

    pub(crate) fn project_dirty(&self) -> bool {
        project::is_dirty(&self.project_snapshot(), self.last_saved_project.as_ref())
    }

    pub(crate) fn path_from_ui(&self) -> Option<PathBuf> {
        let value = self.ui.project_path.trim();
        if value.is_empty() {
            return None;
        }
        let path = PathBuf::from(value);
        Some(if path.is_absolute() {
            path
        } else {
            self.workspace.join(path)
        })
    }

    pub(crate) fn save_project_from_ui(&mut self) {
        let Some(path) = self.path_from_ui() else {
            self.project_status = "Enter a project path first.".to_owned();
            return;
        };
        let snapshot = self.project_snapshot();
        match save_project_atomic(&path, &snapshot) {
            Ok(()) => {
                self.project_path = Some(path.clone());
                self.last_saved_project = Some(snapshot);
                self.recovery_path = None;
                self.project_status = format!("Saved {}", display_path(&path));
            }
            Err(error) => self.project_status = format!("Save failed: {error}"),
        }
    }

    pub(crate) fn open_project_from_ui(&mut self) {
        let Some(path) = self.path_from_ui() else {
            self.project_status = "Enter a project path first.".to_owned();
            return;
        };
        self.open_project(path, false);
    }

    pub(crate) fn open_project(&mut self, path: PathBuf, recovered: bool) {
        match load_project(&path) {
            Ok(mut project_file) => {
                let base = path.parent().unwrap_or(&self.workspace);
                resolve_project_paths(&mut project_file, base);
                self.apply_project(project_file, recovered);
                if recovered {
                    self.project_path = None;
                    self.recovery_path = None;
                    self.ui.project_path = "recovered-show.oneiroi".to_owned();
                    self.project_status =
                        format!("Recovered autosave from {}", display_path(&path));
                } else {
                    self.project_path = Some(path.clone());
                    self.ui.project_path = path.to_string_lossy().into_owned();
                    let recovery = autosave_path(Some(&path), &self.workspace);
                    self.recovery_path = recovery_is_newer(&path, &recovery).then_some(recovery);
                    self.project_status = format!("Opened {}", display_path(&path));
                }
            }
            Err(error) => self.project_status = format!("Open failed: {error}"),
        }
    }

    pub(crate) fn apply_project(&mut self, project_file: ProjectFile, recovered: bool) {
        self.master_effect_processor.reset_history();
        self.project_id.clone_from(&project_file.project_id);
        self.project_takes.clone_from(&project_file.takes);
        self.project_epoch = self.project_epoch.wrapping_add(1);
        self.clips = ClipBank::default();
        self.ui.clear_thumbnails();
        self.thumbnail_requests.clear();
        self.folder_pending.clear();
        self.relink_pending.clear();
        self.relink_active.clear();
        self.folder_status.clear();
        self.live_configs = std::array::from_fn(|_| None);
        self.launches = LaunchQueue::default();
        self.restore_active = [None; 4];
        self.restore_selected = [0; 4];
        self.restore_transport = [None; 4];
        project::apply_master(&project_file, &mut self.ui);
        let _ = self.apply_output_settings();
        self.midi = project::apply_midi(&project_file);

        for deck in DeckId::ALL {
            let index = deck.index();
            self.mixer.eject(deck);
            let generation = self.mixer.deck(deck).generation;
            self.reset_playback(deck, generation);
            let deck_project = &project_file.decks[index];
            let transport = project::apply_deck(deck, deck_project, &mut self.mixer, &mut self.ui);
            self.transports[index] = transport;
            self.clips.select(ClipAddress {
                deck,
                slot: deck_project.selected_slot,
            });
            self.restore_selected[index] = deck_project.selected_slot;
            self.restore_active[index] = deck_project.active_slot;
            self.clips.restore_active(deck, deck_project.active_slot);
            self.restore_transport[index] = deck_project.active_slot.map(|_| transport);

            for (slot, path) in deck_project.clips.iter().enumerate() {
                let address = ClipAddress { deck, slot };
                let path = path.clone();
                if let Some(path) = &path {
                    self.clips.begin_restore(address, path.clone());
                }
                if let Some(playback) = deck_project.clip_playback.get(slot) {
                    self.clips
                        .set_playback(address, project::clip_playback_from_project(*playback));
                }
                let Some(path) = path else {
                    continue;
                };
                if let Err(request) = self.restorer.submit(ClipRestoreRequest {
                    address,
                    path,
                    project_epoch: self.project_epoch,
                }) {
                    self.clips.fail_restore(
                        request.address,
                        request.path,
                        "Restore queue is full.".to_owned(),
                    );
                }
            }
            if let Some(camera) = &deck_project.camera {
                let config = project::camera_from_project(camera);
                let generation = self.mixer.connect_camera(deck, config.clone());
                self.live_configs[index] = Some(config.clone());
                self.reset_playback(deck, generation);
                self.transports[index] = transport;
                self.transports[index].end_mode = oneiroi_media::EndMode::OneShot;
                self.decoders[index].connect_camera(config, generation);
            }
        }

        let baseline = self.session_state_snapshot();
        if let Err(error) = self.performance_runtime.start_project_baseline(
            baseline,
            &self.project_id,
            self.show_time_at(Instant::now()),
        ) {
            log::error!("start project-linked take: {error:#}");
        }

        self.last_saved_project = (!recovered).then_some(project_file);
        self.performance_started = Instant::now();
        self.last_autosave = Instant::now();
    }

    pub(crate) fn poll_restores(&mut self) {
        while let Ok(result) = self.restorer.try_recv() {
            if result.project_epoch != self.project_epoch {
                continue;
            }
            let folder_result = self.folder_pending.remove(&result.address);
            if self.clips.path(result.address) != Some(result.path.as_path()) {
                if folder_result && self.folder_pending.is_empty() {
                    self.folder_status = "Folder import complete".to_owned();
                }
                continue;
            }
            let relink_result = self.relink_pending.remove(&result.address);
            let relink_active = self.relink_active.remove(&result.address);
            match result.metadata {
                Ok(movie) => {
                    let address = result.address;
                    let duration = movie.duration.map(MediaTime::as_seconds);
                    if folder_result || relink_result {
                        self.record_show_operation(
                            CommandOrigin::Operator,
                            Instant::now(),
                            CommandOperation::SetParameter {
                                path: format!(
                                    "deck.{}.clip.{}.media",
                                    address.deck.index(),
                                    address.slot
                                ),
                                value: oneiroi_graph::ParameterValue::Text(
                                    result.path.to_string_lossy().into_owned(),
                                ),
                            },
                        );
                    }
                    self.clips.restore(address, movie);
                    self.request_thumbnail(address, result.path.clone());
                    if relink_active
                        || self.restore_active[address.deck.index()] == Some(address.slot)
                    {
                        let desired = self.restore_transport[address.deck.index()].take();
                        self.launch_clip(address);
                        self.clips.select(ClipAddress {
                            deck: address.deck,
                            slot: self.restore_selected[address.deck.index()],
                        });
                        if let Some(mut transport) = desired {
                            transport.duration = duration;
                            self.transports[address.deck.index()] = transport;
                            if transport.position > 0.0 {
                                self.seek_deck(address.deck);
                            }
                        }
                    }
                    if relink_result {
                        self.project_status = format!(
                            "Relinked Deck {} slot {}",
                            address.deck.label(),
                            address.slot + 1
                        );
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    self.clips
                        .fail_restore(result.address, result.path, message.clone());
                    if relink_result {
                        self.project_status = format!("Relink failed: {message}");
                    }
                }
            }
            if folder_result && self.folder_pending.is_empty() {
                self.folder_status = "Folder import complete".to_owned();
            } else if folder_result {
                self.folder_status = format!(
                    "Folder import · {} file(s) remaining",
                    self.folder_pending.len()
                );
            }
        }
    }

    pub(crate) fn maybe_autosave(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_autosave) < Duration::from_secs(5) {
            return;
        }
        self.last_autosave = now;
        self.autosave_recovery();
    }

    pub(crate) fn autosave_recovery(&mut self) {
        if !self.project_dirty() {
            return;
        }
        let path = autosave_path(self.project_path.as_deref(), &self.workspace);
        match save_project_atomic(&path, &self.project_snapshot()) {
            Ok(()) => {
                self.recovery_path = Some(path);
                self.project_status = "Autosaved recovery snapshot.".to_owned();
            }
            Err(error) => self.project_status = format!("Autosave failed: {error}"),
        }
    }
}
