use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{SessionError, SessionState, ShowCommand, ShowTime, StateCheckpoint};

pub const JOURNAL_FORMAT: &str = "oneiroi-session-journal";
pub const JOURNAL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "record")]
pub enum JournalRecord {
    Header {
        format: String,
        version: u32,
        take_name: String,
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        take_id: Option<String>,
    },
    Command {
        command: ShowCommand,
    },
    Checkpoint {
        checkpoint: StateCheckpoint,
    },
    Marker {
        marker: TimelineMarker,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMarker {
    pub at: ShowTime,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CheckpointFile {
    format: String,
    version: u32,
    checkpoint: StateCheckpoint,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JournalHealth {
    pub commands_written: u64,
    pub checkpoints_written: u64,
    pub markers_written: u64,
    pub queue_overruns: u64,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct SharedHealth {
    commands_written: AtomicU64,
    checkpoints_written: AtomicU64,
    markers_written: AtomicU64,
    queue_overruns: AtomicU64,
    last_error: Mutex<Option<String>>,
}

enum WriterCommand {
    Append(ShowCommand),
    Checkpoint(StateCheckpoint),
    Marker(TimelineMarker),
    Flush(mpsc::Sender<Result<(), String>>),
    Shutdown(mpsc::Sender<Result<(), String>>),
}

pub struct JournalWriter {
    sender: SyncSender<WriterCommand>,
    health: Arc<SharedHealth>,
    worker: Option<JoinHandle<()>>,
    journal_path: PathBuf,
    checkpoint_path: PathBuf,
}

impl JournalWriter {
    pub fn open(
        journal_path: impl Into<PathBuf>,
        checkpoint_path: impl Into<PathBuf>,
        take_name: impl Into<String>,
        queue_capacity: usize,
    ) -> Result<Self, JournalError> {
        Self::open_linked(
            journal_path,
            checkpoint_path,
            take_name,
            None,
            None,
            queue_capacity,
        )
    }

    pub fn open_linked(
        journal_path: impl Into<PathBuf>,
        checkpoint_path: impl Into<PathBuf>,
        take_name: impl Into<String>,
        project_id: Option<String>,
        take_id: Option<String>,
        queue_capacity: usize,
    ) -> Result<Self, JournalError> {
        if queue_capacity == 0 {
            return Err(JournalError::InvalidQueueCapacity);
        }
        validate_optional_identity(project_id.as_deref())?;
        validate_optional_identity(take_id.as_deref())?;
        let journal_path = journal_path.into();
        let checkpoint_path = checkpoint_path.into();
        if let Some(parent) = journal_path.parent() {
            fs::create_dir_all(parent).map_err(|source| JournalError::Io {
                path: parent.to_owned(),
                source,
            })?;
        }
        if let Some(parent) = checkpoint_path.parent() {
            fs::create_dir_all(parent).map_err(|source| JournalError::Io {
                path: parent.to_owned(),
                source,
            })?;
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&journal_path)
            .map_err(|source| JournalError::Io {
                path: journal_path.clone(),
                source,
            })?;
        write_record(
            &mut file,
            &JournalRecord::Header {
                format: JOURNAL_FORMAT.to_owned(),
                version: JOURNAL_VERSION,
                take_name: take_name.into(),
                project_id,
                take_id,
            },
        )
        .map_err(|source| JournalError::Io {
            path: journal_path.clone(),
            source,
        })?;
        file.sync_data().map_err(|source| JournalError::Io {
            path: journal_path.clone(),
            source,
        })?;

        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let health = Arc::new(SharedHealth::default());
        let worker_health = health.clone();
        let worker_checkpoint = checkpoint_path.clone();
        let worker = thread::Builder::new()
            .name("oneiroi-session-journal".to_owned())
            .spawn(move || writer_loop(file, receiver, &worker_checkpoint, &worker_health))
            .map_err(JournalError::Spawn)?;
        Ok(Self {
            sender,
            health,
            worker: Some(worker),
            journal_path,
            checkpoint_path,
        })
    }

    pub fn journal_path(&self) -> &Path {
        &self.journal_path
    }

    pub fn checkpoint_path(&self) -> &Path {
        &self.checkpoint_path
    }

    pub fn try_append(&self, command: ShowCommand) -> Result<(), JournalEnqueueError> {
        self.try_send(WriterCommand::Append(command))
    }

    pub fn try_checkpoint(&self, checkpoint: StateCheckpoint) -> Result<(), JournalEnqueueError> {
        self.try_send(WriterCommand::Checkpoint(checkpoint))
    }

    pub fn try_marker(&self, marker: TimelineMarker) -> Result<(), JournalEnqueueError> {
        self.try_send(WriterCommand::Marker(marker))
    }

    pub fn health(&self) -> JournalHealth {
        JournalHealth {
            commands_written: self.health.commands_written.load(Ordering::Relaxed),
            checkpoints_written: self.health.checkpoints_written.load(Ordering::Relaxed),
            markers_written: self.health.markers_written.load(Ordering::Relaxed),
            queue_overruns: self.health.queue_overruns.load(Ordering::Relaxed),
            last_error: self
                .health
                .last_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        }
    }

    pub fn flush(&self) -> Result<(), JournalError> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(WriterCommand::Flush(sender))
            .map_err(|_| JournalError::WorkerDisconnected)?;
        receiver
            .recv()
            .map_err(|_| JournalError::WorkerDisconnected)?
            .map_err(JournalError::Worker)
    }

    fn try_send(&self, command: WriterCommand) -> Result<(), JournalEnqueueError> {
        match self.sender.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.health.queue_overruns.fetch_add(1, Ordering::Relaxed);
                Err(JournalEnqueueError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => Err(JournalEnqueueError::Disconnected),
        }
    }
}

impl Drop for JournalWriter {
    fn drop(&mut self) {
        let (sender, receiver) = mpsc::channel();
        if self.sender.send(WriterCommand::Shutdown(sender)).is_ok() {
            let _ = receiver.recv();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn writer_loop(
    mut file: File,
    receiver: Receiver<WriterCommand>,
    checkpoint_path: &Path,
    health: &SharedHealth,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            WriterCommand::Append(command) => {
                let result = write_record(&mut file, &JournalRecord::Command { command });
                observe(result, health, &health.commands_written, "append command");
            }
            WriterCommand::Checkpoint(checkpoint) => {
                let result = write_record(
                    &mut file,
                    &JournalRecord::Checkpoint {
                        checkpoint: checkpoint.clone(),
                    },
                )
                .and_then(|()| file.sync_data())
                .and_then(|()| write_checkpoint_atomic(checkpoint_path, &checkpoint));
                observe(
                    result,
                    health,
                    &health.checkpoints_written,
                    "write checkpoint",
                );
            }
            WriterCommand::Marker(marker) => {
                let result = write_record(&mut file, &JournalRecord::Marker { marker });
                observe(
                    result,
                    health,
                    &health.markers_written,
                    "append timeline marker",
                );
            }
            WriterCommand::Flush(response) => {
                let result = file.sync_data().map_err(|error| error.to_string());
                if let Err(error) = &result {
                    set_error(health, format!("flush journal: {error}"));
                }
                let _ = response.send(result);
            }
            WriterCommand::Shutdown(response) => {
                let result = file.sync_data().map_err(|error| error.to_string());
                let _ = response.send(result);
                break;
            }
        }
    }
}

fn observe(
    result: std::io::Result<()>,
    health: &SharedHealth,
    counter: &AtomicU64,
    operation: &str,
) {
    match result {
        Ok(()) => {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        Err(error) => set_error(health, format!("{operation}: {error}")),
    }
}

fn set_error(health: &SharedHealth, error: String) {
    *health
        .last_error
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
}

fn write_record(file: &mut File, record: &JournalRecord) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    file.write_all(&bytes)
}

fn write_checkpoint_atomic(path: &Path, checkpoint: &StateCheckpoint) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let file = File::create(&temporary)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(
        &mut writer,
        &CheckpointFile {
            format: JOURNAL_FORMAT.to_owned(),
            version: JOURNAL_VERSION,
            checkpoint: checkpoint.clone(),
        },
    )
    .map_err(std::io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    fs::rename(temporary, path)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct JournalRecovery {
    pub take_name: String,
    pub project_id: Option<String>,
    pub take_id: Option<String>,
    pub checkpoint: Option<StateCheckpoint>,
    pub commands: Vec<ShowCommand>,
    pub history_commands: Vec<ShowCommand>,
    pub history_checkpoints: Vec<StateCheckpoint>,
    pub markers: Vec<TimelineMarker>,
    pub ignored_partial_tail: bool,
}

impl JournalRecovery {
    /// Reconstruct the latest valid state from the atomic checkpoint and the
    /// strictly later commands retained from the journal.
    pub fn replay_state(&self) -> Result<SessionState, SessionError> {
        let mut state = self
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.state.clone())
            .unwrap_or_default();
        for command in &self.commands {
            state.apply(command)?;
        }
        Ok(state)
    }

    pub fn latest_time(&self) -> ShowTime {
        let command = self
            .history_commands
            .last()
            .map(|command| command.execute_at);
        let checkpoint = self.checkpoint.as_ref().map(|checkpoint| checkpoint.at);
        let marker = self.markers.last().map(|marker| marker.at);
        [command, checkpoint, marker]
            .into_iter()
            .flatten()
            .max_by_key(|time| time.monotonic_ns)
            .unwrap_or_default()
    }

    /// Replay the complete journal timeline at an operator-selected time.
    /// This is separate from `replay_state`, whose post-checkpoint tail is the
    /// fastest path for crash recovery at the latest durable position.
    pub fn replay_at(&self, time: ShowTime) -> Result<SessionState, SessionError> {
        let checkpoint = self
            .history_checkpoints
            .iter()
            .chain(self.checkpoint.iter())
            .filter(|checkpoint| checkpoint.at.monotonic_ns <= time.monotonic_ns)
            .max_by_key(|checkpoint| checkpoint.at.monotonic_ns);
        let mut state = checkpoint
            .map(|checkpoint| checkpoint.state.clone())
            .unwrap_or_default();
        let after = checkpoint.and_then(|checkpoint| checkpoint.after_sequence);
        for command in self.history_commands.iter().filter(|command| {
            command.execute_at.monotonic_ns <= time.monotonic_ns
                && after.is_none_or(|sequence| command.sequence > sequence)
        }) {
            state.apply(command)?;
        }
        Ok(state)
    }
}

pub fn recover_journal(
    journal_path: impl AsRef<Path>,
    checkpoint_path: impl AsRef<Path>,
) -> Result<JournalRecovery, JournalError> {
    let journal_path = journal_path.as_ref();
    let checkpoint_path = checkpoint_path.as_ref();
    let checkpoint = if checkpoint_path.exists() {
        let file = File::open(checkpoint_path).map_err(|source| JournalError::Io {
            path: checkpoint_path.to_owned(),
            source,
        })?;
        let envelope: CheckpointFile =
            serde_json::from_reader(BufReader::new(file)).map_err(|source| JournalError::Json {
                path: checkpoint_path.to_owned(),
                source,
            })?;
        validate_identity(&envelope.format, envelope.version)?;
        Some(envelope.checkpoint)
    } else {
        None
    };
    let file = File::open(journal_path).map_err(|source| JournalError::Io {
        path: journal_path.to_owned(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut line_number = 0_usize;
    let mut recovery = JournalRecovery {
        checkpoint,
        ..JournalRecovery::default()
    };
    let mut saw_header = false;
    loop {
        line.clear();
        let bytes = reader
            .read_until(b'\n', &mut line)
            .map_err(|source| JournalError::Io {
                path: journal_path.to_owned(),
                source,
            })?;
        if bytes == 0 {
            break;
        }
        line_number += 1;
        if !line.ends_with(b"\n") {
            recovery.ignored_partial_tail = true;
            break;
        }
        let record: JournalRecord =
            serde_json::from_slice(&line).map_err(|source| JournalError::JsonLine {
                path: journal_path.to_owned(),
                line: line_number,
                source,
            })?;
        match record {
            JournalRecord::Header {
                format,
                version,
                take_name,
                project_id,
                take_id,
            } if !saw_header => {
                validate_identity(&format, version)?;
                validate_optional_identity(project_id.as_deref())?;
                validate_optional_identity(take_id.as_deref())?;
                recovery.take_name = take_name;
                recovery.project_id = project_id;
                recovery.take_id = take_id;
                saw_header = true;
            }
            JournalRecord::Header { .. } => return Err(JournalError::DuplicateHeader),
            JournalRecord::Command { command } if saw_header => {
                if let Some(previous) = recovery.history_commands.last()
                    && command.sequence <= previous.sequence
                {
                    return Err(JournalError::NonMonotonicSequence {
                        previous: previous.sequence,
                        next: command.sequence,
                    });
                }
                let after = recovery
                    .checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.after_sequence);
                if after.is_none_or(|sequence| command.sequence > sequence) {
                    recovery.commands.push(command.clone());
                }
                recovery.history_commands.push(command);
            }
            JournalRecord::Checkpoint { checkpoint } if saw_header => {
                recovery.history_checkpoints.push(checkpoint);
            }
            JournalRecord::Marker { marker } if saw_header => {
                if marker.label.is_empty()
                    || marker.label.len() > 128
                    || marker.label.chars().any(char::is_control)
                    || recovery
                        .markers
                        .last()
                        .is_some_and(|previous| previous.at.monotonic_ns > marker.at.monotonic_ns)
                {
                    return Err(JournalError::InvalidMarker);
                }
                recovery.markers.push(marker);
            }
            _ => return Err(JournalError::MissingHeader),
        }
    }
    if !saw_header {
        return Err(JournalError::MissingHeader);
    }
    Ok(recovery)
}

fn validate_identity(format: &str, version: u32) -> Result<(), JournalError> {
    if format != JOURNAL_FORMAT {
        return Err(JournalError::WrongFormat(format.to_owned()));
    }
    if version != JOURNAL_VERSION {
        return Err(JournalError::UnsupportedVersion(version));
    }
    Ok(())
}

fn validate_optional_identity(identity: Option<&str>) -> Result<(), JournalError> {
    if identity.is_some_and(|identity| {
        identity.len() != 32 || !identity.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(JournalError::InvalidIdentity);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum JournalEnqueueError {
    #[error("session journal queue is full")]
    QueueFull,
    #[error("session journal worker disconnected")]
    Disconnected,
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal queue capacity must be greater than zero")]
    InvalidQueueCapacity,
    #[error("journal I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("journal JSON failed at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("journal JSON failed at {path} line {line}: {source}")]
    JsonLine {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("journal worker could not start: {0}")]
    Spawn(std::io::Error),
    #[error("journal worker disconnected")]
    WorkerDisconnected,
    #[error("journal worker failed: {0}")]
    Worker(String),
    #[error("wrong journal format {0}")]
    WrongFormat(String),
    #[error("unsupported journal version {0}")]
    UnsupportedVersion(u32),
    #[error("journal is missing its header")]
    MissingHeader,
    #[error("journal contains more than one header")]
    DuplicateHeader,
    #[error("journal sequence moved backward from {previous} to {next}")]
    NonMonotonicSequence { previous: u64, next: u64 },
    #[error("journal project or take identity is invalid")]
    InvalidIdentity,
    #[error("journal timeline marker is invalid or out of order")]
    InvalidMarker,
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{CommandOperation, CommandOrigin, SessionState, ShowTime, StateCheckpoint};

    use super::*;

    fn temporary_paths(name: &str) -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("oneiroi-journal-{name}-{unique}"));
        (
            directory.join("take.jsonl"),
            directory.join("checkpoint.json"),
        )
    }

    fn command(sequence: u64) -> ShowCommand {
        ShowCommand {
            sequence,
            command_id: sequence + 1,
            origin: CommandOrigin::Operator,
            execute_at: ShowTime {
                frame_id: sequence,
                ..ShowTime::default()
            },
            operation: CommandOperation::SetBlackout {
                enabled: sequence.is_multiple_of(2),
            },
        }
    }

    #[test]
    fn background_writer_round_trips_commands_and_atomic_checkpoint() {
        let (journal, checkpoint_path) = temporary_paths("roundtrip");
        let writer = JournalWriter::open(&journal, &checkpoint_path, "show", 8).unwrap();
        writer.try_append(command(0)).unwrap();
        let checkpoint = StateCheckpoint {
            after_sequence: Some(0),
            at: ShowTime::default(),
            state: SessionState {
                last_sequence: Some(0),
                ..SessionState::default()
            },
        };
        writer.try_checkpoint(checkpoint.clone()).unwrap();
        writer.try_append(command(1)).unwrap();
        let marker = TimelineMarker {
            at: ShowTime {
                monotonic_ns: 500,
                ..ShowTime::default()
            },
            label: "Drop".to_owned(),
        };
        writer.try_marker(marker.clone()).unwrap();
        writer.flush().unwrap();
        assert_eq!(writer.health().commands_written, 2);
        assert_eq!(writer.health().markers_written, 1);
        drop(writer);

        let recovered = recover_journal(&journal, &checkpoint_path).unwrap();
        assert_eq!(recovered.take_name, "show");
        assert_eq!(recovered.checkpoint, Some(checkpoint));
        assert_eq!(recovered.commands, vec![command(1)]);
        assert_eq!(recovered.markers, vec![marker]);
    }

    #[test]
    fn recovery_ignores_an_incomplete_final_record() {
        let (journal, checkpoint) = temporary_paths("partial");
        let writer = JournalWriter::open(&journal, &checkpoint, "show", 2).unwrap();
        writer.flush().unwrap();
        drop(writer);
        let mut file = OpenOptions::new().append(true).open(&journal).unwrap();
        file.write_all(br#"{"record":"command","command":"#)
            .unwrap();
        file.sync_all().unwrap();

        let recovered = recover_journal(&journal, &checkpoint).unwrap();
        assert!(recovered.ignored_partial_tail);
        assert!(recovered.commands.is_empty());
    }

    #[test]
    fn legacy_header_without_project_identity_remains_readable() {
        let record: JournalRecord = serde_json::from_str(
            r#"{"record":"header","format":"oneiroi-session-journal","version":1,"take_name":"Legacy"}"#,
        )
        .unwrap();
        assert_eq!(
            record,
            JournalRecord::Header {
                format: JOURNAL_FORMAT.to_owned(),
                version: JOURNAL_VERSION,
                take_name: "Legacy".to_owned(),
                project_id: None,
                take_id: None,
            }
        );
    }

    #[test]
    fn timeline_replay_selects_the_latest_checkpoint_before_the_cursor() {
        let first = ShowCommand {
            sequence: 0,
            command_id: 1,
            origin: CommandOrigin::Operator,
            execute_at: ShowTime {
                monotonic_ns: 1_000,
                ..ShowTime::default()
            },
            operation: CommandOperation::SetTempo { bpm: 130.0 },
        };
        let second = ShowCommand {
            sequence: 1,
            command_id: 2,
            origin: CommandOrigin::Operator,
            execute_at: ShowTime {
                monotonic_ns: 3_000,
                ..ShowTime::default()
            },
            operation: CommandOperation::SetBlackout { enabled: true },
        };
        let recovery = JournalRecovery {
            history_commands: vec![first, second],
            history_checkpoints: vec![StateCheckpoint {
                after_sequence: Some(0),
                at: ShowTime {
                    monotonic_ns: 2_000,
                    ..ShowTime::default()
                },
                state: SessionState {
                    bpm: 130.0,
                    last_sequence: Some(0),
                    ..SessionState::default()
                },
            }],
            ..JournalRecovery::default()
        };

        let before = recovery
            .replay_at(ShowTime {
                monotonic_ns: 1_500,
                ..ShowTime::default()
            })
            .unwrap();
        let after = recovery
            .replay_at(ShowTime {
                monotonic_ns: 3_000,
                ..ShowTime::default()
            })
            .unwrap();
        assert_eq!(before.bpm, 130.0);
        assert!(!before.blackout);
        assert!(after.blackout);
    }

    #[test]
    fn complete_malformed_record_is_not_silently_ignored() {
        let (journal, checkpoint) = temporary_paths("malformed");
        let writer = JournalWriter::open(&journal, &checkpoint, "show", 2).unwrap();
        writer.flush().unwrap();
        drop(writer);
        let mut file = OpenOptions::new().append(true).open(&journal).unwrap();
        file.write_all(b"not-json\n").unwrap();

        assert!(matches!(
            recover_journal(&journal, &checkpoint),
            Err(JournalError::JsonLine { line: 2, .. })
        ));
    }
}
