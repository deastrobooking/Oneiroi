//! Event-sourced show commands, checkpoints, takes, and deterministic replay.

use std::collections::BTreeMap;

use oneiroi_graph::{GraphRevision, ParameterValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmpteTime {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub frames_per_second: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShowTime {
    pub monotonic_ns: u64,
    pub frame_id: u64,
    pub beat_ticks: i64,
    pub timecode: Option<SmpteTime>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum CommandOrigin {
    Operator,
    Midi(String),
    Osc(String),
    Score,
    Replay,
    Automation(String),
    Remote(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub enum CommandOperation {
    LaunchClip {
        deck: u8,
        slot: u8,
    },
    LaunchScene {
        slot: u8,
    },
    SetParameter {
        path: String,
        value: ParameterValue,
    },
    SetTempo {
        bpm: f64,
    },
    SetOutputEnabled {
        enabled: bool,
    },
    SetBlackout {
        enabled: bool,
    },
    SetRandomSeed {
        scope: String,
        seed: u64,
    },
    GraphCommitted {
        revision: GraphRevision,
    },
    External {
        kind: String,
        payload: serde_json::Value,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowCommand {
    pub sequence: u64,
    pub command_id: u64,
    pub origin: CommandOrigin,
    pub execute_at: ShowTime,
    pub operation: CommandOperation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub graph_revision: GraphRevision,
    pub bpm: f64,
    pub output_enabled: bool,
    pub blackout: bool,
    pub active_clips: [Option<u8>; 4],
    pub parameters: BTreeMap<String, ParameterValue>,
    pub random_seeds: BTreeMap<String, u64>,
    pub last_sequence: Option<u64>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            graph_revision: GraphRevision(1),
            bpm: 120.0,
            output_enabled: false,
            blackout: false,
            active_clips: [None; 4],
            parameters: BTreeMap::new(),
            random_seeds: BTreeMap::new(),
            last_sequence: None,
        }
    }
}

impl SessionState {
    pub fn apply(&mut self, command: &ShowCommand) -> Result<(), SessionError> {
        if self
            .last_sequence
            .is_some_and(|last| command.sequence <= last)
        {
            return Err(SessionError::NonMonotonicSequence {
                previous: self.last_sequence.unwrap_or_default(),
                next: command.sequence,
            });
        }
        match &command.operation {
            CommandOperation::LaunchClip { deck, slot } => {
                let active = self
                    .active_clips
                    .get_mut(usize::from(*deck))
                    .ok_or(SessionError::InvalidDeck(*deck))?;
                *active = Some(*slot);
            }
            CommandOperation::LaunchScene { slot } => self.active_clips.fill(Some(*slot)),
            CommandOperation::SetParameter { path, value } => {
                self.parameters.insert(path.clone(), value.clone());
            }
            CommandOperation::SetTempo { bpm }
                if bpm.is_finite() && (20.0..=400.0).contains(bpm) =>
            {
                self.bpm = *bpm;
            }
            CommandOperation::SetTempo { bpm } => return Err(SessionError::InvalidTempo(*bpm)),
            CommandOperation::SetOutputEnabled { enabled } => self.output_enabled = *enabled,
            CommandOperation::SetBlackout { enabled } => self.blackout = *enabled,
            CommandOperation::SetRandomSeed { scope, seed } => {
                self.random_seeds.insert(scope.clone(), *seed);
            }
            CommandOperation::GraphCommitted { revision } => self.graph_revision = *revision,
            CommandOperation::External { .. } => {}
        }
        self.last_sequence = Some(command.sequence);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateCheckpoint {
    pub after_sequence: Option<u64>,
    pub at: ShowTime,
    pub state: SessionState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerformanceTake {
    pub name: String,
    commands: Vec<ShowCommand>,
    checkpoints: Vec<StateCheckpoint>,
    next_sequence: u64,
    next_command_id: u64,
}

/// Serializable, append-only command history organized as named performance
/// takes. Recorded commands are never edited in place; alternate decisions
/// create a branch or a new take.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEventLog {
    takes: Vec<PerformanceTake>,
    active_take: usize,
}

impl SessionEventLog {
    pub fn new(initial_take: impl Into<String>) -> Self {
        Self {
            takes: vec![PerformanceTake::new(initial_take)],
            active_take: 0,
        }
    }

    pub fn takes(&self) -> &[PerformanceTake] {
        &self.takes
    }

    pub fn active_take(&self) -> &PerformanceTake {
        &self.takes[self.active_take]
    }

    pub fn active_take_mut(&mut self) -> &mut PerformanceTake {
        &mut self.takes[self.active_take]
    }

    pub fn start_take(&mut self, name: impl Into<String>) -> usize {
        self.takes.push(PerformanceTake::new(name));
        self.active_take = self.takes.len() - 1;
        self.active_take
    }

    pub fn select_take(&mut self, index: usize) -> Result<(), SessionError> {
        if index >= self.takes.len() {
            return Err(SessionError::InvalidTake(index));
        }
        self.active_take = index;
        Ok(())
    }

    pub fn branch_active(&mut self, name: impl Into<String>, through_sequence: u64) -> usize {
        let branch = self.active_take().branch(name.into(), through_sequence);
        self.takes.push(branch);
        self.active_take = self.takes.len() - 1;
        self.active_take
    }
}

impl PerformanceTake {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            commands: Vec::new(),
            checkpoints: Vec::new(),
            next_sequence: 0,
            next_command_id: 1,
        }
    }

    pub fn commands(&self) -> &[ShowCommand] {
        &self.commands
    }

    pub fn checkpoints(&self) -> &[StateCheckpoint] {
        &self.checkpoints
    }

    pub fn record(
        &mut self,
        origin: CommandOrigin,
        execute_at: ShowTime,
        operation: CommandOperation,
    ) -> &ShowCommand {
        let command = ShowCommand {
            sequence: self.next_sequence,
            command_id: self.next_command_id,
            origin,
            execute_at,
            operation,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.next_command_id = self.next_command_id.saturating_add(1);
        self.commands.push(command);
        self.commands.last().expect("command was appended")
    }

    pub fn record_and_apply(
        &mut self,
        state: &mut SessionState,
        origin: CommandOrigin,
        execute_at: ShowTime,
        operation: CommandOperation,
    ) -> Result<&ShowCommand, SessionError> {
        let command = ShowCommand {
            sequence: self.next_sequence,
            command_id: self.next_command_id,
            origin,
            execute_at,
            operation,
        };
        // Validate and update state before making the command durable in the
        // append-only take.
        state.apply(&command)?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.next_command_id = self.next_command_id.saturating_add(1);
        self.commands.push(command);
        Ok(self.commands.last().expect("command was appended"))
    }

    pub fn checkpoint(&mut self, at: ShowTime, state: &SessionState) {
        self.checkpoints.push(StateCheckpoint {
            after_sequence: state.last_sequence,
            at,
            state: state.clone(),
        });
    }

    pub fn replay_until(&self, time: ShowTime) -> Result<SessionState, SessionError> {
        let checkpoint = self
            .checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.at.monotonic_ns <= time.monotonic_ns)
            .max_by_key(|checkpoint| checkpoint.at.monotonic_ns);
        let mut state = checkpoint
            .map(|checkpoint| checkpoint.state.clone())
            .unwrap_or_default();
        let after = checkpoint.and_then(|checkpoint| checkpoint.after_sequence);
        for command in self.commands.iter().filter(|command| {
            command.execute_at.monotonic_ns <= time.monotonic_ns
                && after.is_none_or(|sequence| command.sequence > sequence)
        }) {
            state.apply(command)?;
        }
        Ok(state)
    }

    pub fn branch(&self, name: impl Into<String>, through_sequence: u64) -> Self {
        let commands: Vec<_> = self
            .commands
            .iter()
            .take_while(|command| command.sequence <= through_sequence)
            .cloned()
            .collect();
        let checkpoints = self
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint
                    .after_sequence
                    .is_none_or(|sequence| sequence <= through_sequence)
            })
            .cloned()
            .collect();
        Self {
            name: name.into(),
            next_sequence: commands
                .last()
                .map_or(0, |command| command.sequence.saturating_add(1)),
            next_command_id: commands
                .iter()
                .map(|command| command.command_id)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            commands,
            checkpoints,
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum SessionError {
    #[error("command sequence moved backward from {previous} to {next}")]
    NonMonotonicSequence { previous: u64, next: u64 },
    #[error("deck {0} is outside the four-deck mixer")]
    InvalidDeck(u8),
    #[error("tempo {0} is outside 20–400 BPM")]
    InvalidTempo(f64),
    #[error("performance take index {0} does not exist")]
    InvalidTake(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(seconds: u64) -> ShowTime {
        ShowTime {
            monotonic_ns: seconds * 1_000_000_000,
            frame_id: seconds * 60,
            beat_ticks: seconds as i64 * 1_920,
            timecode: None,
        }
    }

    #[test]
    fn replay_reconstructs_show_state_from_commands() {
        let mut take = PerformanceTake::new("rehearsal");
        take.record(
            CommandOrigin::Operator,
            time(1),
            CommandOperation::LaunchClip { deck: 2, slot: 5 },
        );
        take.record(
            CommandOrigin::Midi("controller".to_owned()),
            time(2),
            CommandOperation::SetTempo { bpm: 128.0 },
        );
        take.record(
            CommandOrigin::Operator,
            time(3),
            CommandOperation::SetRandomSeed {
                scope: "particles".to_owned(),
                seed: 42,
            },
        );

        let replayed = take.replay_until(time(3)).unwrap();
        assert_eq!(replayed.active_clips[2], Some(5));
        assert_eq!(replayed.bpm, 128.0);
        assert_eq!(replayed.random_seeds["particles"], 42);
    }

    #[test]
    fn replay_starts_from_the_latest_checkpoint() {
        let mut take = PerformanceTake::new("show");
        let first = take
            .record(
                CommandOrigin::Operator,
                time(1),
                CommandOperation::SetBlackout { enabled: true },
            )
            .clone();
        let mut state = SessionState::default();
        state.apply(&first).unwrap();
        take.checkpoint(time(1), &state);
        take.record(
            CommandOrigin::Operator,
            time(2),
            CommandOperation::SetBlackout { enabled: false },
        );

        let replayed = take.replay_until(time(2)).unwrap();
        assert!(!replayed.blackout);
        assert_eq!(replayed.last_sequence, Some(1));
    }

    #[test]
    fn a_take_can_branch_without_renumbering_recorded_commands() {
        let mut take = PerformanceTake::new("show");
        for slot in 0..3 {
            take.record(
                CommandOrigin::Operator,
                time(u64::from(slot) + 1),
                CommandOperation::LaunchScene { slot },
            );
        }
        let mut alternate = take.branch("alternate drop", 1);
        alternate.record(
            CommandOrigin::Operator,
            time(4),
            CommandOperation::LaunchScene { slot: 7 },
        );

        assert_eq!(alternate.commands().len(), 3);
        assert_eq!(alternate.commands()[2].sequence, 2);
        assert_eq!(take.commands().len(), 3);
    }

    #[test]
    fn invalid_commands_do_not_advance_session_sequence() {
        let mut state = SessionState::default();
        let command = ShowCommand {
            sequence: 0,
            command_id: 1,
            origin: CommandOrigin::Operator,
            execute_at: time(0),
            operation: CommandOperation::LaunchClip { deck: 8, slot: 0 },
        };

        assert_eq!(state.apply(&command), Err(SessionError::InvalidDeck(8)));
        assert_eq!(state.last_sequence, None);
    }

    #[test]
    fn invalid_command_is_not_appended_to_a_take() {
        let mut take = PerformanceTake::new("show");
        let mut state = SessionState::default();
        let result = take.record_and_apply(
            &mut state,
            CommandOrigin::Operator,
            time(0),
            CommandOperation::SetTempo { bpm: 900.0 },
        );

        assert_eq!(result, Err(SessionError::InvalidTempo(900.0)));
        assert!(take.commands().is_empty());
    }

    #[test]
    fn event_log_preserves_named_takes_and_switches_explicitly() {
        let mut log = SessionEventLog::new("rehearsal");
        log.start_take("show");
        log.branch_active("alternate", 0);

        assert_eq!(log.takes().len(), 3);
        assert_eq!(log.active_take().name, "alternate");
        log.select_take(0).unwrap();
        assert_eq!(log.active_take().name, "rehearsal");
        assert_eq!(log.select_take(99), Err(SessionError::InvalidTake(99)));
    }
}
