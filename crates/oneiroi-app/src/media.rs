//! Media import, clip launching, thumbnails and camera connection.

use std::path::PathBuf;
use std::time::Instant;

use oneiroi_core::MediaTime;
use oneiroi_media::{
    CLIPS_PER_DECK, CameraConfig, CameraDevice, ClipAddress, ClipRestoreRequest, DeckId, DeckState,
    FolderScanRequest, SubmitError, ThumbnailRequest, VideoFramePayload, discover_cameras,
};
use oneiroi_session::{CommandOperation, CommandOrigin};

use super::{State, display_path, media_time_from_seconds};

impl State {
    pub(crate) fn import_path(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.import_folder(path);
        } else {
            self.import_movie(path);
        }
    }

    pub(crate) fn browse_relink(&mut self, address: ClipAddress) {
        let current = self.clips.path(address).map(PathBuf::from);
        let mut dialog = rfd::FileDialog::new().add_filter(
            "Video and still media",
            &[
                "mov", "mp4", "m4v", "mkv", "avi", "webm", "mxf", "png", "jpg", "jpeg",
            ],
        );
        if let Some(parent) = current.as_deref().and_then(std::path::Path::parent)
            && parent.exists()
        {
            dialog = dialog.set_directory(parent);
        }
        if let Some(name) = current
            .as_deref()
            .and_then(std::path::Path::file_name)
            .and_then(|name| name.to_str())
        {
            dialog = dialog.set_file_name(name);
        }
        if let Some(path) = dialog.pick_file() {
            self.relink_slot(address, path);
        } else {
            self.project_status = "Relink cancelled".to_owned();
        }
    }

    pub(crate) fn relink_slot(&mut self, address: ClipAddress, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        if path.is_dir() {
            self.project_status = "Relink requires a media file, not a folder".to_owned();
            return;
        }
        if self.clips.active(address.deck) == Some(address.slot) {
            self.relink_active.insert(address);
        }
        self.relink_pending.insert(address);
        self.ui.clear_thumbnail(address);
        self.thumbnail_requests.remove(&address);
        self.clips.begin_relink(address, path.clone());
        match self.restorer.submit(ClipRestoreRequest {
            address,
            path: path.clone(),
            project_epoch: self.project_epoch,
        }) {
            Ok(()) => {
                self.project_status = format!(
                    "Relinking Deck {} slot {} to {}…",
                    address.deck.label(),
                    address.slot + 1,
                    display_path(&path)
                );
            }
            Err(request) => {
                self.relink_pending.remove(&address);
                self.relink_active.remove(&address);
                self.clips.fail_restore(
                    request.address,
                    request.path,
                    "Relink probe queue is full.".to_owned(),
                );
                self.project_status = "Relink queue is full".to_owned();
            }
        }
    }

    /// Work that would land results at a now-wrong address if the slot moved
    /// under it: folder import, relink probe, or a deck import targeting the
    /// slot.
    fn clip_move_blocked(&self, address: ClipAddress) -> bool {
        self.folder_pending.contains(&address)
            || self.relink_pending.contains(&address)
            || self.import_slots[address.deck.index()].is_some_and(|(_, slot)| slot == address.slot)
    }

    /// Remove one bank slot through the same cleanup path for UI buttons,
    /// context menus and keyboard shortcuts.
    pub(crate) fn clear_clip(&mut self, address: ClipAddress, now: Instant, origin: CommandOrigin) {
        let occupied = self.clips.slot(address).is_some_and(|slot| {
            slot.movie.is_some() || slot.pending_path.is_some() || slot.error.is_some()
        });
        if !occupied {
            return;
        }

        self.record_show_operation(
            origin,
            now,
            CommandOperation::ClearClip {
                deck: address.deck.index() as u8,
                slot: address.slot as u8,
            },
        );
        if self.clips.active(address.deck) == Some(address.slot) {
            self.master_effect_processor.reset_history();
        }
        if self.import_slots[address.deck.index()].is_some_and(|(_, slot)| slot == address.slot) {
            self.mixer.eject(address.deck);
            self.import_slots[address.deck.index()] = None;
            let generation = self.mixer.deck(address.deck).generation;
            self.reset_playback(address.deck, generation);
        }
        if self.launches.queued(address) {
            self.launches.cancel(address.deck);
        }
        self.clips.clear(address);
        self.folder_pending.remove(&address);
        self.relink_pending.remove(&address);
        self.relink_active.remove(&address);
        self.ui.clear_thumbnail(address);
        self.thumbnail_requests.remove(&address);
        self.project_status = format!(
            "Deleted Deck {} clip {}",
            address.deck.label(),
            address.slot + 1
        );
    }

    pub(crate) fn move_clip(&mut self, from: ClipAddress, to: ClipAddress, now: Instant) {
        if self.clip_move_blocked(from) || self.clip_move_blocked(to) {
            self.project_status = "Cannot move a clip while its slot is importing".to_owned();
            return;
        }
        let swapped = self.clips.slot(to).is_some_and(|slot| {
            slot.movie.is_some() || slot.pending_path.is_some() || slot.error.is_some()
        });
        if !self.clips.move_clip(from, to) {
            self.project_status = "Clip move refused (empty or still restoring)".to_owned();
            return;
        }

        // Queued launches reference addresses whose content just changed;
        // firing them now would launch the wrong clip on a quantized boundary.
        if self.launches.queued(from) {
            self.launches.cancel(from.deck);
        }
        if self.launches.queued(to) {
            self.launches.cancel(to.deck);
        }

        // Cached previews follow the clips; in-flight thumbnail requests are
        // dropped because their results would land at the old address.
        self.ui.swap_thumbnails(from, to);
        self.thumbnail_requests.remove(&from);
        self.thumbnail_requests.remove(&to);

        self.record_show_operation(
            CommandOrigin::Operator,
            now,
            CommandOperation::MoveClip {
                from_deck: from.deck.index() as u8,
                from_slot: from.slot as u8,
                to_deck: to.deck.index() as u8,
                to_slot: to.slot as u8,
            },
        );
        self.project_status = format!(
            "{} clip {}{} {} {}{}",
            if swapped { "Swapped" } else { "Moved" },
            from.deck.label(),
            from.slot + 1,
            if swapped { "↔" } else { "→" },
            to.deck.label(),
            to.slot + 1,
        );
    }

    pub(crate) fn import_folder(&mut self, path: PathBuf) {
        let start = ClipAddress {
            deck: self.mixer.selected(),
            slot: self.clips.selected(self.mixer.selected()),
        };
        let available = self.clips.available_slots_from(start, CLIPS_PER_DECK * 4);
        if available.is_empty() {
            self.folder_status = "Folder import skipped · all 32 slots are occupied".to_owned();
            return;
        }
        let request_id = self.folder_request_id.wrapping_add(1);
        let request = FolderScanRequest {
            root: path.clone(),
            request_id,
            project_epoch: self.project_epoch,
            max_files: available.len(),
        };
        match self.folder_scanner.submit(request) {
            Ok(()) => {
                self.folder_request_id = request_id;
                self.folder_scan_start = start;
                self.folder_status = format!("Scanning {}…", display_path(&path));
            }
            Err(_) => {
                self.folder_status = "Folder scan is busy · wait for the current folder".to_owned();
            }
        }
    }

    pub(crate) fn poll_folder_scans(&mut self) {
        while let Ok(result) = self.folder_scanner.try_recv() {
            if result.project_epoch != self.project_epoch
                || result.request_id != self.folder_request_id
            {
                continue;
            }
            let paths = match result.paths {
                Ok(paths) => paths,
                Err(error) => {
                    self.folder_status = format!("Folder scan failed: {error}");
                    continue;
                }
            };
            let slots = self
                .clips
                .available_slots_from(self.folder_scan_start, paths.len());
            let mut submitted = 0;
            for (address, path) in slots.into_iter().zip(paths) {
                self.clips.begin_restore(address, path.clone());
                match self.restorer.submit(ClipRestoreRequest {
                    address,
                    path,
                    project_epoch: self.project_epoch,
                }) {
                    Ok(()) => {
                        self.folder_pending.insert(address);
                        submitted += 1;
                    }
                    Err(request) => {
                        self.clips.fail_restore(
                            request.address,
                            request.path,
                            "Folder probe queue is full.".to_owned(),
                        );
                    }
                }
            }
            self.folder_status = if submitted == 0 {
                format!("No supported media found in {}", display_path(&result.root))
            } else {
                format!(
                    "Importing {submitted} file(s) from {}{}",
                    display_path(&result.root),
                    if result.truncated {
                        " · limited by available slots"
                    } else {
                        ""
                    }
                )
            };
        }
    }

    pub(crate) fn import_movie(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        let deck = self.mixer.selected();
        self.live_configs[deck.index()] = None;
        let address = ClipAddress {
            deck,
            slot: self.clips.selected(deck),
        };
        self.ui.clear_thumbnail(address);
        self.thumbnail_requests.remove(&address);
        let request = self.mixer.begin_import(deck, path);
        self.import_slots[deck.index()] = Some((request.generation, self.clips.selected(deck)));
        self.reset_playback(deck, request.generation);
        match self.importer.submit(request) {
            Ok(()) => {
                self.mixer.select(deck.next());
                self.window.request_redraw();
            }
            Err(SubmitError::Busy(request)) | Err(SubmitError::Disconnected(request)) => {
                let target = self.mixer.deck_mut(request.deck);
                if target.generation == request.generation {
                    target.state = DeckState::Error {
                        path: request.path,
                        message: "The media import worker is unavailable.".to_owned(),
                    };
                }
            }
        }
    }

    pub(crate) fn poll_imports(&mut self) {
        while let Ok(result) = self.importer.try_recv() {
            let playback = result.metadata.as_ref().ok().map(|movie| {
                (
                    result.deck,
                    result.generation,
                    movie.path.clone(),
                    movie.decode_path,
                    movie.clone(),
                )
            });
            if self.mixer.complete_import(result)
                && let Some((deck, generation, path, decode_path, movie)) = playback
            {
                if let Some((slot_generation, slot)) = self.import_slots[deck.index()]
                    && slot_generation == generation
                {
                    let address = ClipAddress { deck, slot };
                    self.record_show_operation(
                        CommandOrigin::Operator,
                        Instant::now(),
                        CommandOperation::SetParameter {
                            path: format!("deck.{}.clip.{slot}.media", deck.index()),
                            value: oneiroi_graph::ParameterValue::Text(
                                path.to_string_lossy().into_owned(),
                            ),
                        },
                    );
                    self.clips.assign(address, movie);
                    self.clips.activate(address);
                    self.request_thumbnail(address, path.clone());
                }
                self.reset_playback(deck, generation);
                self.decoders[deck.index()].load(path, decode_path, generation);
            }
        }
    }

    pub(crate) fn launch_clip(&mut self, address: ClipAddress) {
        let Some(movie) = self.clips.movie(address).cloned() else {
            return;
        };
        self.master_effect_processor.reset_history();
        self.clips
            .remember_position(address.deck, self.transports[address.deck.index()].position);
        let media_duration = movie.duration.map(MediaTime::as_seconds);
        let playback = self.clips.playback(address).unwrap_or_default();
        let launch_position = self
            .clips
            .launch_position(address, media_duration, self.ui.bpm)
            .unwrap_or(playback.in_point);
        let (in_point, out_point) = playback.range(media_duration, self.ui.bpm);
        let path = movie.path.clone();
        let preload = self
            .ui
            .preloaded_frame(address, Some(path.as_path()))
            .cloned();
        let decode_path = movie.decode_path;
        let start_at = media_time_from_seconds(launch_position);
        let seek_to = if decode_path == oneiroi_media::DecodePath::FfmpegVideo {
            start_at.and_then(|target| movie.keyframes.nearest_preceding(target))
        } else {
            None
        };
        let generation = self.mixer.activate(address.deck, movie);
        self.live_configs[address.deck.index()] = None;
        self.clips.activate(address);
        self.reset_playback(address.deck, generation);
        self.transports[address.deck.index()].reset_range(in_point, out_point);
        self.transports[address.deck.index()].position = launch_position;
        if let Some(preload) = preload
            && let Err(error) = self.compositor.upload(
                &self.gpu.device,
                &self.gpu.queue,
                address.deck.index(),
                &VideoFramePayload::Rgba8(preload),
            )
        {
            log::error!(
                "deck {} first-frame preload upload failed: {error}",
                address.deck.label()
            );
        }
        self.decoders[address.deck.index()].load_indexed(
            path,
            decode_path,
            generation,
            start_at,
            seek_to,
        );
    }

    pub(crate) fn queue_clip(&mut self, address: ClipAddress, now: Instant) {
        if self.clips.movie(address).is_none() {
            return;
        }
        let elapsed = now
            .saturating_duration_since(self.performance_started)
            .as_secs_f64();
        self.launches
            .queue(address, self.ui.quantization, self.tempo, elapsed);
    }

    pub(crate) fn process_launches(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.performance_started)
            .as_secs_f64();
        if (self.tempo.bpm() - self.ui.bpm).abs() > f64::EPSILON {
            self.tempo.set_bpm(self.ui.bpm, elapsed);
        }
        for address in self.launches.take_due(self.tempo, elapsed) {
            self.launch_clip(address);
        }
    }

    pub(crate) fn refresh_cameras(&mut self) {
        match discover_cameras() {
            Ok(cameras) => {
                let count = cameras.len();
                self.cameras = cameras;
                self.camera_status = if count == 0 {
                    "No cameras discovered; check macOS camera permission or enter a device ID."
                        .to_owned()
                } else {
                    format!("{count} camera(s) available")
                };
            }
            Err(error) => self.camera_status = format!("Camera discovery failed: {error}"),
        }
    }

    pub(crate) fn connect_camera(
        &mut self,
        deck: DeckId,
        device_id: String,
        label: String,
        extent: [u32; 2],
        fps: u32,
    ) {
        self.master_effect_processor.reset_history();
        self.clips
            .remember_position(deck, self.transports[deck.index()].position);
        let config = CameraConfig {
            device: CameraDevice {
                id: device_id,
                label,
                backend: "avfoundation".to_owned(),
            },
            requested_extent: Some(extent),
            requested_fps: Some(fps),
        };
        self.launches.cancel(deck);
        self.clips.deactivate(deck);
        let generation = self.mixer.connect_camera(deck, config.clone());
        self.live_configs[deck.index()] = Some(config.clone());
        self.reset_playback(deck, generation);
        self.transports[deck.index()].end_mode = oneiroi_media::EndMode::OneShot;
        self.decoders[deck.index()].connect_camera(config, generation);
        self.camera_status = format!("Connecting Deck {}…", deck.label());
    }

    pub(crate) fn request_thumbnail(&mut self, address: ClipAddress, path: PathBuf) {
        self.thumbnail_request_id = self.thumbnail_request_id.wrapping_add(1);
        let request_id = self.thumbnail_request_id;
        self.thumbnail_requests
            .insert(address, (request_id, path.clone()));
        if self
            .thumbnails
            .submit(ThumbnailRequest {
                address,
                path,
                request_id,
            })
            .is_err()
        {
            self.thumbnail_requests.remove(&address);
        }
    }

    pub(crate) fn poll_thumbnails(&mut self) {
        let context = self.egui_state.egui_ctx().clone();
        while let Ok(result) = self.thumbnails.try_recv() {
            let current = self.thumbnail_requests.get(&result.address);
            if !current.is_some_and(|(request_id, path)| {
                *request_id == result.request_id && *path == result.path
            }) || self.clips.path(result.address) != Some(result.path.as_path())
            {
                continue;
            }
            self.thumbnail_requests.remove(&result.address);
            match result.thumbnail {
                Ok(thumbnail) => {
                    self.ui
                        .install_thumbnail(&context, result.address, result.path, thumbnail);
                }
                Err(message) => {
                    self.ui
                        .mark_thumbnail_failed(result.address, result.path, message);
                }
            }
        }
    }
}
