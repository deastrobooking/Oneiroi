//! Four-deck import state and background media probing.

use std::array;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{MovieMetadata, ProbeError, probe_movie};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeckId {
    A,
    B,
    C,
    D,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CrossfadeBus {
    Left,
    Right,
}

pub fn crossfade_gains(position: f32, equal_power: bool) -> [f32; 2] {
    let position = position.clamp(0.0, 1.0);
    if equal_power {
        let phase = position * std::f32::consts::FRAC_PI_2;
        [phase.cos(), phase.sin()]
    } else {
        [1.0 - position, position]
    }
}

impl DeckId {
    pub const ALL: [Self; 4] = [Self::A, Self::B, Self::C, Self::D];

    pub const fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::C => 2,
            Self::D => 3,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::C,
            Self::C => Self::D,
            Self::D => Self::A,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DeckState {
    Empty,
    Loading { path: PathBuf },
    Ready(MovieMetadata),
    Error { path: PathBuf, message: String },
}

#[derive(Clone, Debug)]
pub struct Deck {
    pub id: DeckId,
    pub generation: u64,
    pub level: f32,
    pub bus: CrossfadeBus,
    pub state: DeckState,
}

impl Deck {
    fn new(id: DeckId) -> Self {
        Self {
            id,
            generation: 0,
            level: 1.0,
            bus: if id.index().is_multiple_of(2) {
                CrossfadeBus::Left
            } else {
                CrossfadeBus::Right
            },
            state: DeckState::Empty,
        }
    }
}

pub struct FourDeckMixer {
    decks: [Deck; 4],
    selected: DeckId,
}

impl Default for FourDeckMixer {
    fn default() -> Self {
        Self {
            decks: array::from_fn(|index| Deck::new(DeckId::ALL[index])),
            selected: DeckId::A,
        }
    }
}

impl FourDeckMixer {
    pub fn selected(&self) -> DeckId {
        self.selected
    }

    pub fn select(&mut self, deck: DeckId) {
        self.selected = deck;
    }

    pub fn deck(&self, id: DeckId) -> &Deck {
        &self.decks[id.index()]
    }

    pub fn deck_mut(&mut self, id: DeckId) -> &mut Deck {
        &mut self.decks[id.index()]
    }

    pub fn begin_import(&mut self, id: DeckId, path: PathBuf) -> ImportRequest {
        let deck = self.deck_mut(id);
        deck.generation = deck.generation.wrapping_add(1);
        deck.state = DeckState::Loading { path: path.clone() };
        ImportRequest {
            deck: id,
            generation: deck.generation,
            path,
        }
    }

    pub fn complete_import(&mut self, result: ImportResult) -> bool {
        let deck = self.deck_mut(result.deck);
        if deck.generation != result.generation {
            return false;
        }
        deck.state = match result.metadata {
            Ok(metadata) => DeckState::Ready(metadata),
            Err(error) => DeckState::Error {
                path: result.path,
                message: error.to_string(),
            },
        };
        true
    }

    pub fn eject(&mut self, id: DeckId) {
        let deck = self.deck_mut(id);
        deck.generation = deck.generation.wrapping_add(1);
        deck.state = DeckState::Empty;
    }
}

#[derive(Debug)]
pub struct ImportRequest {
    pub deck: DeckId,
    pub generation: u64,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct ImportResult {
    pub deck: DeckId,
    pub generation: u64,
    pub path: PathBuf,
    pub metadata: Result<MovieMetadata, ProbeError>,
}

enum WorkerCommand {
    Probe(ImportRequest),
    Shutdown,
}

pub struct MediaImporter {
    commands: SyncSender<WorkerCommand>,
    results: Receiver<ImportResult>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub enum SubmitError {
    Busy(ImportRequest),
    Disconnected(ImportRequest),
}

impl MediaImporter {
    pub fn new(queue_capacity: usize) -> Self {
        let (commands_tx, commands_rx) = mpsc::sync_channel(queue_capacity.max(1));
        let (results_tx, results_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("oneiroi-media-import".to_owned())
            .spawn(move || {
                while let Ok(command) = commands_rx.recv() {
                    match command {
                        WorkerCommand::Probe(request) => {
                            let metadata = probe_movie(&request.path);
                            if results_tx
                                .send(ImportResult {
                                    deck: request.deck,
                                    generation: request.generation,
                                    path: request.path,
                                    metadata,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        WorkerCommand::Shutdown => break,
                    }
                }
            })
            .expect("spawn media import worker");
        Self {
            commands: commands_tx,
            results: results_rx,
            worker: Some(worker),
        }
    }

    pub fn submit(&self, request: ImportRequest) -> Result<(), SubmitError> {
        match self.commands.try_send(WorkerCommand::Probe(request)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(WorkerCommand::Probe(request))) => {
                Err(SubmitError::Busy(request))
            }
            Err(TrySendError::Disconnected(WorkerCommand::Probe(request))) => {
                Err(SubmitError::Disconnected(request))
            }
            Err(_) => unreachable!("only probe commands are submitted"),
        }
    }

    pub fn try_recv(&self) -> Result<ImportResult, TryRecvError> {
        self.results.try_recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<ImportResult, RecvTimeoutError> {
        self.results.recv_timeout(timeout)
    }
}

impl Drop for MediaImporter {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixer_has_exactly_four_named_decks() {
        let mixer = FourDeckMixer::default();
        assert_eq!(DeckId::ALL.map(|id| mixer.deck(id).id), DeckId::ALL);
    }

    #[test]
    fn crossfade_curves_reach_clean_endpoints() {
        assert_eq!(crossfade_gains(0.0, false), [1.0, 0.0]);
        assert_eq!(crossfade_gains(1.0, false), [0.0, 1.0]);
        let center = crossfade_gains(0.5, true);
        assert!((center[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((center[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn late_import_result_cannot_replace_newer_assignment() {
        let mut mixer = FourDeckMixer::default();
        let old = mixer.begin_import(DeckId::A, PathBuf::from("old.mov"));
        let new = mixer.begin_import(DeckId::A, PathBuf::from("new.mov"));

        let accepted = mixer.complete_import(ImportResult {
            deck: old.deck,
            generation: old.generation,
            path: old.path,
            metadata: Err(ProbeError::NoVideoStream),
        });

        assert!(!accepted);
        assert_eq!(mixer.deck(DeckId::A).generation, new.generation);
        assert!(matches!(
            &mixer.deck(DeckId::A).state,
            DeckState::Loading { path } if path == &new.path
        ));
    }

    #[test]
    fn eject_invalidates_in_flight_import() {
        let mut mixer = FourDeckMixer::default();
        let request = mixer.begin_import(DeckId::C, PathBuf::from("clip.mp4"));
        mixer.eject(DeckId::C);

        assert_ne!(mixer.deck(DeckId::C).generation, request.generation);
        assert!(matches!(mixer.deck(DeckId::C).state, DeckState::Empty));
    }
}
