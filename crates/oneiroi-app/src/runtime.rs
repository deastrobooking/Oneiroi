use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use oneiroi_graph::{
    CompileBudget, GraphCompiler, TimelinePosition, TransactionManager, builtin_registry,
    four_deck_performance_graph,
};
use oneiroi_io::{TakeMetadataProject, new_project_id};
use oneiroi_render::LoweredRenderPlan;
use oneiroi_session::{
    CommandOperation, CommandOrigin, JournalWriter, SessionEventLog, SessionState, ShowTime,
};

pub(crate) struct PerformanceRuntime {
    transactions: TransactionManager,
    render_plan: LoweredRenderPlan,
    event_log: SessionEventLog,
    state: SessionState,
    next_checkpoint_frame: u64,
    journal: Option<JournalWriter>,
    journal_directory: Option<PathBuf>,
    project_id: Option<String>,
    take_id: String,
    take_created_unix_ms: u64,
}

impl PerformanceRuntime {
    pub(crate) fn new(composition_extent: [u32; 2]) -> Result<Self> {
        let registry = builtin_registry();
        let graph = four_deck_performance_graph();
        let compiler = GraphCompiler::new(
            &registry,
            CompileBudget {
                composition_extent,
                ..CompileBudget::default()
            },
        );
        let plan = compiler
            .compile(&graph)
            .context("compile the four-deck performance graph")?;
        let render_plan =
            LoweredRenderPlan::lower(&plan).context("lower the four-deck render plan")?;
        let transactions = TransactionManager::new(graph, plan);
        let mut runtime = Self {
            transactions,
            render_plan,
            event_log: SessionEventLog::new("Live"),
            state: SessionState::default(),
            next_checkpoint_frame: 600,
            journal: None,
            journal_directory: None,
            project_id: None,
            take_id: new_project_id(),
            take_created_unix_ms: unix_millis(),
        };
        runtime.record(
            CommandOrigin::Automation("startup".to_owned()),
            ShowTime::default(),
            CommandOperation::GraphCommitted {
                revision: runtime.transactions.active_plan().revision(),
            },
        )?;
        Ok(runtime)
    }

    pub(crate) fn status(&self) -> String {
        let plan = self.transactions.active_plan();
        let graph = format!(
            "graph r{} · {} nodes · {:.2} ms budget · {:.1} MiB transient",
            plan.revision().0,
            plan.nodes().len(),
            plan.estimated_gpu_us() as f64 / 1_000.0,
            plan.estimated_texture_bytes() as f64 / (1024.0 * 1024.0),
        );
        let Some(journal) = &self.journal else {
            return format!("{graph} · journal disabled");
        };
        let health = journal.health();
        if let Some(error) = health.last_error {
            format!("{graph} · journal error: {error}")
        } else {
            format!(
                "{graph} · journal {} commands / {} checkpoints / {} markers / {} overruns",
                health.commands_written,
                health.checkpoints_written,
                health.markers_written,
                health.queue_overruns
            )
        }
    }

    pub(crate) fn enable_journal(&mut self, directory: &Path, project_id: &str) -> Result<()> {
        self.project_id = Some(project_id.to_owned());
        let writer = self.open_journal(
            directory,
            &self.event_log.active_take().name,
            project_id,
            &self.take_id,
        )?;
        for command in self.event_log.active_take().commands().iter().cloned() {
            writer.try_append(command).context("seed session journal")?;
        }
        self.journal = Some(writer);
        self.journal_directory = Some(directory.to_owned());
        Ok(())
    }

