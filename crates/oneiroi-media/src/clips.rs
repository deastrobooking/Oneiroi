//! Four-deck clip bank and generation-independent launch queue.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use oneiroi_core::{Quantization, TempoClock};

use crate::{DeckId, MovieMetadata, ProbeError, probe_movie};

pub const CLIPS_PER_DECK: usize = 8;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClipAddress {
    pub deck: DeckId,
    pub slot: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ClipLaunchMode {
    #[default]
    Restart,
    Resume,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlayback {
    pub in_point: f64,
    pub out_point: Option<f64>,
    pub launch_mode: ClipLaunchMode,
    /// Optional musical length in beats, measured from the in point.
    pub beat_duration: Option<f64>,
}

impl Default for ClipPlayback {
    fn default() -> Self {
        Self {
            in_point: 0.0,
            out_point: None,
            launch_mode: ClipLaunchMode::Restart,
            beat_duration: None,
        }
    }
}

impl ClipPlayback {
    pub fn sanitized(self, media_duration: Option<f64>) -> Self {
        let maximum = media_duration
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .unwrap_or(f64::MAX);
        let maximum_in = if maximum == f64::MAX {
            maximum
        } else {
            (maximum - 0.000_001).max(0.0)
        };
        let in_point = if self.in_point.is_finite() {
            self.in_point.clamp(0.0, maximum_in)
        } else {
            0.0
        };
        let out_point = self.out_point.filter(|out| out.is_finite()).map(|out| {
            out.clamp(
                (in_point + 0.000_001).min(maximum),
                maximum.max(in_point + 0.000_001),
            )
        });
        let beat_duration = self
            .beat_duration
            .filter(|beats| beats.is_finite() && *beats > 0.0)
            .map(|beats| beats.clamp(0.0625, 256.0));
        Self {
            in_point,
            out_point,
            launch_mode: self.launch_mode,
            beat_duration,
        }
    }

    pub fn range(self, media_duration: Option<f64>, bpm: f64) -> (f64, Option<f64>) {
        let playback = self.sanitized(media_duration);
        let musical_out = playback
            .beat_duration
            .filter(|_| bpm.is_finite() && bpm > 0.0)
            .map(|beats| playback.in_point + beats * 60.0 / bpm);
        let end = [playback.out_point, musical_out, media_duration]
            .into_iter()
            .flatten()
            .filter(|end| end.is_finite() && *end > playback.in_point)
            .reduce(f64::min);
        (playback.in_point, end)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClipSlot {
    pub movie: Option<MovieMetadata>,
    pub pending_path: Option<PathBuf>,
    pub error: Option<String>,
    pub playback: ClipPlayback,
    resume_position: f64,
}

pub struct ClipBank {
    slots: [[ClipSlot; CLIPS_PER_DECK]; 4],
    selected: [usize; 4],
    active: [Option<usize>; 4],
}

impl Default for ClipBank {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| std::array::from_fn(|_| ClipSlot::default())),
            selected: [0; 4],
            active: [None; 4],
        }
    }
}

impl ClipBank {
    pub fn selected(&self, deck: DeckId) -> usize {
        self.selected[deck.index()]
    }

    pub fn active(&self, deck: DeckId) -> Option<usize> {
        self.active[deck.index()]
    }

    pub fn activate(&mut self, address: ClipAddress) {
        if self.movie(address).is_some() {
            self.active[address.deck.index()] = Some(address.slot);
            self.select(address);
        }
    }

    pub fn deactivate(&mut self, deck: DeckId) {
        self.active[deck.index()] = None;
    }

    pub fn restore_active(&mut self, deck: DeckId, slot: Option<usize>) {
        self.active[deck.index()] = slot.filter(|slot| *slot < CLIPS_PER_DECK);
    }

    pub fn select(&mut self, address: ClipAddress) {
        if address.slot < CLIPS_PER_DECK {
            self.selected[address.deck.index()] = address.slot;
        }
    }

    pub fn slot(&self, address: ClipAddress) -> Option<&ClipSlot> {
        self.slots
            .get(address.deck.index())
            .and_then(|row| row.get(address.slot))
    }

    pub fn assign(&mut self, address: ClipAddress, movie: MovieMetadata) -> bool {
        if let Some(slot) = self
            .slots
            .get_mut(address.deck.index())
            .and_then(|row| row.get_mut(address.slot))
        {
            slot.playback = ClipPlayback::default();
            slot.resume_position = 0.0;
        }
        if !self.restore(address, movie) {
            return false;
        }
        self.select(address);
        true
    }

    pub fn restore(&mut self, address: ClipAddress, movie: MovieMetadata) -> bool {
        let Some(slot) = self
            .slots
            .get_mut(address.deck.index())
            .and_then(|row| row.get_mut(address.slot))
        else {
            return false;
        };
        slot.movie = Some(movie);
        slot.pending_path = None;
        slot.error = None;
        true
    }

    pub fn movie(&self, address: ClipAddress) -> Option<&MovieMetadata> {
        self.slot(address)?.movie.as_ref()
    }

    pub fn path(&self, address: ClipAddress) -> Option<&Path> {
        let slot = self.slot(address)?;
        slot.movie
            .as_ref()
            .map(|movie| movie.path.as_path())
            .or(slot.pending_path.as_deref())
    }

    pub fn available_slots_from(&self, start: ClipAddress, limit: usize) -> Vec<ClipAddress> {
        let start_index = start.deck.index() * CLIPS_PER_DECK + start.slot.min(CLIPS_PER_DECK - 1);
        (0..4 * CLIPS_PER_DECK)
            .map(|offset| (start_index + offset) % (4 * CLIPS_PER_DECK))
            .map(|index| ClipAddress {
                deck: DeckId::ALL[index / CLIPS_PER_DECK],
                slot: index % CLIPS_PER_DECK,
            })
            .filter(|address| self.path(*address).is_none())
            .take(limit)
            .collect()
    }

    pub fn playback(&self, address: ClipAddress) -> Option<ClipPlayback> {
        self.slot(address).map(|slot| slot.playback)
    }

    pub fn set_playback(&mut self, address: ClipAddress, playback: ClipPlayback) -> bool {
        let Some(slot) = self
            .slots
            .get_mut(address.deck.index())
            .and_then(|row| row.get_mut(address.slot))
        else {
            return false;
        };
        let duration = slot
            .movie
            .as_ref()
            .and_then(|movie| movie.duration)
            .map(oneiroi_core::MediaTime::as_seconds);
        slot.playback = playback.sanitized(duration);
        true
    }

    pub fn remember_position(&mut self, deck: DeckId, position: f64) {
        let Some(slot_index) = self.active[deck.index()] else {
            return;
        };
        let Some(slot) = self.slots[deck.index()].get_mut(slot_index) else {
            return;
        };
        if position.is_finite() {
            slot.resume_position = position.max(0.0);
        }
    }

    pub fn launch_position(
        &self,
        address: ClipAddress,
        media_duration: Option<f64>,
        bpm: f64,
    ) -> Option<f64> {
        let slot = self.slot(address)?;
        let (start, end) = slot.playback.range(media_duration, bpm);
        let position = match slot.playback.launch_mode {
            ClipLaunchMode::Restart => start,
            ClipLaunchMode::Resume => slot.resume_position,
        };
        Some(match end {
            Some(end) if position >= end => start,
            Some(end) => position.clamp(start, end),
            None => position.max(start),
        })
    }

    pub fn begin_restore(&mut self, address: ClipAddress, path: PathBuf) -> bool {
        let Some(slot) = self
            .slots
            .get_mut(address.deck.index())
            .and_then(|row| row.get_mut(address.slot))
        else {
            return false;
        };
        slot.movie = None;
        slot.pending_path = Some(path);
        slot.error = None;
        slot.playback = ClipPlayback::default();
        slot.resume_position = 0.0;
        true
    }

    pub fn begin_relink(&mut self, address: ClipAddress, path: PathBuf) -> bool {
        let Some(slot) = self
            .slots
            .get_mut(address.deck.index())
            .and_then(|row| row.get_mut(address.slot))
        else {
            return false;
        };
        slot.movie = None;
        slot.pending_path = Some(path);
        slot.error = None;
        true
    }

    pub fn fail_restore(&mut self, address: ClipAddress, path: PathBuf, message: String) -> bool {
        let Some(slot) = self
            .slots
            .get_mut(address.deck.index())
            .and_then(|row| row.get_mut(address.slot))
        else {
            return false;
        };
        slot.movie = None;
        slot.pending_path = Some(path);
        slot.error = Some(message);
        true
    }

    /// True when the slot is mid-restore or mid-relink: it has a path on the
    /// way in but neither media nor a failure yet. Moving such a slot would
    /// let the in-flight probe result land at a stale address.
    fn slot_in_flight(&self, address: ClipAddress) -> bool {
        self.slot(address).is_some_and(|slot| {
            slot.movie.is_none() && slot.pending_path.is_some() && slot.error.is_none()
        })
    }

    /// Move a clip to another slot, on any deck. An occupied destination
    /// swaps rather than being overwritten, so no drop can destroy media.
    ///
    /// Selection and active markers follow the content within a deck. Across
    /// decks an active marker is cleared instead of followed: active means
    /// "this deck is playing this slot's media", and after a cross-deck move
    /// that statement is no longer true of either endpoint - the deck keeps
    /// playing what its decoder holds, it just stops claiming a slot.
    pub fn move_clip(&mut self, from: ClipAddress, to: ClipAddress) -> bool {
        if from == to
            || from.slot >= CLIPS_PER_DECK
            || to.slot >= CLIPS_PER_DECK
            || self.slot_in_flight(from)
            || self.slot_in_flight(to)
        {
            return false;
        }
        let source_occupied = self.slot(from).is_some_and(|slot| {
            slot.movie.is_some() || slot.pending_path.is_some() || slot.error.is_some()
        });
        if !source_occupied {
            return false;
        }

        if from.deck == to.deck {
            let deck = from.deck.index();
            self.slots[deck].swap(from.slot, to.slot);
            for marker in [&mut self.selected[deck]] {
                if *marker == from.slot {
                    *marker = to.slot;
                } else if *marker == to.slot {
                    *marker = from.slot;
                }
            }
            if self.active[deck] == Some(from.slot) {
                self.active[deck] = Some(to.slot);
            } else if self.active[deck] == Some(to.slot) {
                self.active[deck] = Some(from.slot);
            }
        } else {
            let (low, high) = if from.deck.index() < to.deck.index() {
                (from.deck.index(), to.deck.index())
            } else {
                (to.deck.index(), from.deck.index())
            };
            let (first, second) = self.slots.split_at_mut(high);
            let (from_row, to_row) = if from.deck.index() < to.deck.index() {
                (&mut first[low], &mut second[0])
            } else {
                (&mut second[0], &mut first[low])
            };
            std::mem::swap(&mut from_row[from.slot], &mut to_row[to.slot]);
            if self.active[from.deck.index()] == Some(from.slot) {
                self.active[from.deck.index()] = None;
            }
            if self.active[to.deck.index()] == Some(to.slot) {
                self.active[to.deck.index()] = None;
            }
        }
        true
    }

    pub fn clear(&mut self, address: ClipAddress) -> bool {
        let Some(slot) = self
            .slots
            .get_mut(address.deck.index())
            .and_then(|row| row.get_mut(address.slot))
        else {
            return false;
        };
        slot.movie = None;
        slot.pending_path = None;
        slot.error = None;
        if self.active[address.deck.index()] == Some(address.slot) {
            self.active[address.deck.index()] = None;
        }
        true
    }
}

#[derive(Debug)]
pub struct ClipRestoreRequest {
    pub address: ClipAddress,
    pub path: PathBuf,
    pub project_epoch: u64,
}

#[derive(Debug)]
pub struct ClipRestoreResult {
    pub address: ClipAddress,
    pub path: PathBuf,
    pub project_epoch: u64,
    pub metadata: Result<MovieMetadata, ProbeError>,
}

enum RestoreCommand {
    Probe(ClipRestoreRequest),
    Shutdown,
}

pub struct ClipRestorer {
    commands: SyncSender<RestoreCommand>,
    results: Receiver<ClipRestoreResult>,
    worker: Option<JoinHandle<()>>,
}

impl ClipRestorer {
    pub fn new(queue_capacity: usize) -> Self {
        let (commands_tx, commands_rx) = mpsc::sync_channel(queue_capacity.max(1));
        let (results_tx, results_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("oneiroi-project-restore".to_owned())
            .spawn(move || {
                while let Ok(command) = commands_rx.recv() {
                    match command {
                        RestoreCommand::Probe(request) => {
                            let metadata = probe_movie(&request.path);
                            if results_tx
                                .send(ClipRestoreResult {
                                    address: request.address,
                                    path: request.path,
                                    project_epoch: request.project_epoch,
                                    metadata,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        RestoreCommand::Shutdown => break,
                    }
                }
            })
            .expect("spawn project restore worker");
        Self {
            commands: commands_tx,
            results: results_rx,
            worker: Some(worker),
        }
    }

    pub fn submit(&self, request: ClipRestoreRequest) -> Result<(), ClipRestoreRequest> {
        match self.commands.try_send(RestoreCommand::Probe(request)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(RestoreCommand::Probe(request)))
            | Err(TrySendError::Disconnected(RestoreCommand::Probe(request))) => Err(request),
            Err(_) => unreachable!("only probe requests are submitted"),
        }
    }

    pub fn try_recv(&self) -> Result<ClipRestoreResult, TryRecvError> {
        self.results.try_recv()
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<ClipRestoreResult, mpsc::RecvTimeoutError> {
        self.results.recv_timeout(timeout)
    }
}

impl Drop for ClipRestorer {
    fn drop(&mut self) {
        let _ = self.commands.send(RestoreCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PendingLaunch {
    address: ClipAddress,
    launch_beat: f64,
}

#[derive(Default)]
pub struct LaunchQueue {
    pending: [Option<PendingLaunch>; 4],
}

impl LaunchQueue {
    pub fn queue(
        &mut self,
        address: ClipAddress,
        quantization: Quantization,
        clock: TempoClock,
        now_seconds: f64,
    ) {
        self.pending[address.deck.index()] = Some(PendingLaunch {
            address,
            launch_beat: clock.launch_beat(quantization, now_seconds),
        });
    }

    pub fn cancel(&mut self, deck: DeckId) {
        self.pending[deck.index()] = None;
    }

    pub fn queued(&self, address: ClipAddress) -> bool {
        self.pending[address.deck.index()].is_some_and(|pending| pending.address == address)
    }

    pub fn take_due(&mut self, clock: TempoClock, now_seconds: f64) -> Vec<ClipAddress> {
        let beat = clock.beat_at(now_seconds);
        let mut due = Vec::with_capacity(4);
        for pending in &mut self.pending {
            if pending.is_some_and(|launch| beat + 1e-9 >= launch.launch_beat)
                && let Some(launch) = pending.take()
            {
                due.push(launch.address);
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{AlphaMode, DecodePath, FrameRate, MediaHealth};

    fn movie(name: &str) -> MovieMetadata {
        MovieMetadata {
            path: PathBuf::from(name),
            display_name: name.to_owned(),
            container: "mov".to_owned(),
            stream_index: 0,
            codec: "h264".to_owned(),
            codec_tag: *b"avc1",
            visible_extent: [1920, 1080],
            frame_rate: Some(FrameRate {
                numerator: 30,
                denominator: 1,
            }),
            duration: None,
            frame_count: None,
            alpha: AlphaMode::Absent,
            decode_path: DecodePath::FfmpegVideo,
            health: MediaHealth::Caution,
            health_reason: String::new(),
            keyframes: crate::KeyframeIndex::default(),
        }
    }

    #[test]
    fn stores_eight_independent_slots_per_deck() {
        let mut bank = ClipBank::default();
        let address = ClipAddress {
            deck: DeckId::D,
            slot: 7,
        };
        assert!(bank.assign(address, movie("clip.mov")));
        assert_eq!(bank.movie(address).unwrap().display_name, "clip.mov");
        assert_eq!(bank.selected(DeckId::D), 7);
    }

    #[test]
    fn newer_queued_launch_replaces_older_launch_on_same_deck() {
        let clock = TempoClock::new(120.0, 4);
        let mut queue = LaunchQueue::default();
        queue.queue(
            ClipAddress {
                deck: DeckId::A,
                slot: 1,
            },
            Quantization::Bar,
            clock,
            0.1,
        );
        let newest = ClipAddress {
            deck: DeckId::A,
            slot: 2,
        };
        queue.queue(newest, Quantization::Beat, clock, 0.1);
        assert_eq!(queue.take_due(clock, 0.5), vec![newest]);
    }

    #[test]
    fn scene_launches_can_be_queued_for_all_four_decks() {
        let clock = TempoClock::new(120.0, 4);
        let mut queue = LaunchQueue::default();
        for deck in DeckId::ALL {
            queue.queue(
                ClipAddress { deck, slot: 3 },
                Quantization::Beat,
                clock,
                0.1,
            );
        }
        assert_eq!(queue.take_due(clock, 0.49).len(), 0);
        assert_eq!(queue.take_due(clock, 0.5).len(), 4);
    }

    #[test]
    fn failed_restore_retains_path_for_relink_and_resave() {
        let mut bank = ClipBank::default();
        let address = ClipAddress {
            deck: DeckId::B,
            slot: 4,
        };
        let path = PathBuf::from("/missing/clip.mov");
        bank.begin_restore(address, path.clone());
        bank.fail_restore(address, path.clone(), "not found".to_owned());
        assert_eq!(bank.path(address), Some(path.as_path()));
        assert_eq!(
            bank.slot(address).unwrap().error.as_deref(),
            Some("not found")
        );
    }

    #[test]
    fn clip_range_combines_trim_media_and_musical_duration() {
        let playback = ClipPlayback {
            in_point: 2.0,
            out_point: Some(12.0),
            beat_duration: Some(8.0),
            ..ClipPlayback::default()
        };
        assert_eq!(playback.range(Some(20.0), 120.0), (2.0, Some(6.0)));
        assert_eq!(playback.range(Some(5.0), 60.0), (2.0, Some(5.0)));
    }

    #[test]
    fn resume_launch_uses_the_remembered_position() {
        let mut bank = ClipBank::default();
        let address = ClipAddress {
            deck: DeckId::A,
            slot: 2,
        };
        bank.assign(address, movie("clip.mov"));
        bank.set_playback(
            address,
            ClipPlayback {
                in_point: 1.0,
                out_point: Some(8.0),
                launch_mode: ClipLaunchMode::Resume,
                beat_duration: None,
            },
        );
        bank.activate(address);
        bank.remember_position(DeckId::A, 4.5);
        assert_eq!(bank.launch_position(address, Some(10.0), 120.0), Some(4.5));
    }

    #[test]
    fn clearing_the_active_slot_removes_media_and_deactivates_the_deck() {
        let mut bank = ClipBank::default();
        let address = ClipAddress {
            deck: DeckId::C,
            slot: 5,
        };
        bank.assign(address, movie("active.mov"));
        bank.activate(address);

        assert!(bank.clear(address));

        assert!(bank.movie(address).is_none());
        assert!(bank.path(address).is_none());
        assert_eq!(bank.active(DeckId::C), None);
    }

    #[test]
    fn trim_is_clamped_inside_known_media_duration() {
        let playback = ClipPlayback {
            in_point: 99.0,
            out_point: Some(120.0),
            ..ClipPlayback::default()
        }
        .sanitized(Some(10.0));
        assert!(playback.in_point < 10.0);
        assert_eq!(playback.out_point, Some(10.0));
    }

    #[test]
    fn available_slots_wrap_from_selection_and_skip_occupied_slots() {
        let mut bank = ClipBank::default();
        bank.assign(
            ClipAddress {
                deck: DeckId::D,
                slot: 7,
            },
            movie("occupied.mov"),
        );
        let slots = bank.available_slots_from(
            ClipAddress {
                deck: DeckId::D,
                slot: 6,
            },
            3,
        );
        assert_eq!(
            slots,
            [
                ClipAddress {
                    deck: DeckId::D,
                    slot: 6
                },
                ClipAddress {
                    deck: DeckId::A,
                    slot: 0
                },
                ClipAddress {
                    deck: DeckId::A,
                    slot: 1
                }
            ]
        );
    }

    #[test]
    fn relink_preserves_clip_playback_settings() {
        let mut bank = ClipBank::default();
        let address = ClipAddress {
            deck: DeckId::B,
            slot: 3,
        };
        bank.assign(address, movie("old.mov"));
        let playback = ClipPlayback {
            in_point: 1.0,
            out_point: Some(7.0),
            launch_mode: ClipLaunchMode::Resume,
            beat_duration: Some(8.0),
        };
        bank.set_playback(address, playback);
        bank.begin_relink(address, PathBuf::from("new.mov"));
        assert_eq!(bank.path(address), Some(Path::new("new.mov")));
        assert_eq!(bank.playback(address), Some(playback));
        assert!(bank.movie(address).is_none());

        bank.begin_relink(address, PathBuf::from("newer.mov"));
        assert_ne!(bank.path(address), Some(Path::new("new.mov")));
        assert_eq!(bank.path(address), Some(Path::new("newer.mov")));
        assert_eq!(bank.playback(address), Some(playback));
    }

    #[test]
    fn move_to_empty_slot_carries_media_playback_and_resume_position() {
        let mut bank = ClipBank::default();
        let from = ClipAddress {
            deck: DeckId::A,
            slot: 2,
        };
        let to = ClipAddress {
            deck: DeckId::C,
            slot: 5,
        };
        bank.assign(from, movie("alpha.mov"));
        bank.set_playback(
            from,
            ClipPlayback {
                in_point: 1.5,
                out_point: Some(9.0),
                launch_mode: ClipLaunchMode::Resume,
                beat_duration: None,
            },
        );

        assert!(bank.move_clip(from, to));

        assert!(bank.movie(from).is_none());
        assert_eq!(
            bank.movie(to).map(|movie| movie.display_name.as_str()),
            Some("alpha.mov")
        );
        let playback = bank.playback(to).expect("playback follows the clip");
        assert_eq!(playback.in_point, 1.5);
        assert_eq!(playback.launch_mode, ClipLaunchMode::Resume);
        assert_eq!(bank.playback(from), Some(ClipPlayback::default()));
    }

    #[test]
    fn move_to_occupied_slot_swaps_instead_of_destroying() {
        let mut bank = ClipBank::default();
        let from = ClipAddress {
            deck: DeckId::A,
            slot: 0,
        };
        let to = ClipAddress {
            deck: DeckId::B,
            slot: 7,
        };
        bank.assign(from, movie("alpha.mov"));
        bank.assign(to, movie("beta.mov"));

        assert!(bank.move_clip(from, to));

        assert_eq!(
            bank.movie(from).map(|movie| movie.display_name.as_str()),
            Some("beta.mov")
        );
        assert_eq!(
            bank.movie(to).map(|movie| movie.display_name.as_str()),
            Some("alpha.mov")
        );
    }

    #[test]
    fn same_deck_move_remaps_active_and_selected_markers() {
        let mut bank = ClipBank::default();
        let from = ClipAddress {
            deck: DeckId::A,
            slot: 1,
        };
        let to = ClipAddress {
            deck: DeckId::A,
            slot: 4,
        };
        bank.assign(from, movie("alpha.mov"));
        bank.activate(from);
        assert_eq!(bank.active(DeckId::A), Some(1));
        assert_eq!(bank.selected(DeckId::A), 1);

        assert!(bank.move_clip(from, to));

        assert_eq!(bank.active(DeckId::A), Some(4));
        assert_eq!(bank.selected(DeckId::A), 4);
    }

    #[test]
    fn cross_deck_move_clears_active_markers_at_both_endpoints() {
        let mut bank = ClipBank::default();
        let from = ClipAddress {
            deck: DeckId::A,
            slot: 1,
        };
        let to = ClipAddress {
            deck: DeckId::B,
            slot: 3,
        };
        bank.assign(from, movie("alpha.mov"));
        bank.assign(to, movie("beta.mov"));
        bank.activate(from);
        bank.activate(to);

        assert!(bank.move_clip(from, to));

        // Both decks keep playing whatever their decoders hold; the markers
        // are cleared because the slots no longer contain that media.
        assert_eq!(bank.active(DeckId::A), None);
        assert_eq!(bank.active(DeckId::B), None);
    }

    #[test]
    fn refuses_moves_involving_in_flight_or_empty_slots() {
        let mut bank = ClipBank::default();
        let restoring = ClipAddress {
            deck: DeckId::A,
            slot: 0,
        };
        let empty = ClipAddress {
            deck: DeckId::A,
            slot: 1,
        };
        let occupied = ClipAddress {
            deck: DeckId::B,
            slot: 0,
        };
        bank.assign(occupied, movie("alpha.mov"));
        bank.begin_restore(restoring, PathBuf::from("gone.mov"));

        assert!(!bank.move_clip(restoring, empty), "mid-restore source");
        assert!(!bank.move_clip(occupied, restoring), "mid-restore target");
        assert!(!bank.move_clip(empty, occupied), "empty source");
        assert!(!bank.move_clip(occupied, occupied), "same address");

        // A missing-media placeholder (path plus error, no movie) is movable:
        // it is a real slot waiting for relink, not an in-flight probe.
        bank.fail_restore(restoring, PathBuf::from("gone.mov"), "missing".to_owned());
        assert!(bank.move_clip(restoring, empty));
        assert_eq!(
            bank.slot(empty).and_then(|slot| slot.error.as_deref()),
            Some("missing")
        );
    }
}
