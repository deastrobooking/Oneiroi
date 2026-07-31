use anyhow::{Context, Result};
use oneiroi_graph::{
    CompileBudget, GraphCompiler, TimelinePosition, TransactionManager, builtin_registry,
    four_deck_performance_graph,
};
use oneiroi_session::{CommandOperation, CommandOrigin, SessionEventLog, SessionState, ShowTime};

pub(crate) struct PerformanceRuntime {
    transactions: TransactionManager,
    event_log: SessionEventLog,
    state: SessionState,
    next_checkpoint_frame: u64,
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
        let transactions = TransactionManager::new(graph, plan);
        let mut runtime = Self {
            transactions,
            event_log: SessionEventLog::new("Live"),
            state: SessionState::default(),
            next_checkpoint_frame: 600,
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
        format!(
            "graph r{} · {} nodes · {:.2} ms budget · {:.1} MiB transient",
            plan.revision().0,
            plan.nodes().len(),
            plan.estimated_gpu_us() as f64 / 1_000.0,
            plan.estimated_texture_bytes() as f64 / (1024.0 * 1024.0),
        )
    }

    pub(crate) fn record(
        &mut self,
        origin: CommandOrigin,
        at: ShowTime,
        operation: CommandOperation,
    ) -> Result<()> {
        self.event_log
            .active_take_mut()
            .record_and_apply(&mut self.state, origin, at, operation)
            .context("record and apply show command")?;
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
        if let Some(receipt) = self.transactions.advance(timeline) {
            self.record(
                CommandOrigin::Automation("graph_transaction".to_owned()),
                at,
                CommandOperation::GraphCommitted {
                    revision: receipt.revision,
                },
            )?;
        }
        if at.frame_id >= self.next_checkpoint_frame {
            self.event_log.active_take_mut().checkpoint(at, &self.state);
            self.next_checkpoint_frame = at.frame_id.saturating_add(600);
        }
        Ok(())
    }
}
