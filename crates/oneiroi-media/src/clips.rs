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

#[derive(Clone, Debug, Default)]
pub struct ClipSlot {
    pub movie: Option<MovieMetadata>,
    pub pending_path: Option<PathBuf>,
    pub error: Option<String>,
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
}
