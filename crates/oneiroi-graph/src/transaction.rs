use std::sync::Arc;

use thiserror::Error;

use crate::{CompileError, GraphCompiler, GraphRevision, ProjectGraph, RenderPlan};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimelinePosition {
    pub frame_id: u64,
    pub beat_ticks: i64,
    pub timecode_frames: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitPoint {
    NextFrame,
    NextBeat {
        ticks_per_beat: i64,
    },
    NextBar {
        ticks_per_beat: i64,
        beats_per_bar: i64,
    },
    Frame(u64),
    BeatTick(i64),
    TimecodeFrame(i64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionState {
    Editing,
    Ready,
    Scheduled,
}

#[derive(Clone, Debug)]
pub struct GraphTransaction {
    pub id: u64,
    pub graph: ProjectGraph,
    pub state: TransactionState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitReceipt {
    pub transaction_id: u64,
    pub revision: GraphRevision,
    pub committed_at: TimelinePosition,
}

#[derive(Clone, Debug)]
struct Prepared {
    transaction: GraphTransaction,
    plan: RenderPlan,
    target: Option<TimelinePosition>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TransactionError {
    #[error("a shadow graph transaction is already open")]
    AlreadyOpen,
    #[error("no shadow graph transaction is open")]
    NotOpen,
    #[error("shadow graph has not compiled successfully")]
    NotReady,
    #[error("commit point has invalid timing parameters")]
    InvalidCommitPoint,
    #[error(transparent)]
    Compile(#[from] CompileError),
}

pub struct TransactionManager {
    active_graph: ProjectGraph,
    active_plan: RenderPlan,
    last_known_good: RenderPlan,
    shadow: Option<Prepared>,
    next_id: u64,
}

impl TransactionManager {
    pub fn new(active_graph: ProjectGraph, active_plan: RenderPlan) -> Self {
        Self {
            active_graph,
            last_known_good: active_plan.clone(),
            active_plan,
            shadow: None,
            next_id: 1,
        }
    }

    pub fn active_graph(&self) -> &ProjectGraph {
        &self.active_graph
    }

    pub fn active_plan(&self) -> &RenderPlan {
        &self.active_plan
    }

    pub fn active_plan_shared(&self) -> Arc<RenderPlan> {
        Arc::new(self.active_plan.clone())
    }

    pub fn last_known_good(&self) -> &RenderPlan {
        &self.last_known_good
    }

    pub fn begin(&mut self) -> Result<&mut ProjectGraph, TransactionError> {
        if self.shadow.is_some() {
            return Err(TransactionError::AlreadyOpen);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.shadow = Some(Prepared {
            transaction: GraphTransaction {
                id,
                graph: self.active_graph.clone(),
                state: TransactionState::Editing,
            },
            plan: self.active_plan.clone(),
            target: None,
        });
        Ok(&mut self.shadow.as_mut().expect("inserted").transaction.graph)
    }

    pub fn shadow_graph_mut(&mut self) -> Result<&mut ProjectGraph, TransactionError> {
        let prepared = self.shadow.as_mut().ok_or(TransactionError::NotOpen)?;
        prepared.transaction.state = TransactionState::Editing;
        prepared.target = None;
        Ok(&mut prepared.transaction.graph)
    }

    pub fn prepare(&mut self, compiler: &GraphCompiler<'_>) -> Result<(), TransactionError> {
        let prepared = self.shadow.as_mut().ok_or(TransactionError::NotOpen)?;
        let next_revision = GraphRevision(self.active_graph.revision.0.saturating_add(1));
        prepared.transaction.graph.revision = next_revision;
        // Compilation is all-or-nothing: the active plan is untouched on error.
        prepared.plan = compiler.compile(&prepared.transaction.graph)?;
        prepared.transaction.state = TransactionState::Ready;
        Ok(())
    }

    pub fn schedule(
        &mut self,
        point: CommitPoint,
        now: TimelinePosition,
    ) -> Result<TimelinePosition, TransactionError> {
        let prepared = self.shadow.as_mut().ok_or(TransactionError::NotOpen)?;
        if prepared.transaction.state != TransactionState::Ready {
            return Err(TransactionError::NotReady);
        }
        let target = target_position(point, now)?;
        prepared.target = Some(target);
        prepared.transaction.state = TransactionState::Scheduled;
        Ok(target)
    }

    pub fn advance(&mut self, now: TimelinePosition) -> Option<CommitReceipt> {
        let due = self
            .shadow
            .as_ref()
            .and_then(|prepared| prepared.target)
            .is_some_and(|target| reached(target, now));
        if !due {
            return None;
        }
        let prepared = self.shadow.take().expect("due transaction exists");
        self.active_graph = prepared.transaction.graph;
        self.active_plan = prepared.plan;
        self.last_known_good = self.active_plan.clone();
        Some(CommitReceipt {
            transaction_id: prepared.transaction.id,
            revision: self.active_graph.revision,
            committed_at: now,
        })
    }

    pub fn discard(&mut self) -> Result<(), TransactionError> {
        self.shadow
            .take()
            .map(|_| ())
            .ok_or(TransactionError::NotOpen)
    }
}

fn target_position(
    point: CommitPoint,
    now: TimelinePosition,
) -> Result<TimelinePosition, TransactionError> {
    let mut target = now;
    match point {
        CommitPoint::NextFrame => target.frame_id = now.frame_id.saturating_add(1),
        CommitPoint::NextBeat { ticks_per_beat } if ticks_per_beat > 0 => {
            target.beat_ticks = now
                .beat_ticks
                .div_euclid(ticks_per_beat)
                .saturating_add(1)
                .saturating_mul(ticks_per_beat);
        }
        CommitPoint::NextBar {
            ticks_per_beat,
            beats_per_bar,
        } if ticks_per_beat > 0 && beats_per_bar > 0 => {
            let ticks = ticks_per_beat.saturating_mul(beats_per_bar);
            target.beat_ticks = now
                .beat_ticks
                .div_euclid(ticks)
                .saturating_add(1)
                .saturating_mul(ticks);
        }
        CommitPoint::Frame(frame) => target.frame_id = frame,
        CommitPoint::BeatTick(tick) => target.beat_ticks = tick,
        CommitPoint::TimecodeFrame(frame) if frame >= 0 => target.timecode_frames = Some(frame),
        _ => return Err(TransactionError::InvalidCommitPoint),
    }
    Ok(target)
}

fn reached(target: TimelinePosition, now: TimelinePosition) -> bool {
    let frame_reached = now.frame_id >= target.frame_id;
    let beat_reached = now.beat_ticks >= target.beat_ticks;
    let timecode_reached = match target.timecode_frames {
        Some(target) => now.timecode_frames.is_some_and(|now| now >= target),
        None => true,
    };
    frame_reached && beat_reached && timecode_reached
}