    fn open_journal(
        &self,
        directory: &Path,
        take_name: &str,
        project_id: &str,
        take_id: &str,
    ) -> Result<JournalWriter> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before the Unix epoch")?
            .as_nanos();
        let stem = format!("session-{}-{unique}", std::process::id());
        let writer = JournalWriter::open_linked(
            directory.join(format!("{stem}.jsonl")),
            directory.join(format!("{stem}.checkpoint.json")),
            take_name,
            Some(project_id.to_owned()),
            Some(take_id.to_owned()),
            4_096,
        )
        .context("open session journal")?;
        Ok(writer)
    }

    pub(crate) fn journal_path(&self) -> Option<&Path> {
        self.journal.as_ref().map(JournalWriter::journal_path)
    }

    pub(crate) fn take_metadata(&self) -> Option<TakeMetadataProject> {
        let journal_file = self
            .journal_path()?
            .file_name()?
            .to_string_lossy()
            .into_owned();
        Some(TakeMetadataProject {
            take_id: self.take_id.clone(),
            name: self.event_log.active_take().name.clone(),
            journal_file,
            created_unix_ms: self.take_created_unix_ms,
        })
    }

    /// Adopt a recovered state as the baseline of a fresh take. A new journal
    /// is opened before the current writer is replaced, and the baseline is
    /// checkpointed immediately so future sequence numbers restart safely.
    pub(crate) fn restore_baseline(
        &mut self,
        state: SessionState,
        take_name: &str,
        at: ShowTime,
    ) -> Result<()> {
        let project_id = self
            .project_id
            .clone()
            .context("session journal has no project identity")?;
        self.replace_baseline(state, &project_id, &format!("Recovered · {take_name}"), at)
    }

    pub(crate) fn start_project_baseline(
        &mut self,
        state: SessionState,
        project_id: &str,
        at: ShowTime,
    ) -> Result<()> {
        self.replace_baseline(state, project_id, "Live", at)
    }

    pub(crate) fn start_named_baseline(
        &mut self,
        state: SessionState,
        name: &str,
        at: ShowTime,
    ) -> Result<()> {
        let project_id = self
            .project_id
            .clone()
            .context("missing project identity")?;
        self.replace_baseline(state, &project_id, name, at)
    }

    pub(crate) fn active_graph(&self) -> &oneiroi_graph::ProjectGraph {
        self.transactions.active_graph()
    }

    pub(crate) fn random_seeds(&self) -> &std::collections::BTreeMap<String, u64> {
        &self.state.random_seeds
    }

    pub(crate) fn add_timeline_marker(&mut self, at: ShowTime, label: String) -> Result<()> {
        let journal = self
            .journal
            .as_ref()
            .context("session journal is disabled")?;
        journal
            .try_marker(oneiroi_session::TimelineMarker { at, label })
            .context("enqueue timeline marker")
    }

    pub(crate) fn set_project_graph(
        &mut self,
        graph: oneiroi_graph::ProjectGraph,
        extent: [u32; 2],
    ) -> Result<()> {
        let registry = builtin_registry();
        let plan = GraphCompiler::new(
            &registry,
            CompileBudget {
                composition_extent: extent,
                ..CompileBudget::default()
            },
        )
        .compile(&graph)
        .context("compile persisted project graph")?;
        let render_plan =
            LoweredRenderPlan::lower(&plan).context("lower persisted project graph")?;
        self.transactions = TransactionManager::new(graph, plan);
        self.render_plan = render_plan;
        Ok(())
    }

    fn replace_baseline(
        &mut self,
        mut state: SessionState,
        project_id: &str,
        take_name: &str,
        at: ShowTime,
    ) -> Result<()> {
        state.graph_revision = self.transactions.active_plan().revision();
        state.last_sequence = None;
        let new_name = take_name.to_owned();
        let new_take_id = new_project_id();
        let replacement = self
            .journal_directory
            .as_deref()
            .map(|directory| self.open_journal(directory, &new_name, project_id, &new_take_id))
            .transpose()?;
        if let Some(writer) = &replacement {
            writer
                .try_checkpoint(oneiroi_session::StateCheckpoint {
                    after_sequence: None,
                    at,
                    state: state.clone(),
                })
                .context("checkpoint recovered session baseline")?;
        }
        self.event_log = SessionEventLog::new(new_name);
        self.state = state;
        self.project_id = Some(project_id.to_owned());
        self.take_id = new_take_id;
        self.take_created_unix_ms = unix_millis();
        self.next_checkpoint_frame = at.frame_id.saturating_add(600);
        if replacement.is_some() {
            self.journal = replacement;
        }
        Ok(())
    }

    pub(crate) fn render_plan(&self) -> &LoweredRenderPlan {
        &self.render_plan
    }

    pub(crate) fn set_composition_extent(&mut self, extent: [u32; 2]) -> Result<()> {
        if self.render_plan.extent() == extent {
            return Ok(());
        }
        let registry = builtin_registry();
        let graph = self.transactions.active_graph().clone();
        let compiler = GraphCompiler::new(
            &registry,
            CompileBudget {
                composition_extent: extent,
                ..CompileBudget::default()
            },
        );
        let plan = compiler
            .compile(&graph)
            .context("recompile graph for composition extent")?;
        let render_plan = LoweredRenderPlan::lower(&plan).context("lower resized render plan")?;
        self.transactions = TransactionManager::new(graph, plan);
        self.render_plan = render_plan;
        Ok(())
    }

    pub(crate) fn record(
        &mut self,
        origin: CommandOrigin,
        at: ShowTime,
        operation: CommandOperation,
    ) -> Result<()> {
        let command = self
            .event_log
            .active_take_mut()
            .record_and_apply(&mut self.state, origin, at, operation)
            .context("record and apply show command")?
            .clone();
        if let Some(journal) = &self.journal {
            journal
                .try_append(command)
                .context("enqueue show command in session journal")?;
        }
        Ok(())
    }

    pub(crate) fn tick(&mut self, at: ShowTime) -> Result<()> {
        let timeline = TimelinePosition {
            frame_id: at.frame_id,
            beat_ticks: at.beat_ticks,
            timecode_frames: at.timecode.map(|timecode| {
                let seconds = i64::from(timecode.hours) * 3_600
                    + i64::from(timecode.minutes) * 60
                    + i64::from(timecode.seconds);
                seconds * i64::from(timecode.frames_per_second) + i64::from(timecode.frames)
            }),
        };
        let previous_graph = self.transactions.active_graph().clone();
        let previous_plan = self.transactions.active_plan().clone();
        if let Some(receipt) = self.transactions.advance(timeline) {
            let candidate = match LoweredRenderPlan::lower(self.transactions.active_plan()) {
                Ok(candidate) => candidate,
                Err(error) => {
                    self.transactions = TransactionManager::new(previous_graph, previous_plan);
                    return Err(error).context(
                        "committed graph has no safe renderer lowering; restored previous plan",
                    );
                }
            };
            self.render_plan = candidate;
            self.record(
                CommandOrigin::Automation("graph_transaction".to_owned()),
                at,
                CommandOperation::GraphCommitted {
                    revision: receipt.revision,
                },
            )?;
        }
        if at.frame_id >= self.next_checkpoint_frame {
            let checkpoint = self
                .event_log
                .active_take_mut()
                .checkpoint(at, &self.state)
                .clone();
            self.next_checkpoint_frame = at.frame_id.saturating_add(600);
            if let Some(journal) = &self.journal {
                journal
                    .try_checkpoint(checkpoint)
                    .context("enqueue session checkpoint")?;
            }
        }
        Ok(())
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use oneiroi_session::recover_journal;

    use super::*;

    #[test]
    fn resize_recompiles_before_replacing_the_active_render_schedule() {
        let mut runtime = PerformanceRuntime::new([1920, 1080]).unwrap();

        runtime.set_composition_extent([1280, 720]).unwrap();
        assert_eq!(runtime.render_plan().extent(), [1280, 720]);

        let error = runtime.set_composition_extent([7680, 4320]).unwrap_err();
        assert!(format!("{error:#}").contains("texture"));
        assert_eq!(runtime.render_plan().extent(), [1280, 720]);
    }

    #[test]
    fn persistent_runtime_journals_commands_and_checkpoints_off_thread() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("oneiroi-runtime-journal-{unique}"));
        let mut runtime = PerformanceRuntime::new([1920, 1080]).unwrap();
        runtime
            .enable_journal(&directory, &new_project_id())
            .unwrap();
        runtime
            .record(
                CommandOrigin::Operator,
                ShowTime {
                    frame_id: 10,
                    ..ShowTime::default()
                },
                CommandOperation::SetTempo { bpm: 128.0 },
            )
            .unwrap();
        runtime
            .tick(ShowTime {
                frame_id: 600,
                monotonic_ns: 10_000_000_000,
                ..ShowTime::default()
            })
            .unwrap();
        let journal = runtime.journal.as_ref().unwrap();
        journal.flush().unwrap();
        let journal_path = journal.journal_path().to_owned();
        let checkpoint_path = journal.checkpoint_path().to_owned();

        let recovered = recover_journal(journal_path, checkpoint_path).unwrap();
        assert_eq!(recovered.checkpoint.unwrap().state.bpm, 128.0);
        assert!(recovered.commands.is_empty());
    }

    #[test]
    fn restored_baseline_restarts_command_sequence_in_a_fresh_take() {
        let mut runtime = PerformanceRuntime::new([1920, 1080]).unwrap();
        runtime.project_id = Some(new_project_id());
        runtime
            .restore_baseline(
                SessionState {
                    bpm: 144.0,
                    last_sequence: Some(900),
                    ..SessionState::default()
                },
                "Recovered",
                ShowTime {
                    frame_id: 1_200,
                    ..ShowTime::default()
                },
            )
            .unwrap();

        runtime
            .record(
                CommandOrigin::Operator,
                ShowTime {
                    frame_id: 1_201,
                    ..ShowTime::default()
                },
                CommandOperation::SetBlackout { enabled: true },
            )
            .unwrap();

        assert_eq!(runtime.event_log.active_take().commands()[0].sequence, 0);
        assert_eq!(runtime.state.bpm, 144.0);
        assert!(runtime.state.blackout);
    }

    #[test]
    fn named_baseline_updates_take_metadata() {
        let mut runtime = PerformanceRuntime::new([1920, 1080]).unwrap();
        let project_id = new_project_id();
        runtime.project_id = Some(project_id);
        runtime
            .start_named_baseline(
                SessionState::default(),
                "Finale branch",
                ShowTime::default(),
            )
            .unwrap();
        assert_eq!(runtime.event_log.active_take().name, "Finale branch");
        assert_eq!(runtime.take_id.len(), 32);
    }
}
