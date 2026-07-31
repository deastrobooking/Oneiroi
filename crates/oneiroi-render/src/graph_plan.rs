//! Lowering from a device-neutral graph plan to the renderer's proven passes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use oneiroi_graph::{
    ColorSpace, DECK_EFFECTS_NODE, DECK_SOURCE_NODE, FOUR_DECK_MIXER_NODE, GraphRevision,
    MASTER_EFFECTS_NODE, NodeId, PROGRAM_OUTPUT_NODE, RenderPlan,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FusedDeckNodes {
    pub source: NodeId,
    pub effects: NodeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuiltInRenderStage {
    /// The fixed compositor fuses four source/effect branches into one bounded
    /// render pass. The logical graph nodes remain separately addressable.
    FourDeckComposite {
        decks: [FusedDeckNodes; 4],
        mixer: NodeId,
    },
    MasterEffects {
        node: NodeId,
    },
    ProgramOutput {
        node: NodeId,
    },
}

#[derive(Clone, Debug)]
struct LoweredPlanInner {
    revision: GraphRevision,
    extent: [u32; 2],
    stages: Arc<[BuiltInRenderStage]>,
}

/// Immutable renderer-specific schedule produced from a validated graph plan.
#[derive(Clone, Debug)]
pub struct LoweredRenderPlan(Arc<LoweredPlanInner>);

impl LoweredRenderPlan {
    pub fn lower(plan: &RenderPlan) -> Result<Self, LoweredPlanError> {
        if let Some(adapter) = plan.rate_adapters().first() {
            return Err(LoweredPlanError::UnsupportedRateAdapter(adapter.edge_index));
        }
        let nodes: BTreeMap<_, _> = plan.nodes().iter().map(|node| (node.id, node)).collect();
        let supported = [
            DECK_SOURCE_NODE,
            DECK_EFFECTS_NODE,
            FOUR_DECK_MIXER_NODE,
            MASTER_EFFECTS_NODE,
            PROGRAM_OUTPUT_NODE,
        ];
        if let Some(node) = plan
            .nodes()
            .iter()
            .find(|node| !supported.contains(&node.kind.as_str()))
        {
            return Err(LoweredPlanError::UnsupportedNode {
                node: node.id,
                kind: node.kind.clone(),
            });
        }

        let mixer = unique_node(plan, FOUR_DECK_MIXER_NODE)?;
        let master = unique_node(plan, MASTER_EFFECTS_NODE)?;
        let output = unique_node(plan, PROGRAM_OUTPUT_NODE)?;
        let mut consumed = BTreeSet::from([mixer, master, output]);
        let mut decks = [FusedDeckNodes {
            source: NodeId(0),
            effects: NodeId(0),
        }; 4];
        for (index, port) in ["deck_a", "deck_b", "deck_c", "deck_d"]
            .into_iter()
            .enumerate()
        {
            let effects = input_source(plan, mixer, port)?;
            require_kind(&nodes, effects, DECK_EFFECTS_NODE)?;
            let source = input_source(plan, effects, "video")?;
            require_kind(&nodes, source, DECK_SOURCE_NODE)?;
            decks[index] = FusedDeckNodes { source, effects };
            consumed.extend([source, effects]);
        }
        require_connection(plan, mixer, "program", master, "video")?;
        require_connection(plan, master, "video", output, "video")?;
        const COMPATIBILITY_NODE_COUNT: usize = 11;
        if consumed.len() != COMPATIBILITY_NODE_COUNT
            || plan.nodes().len() != COMPATIBILITY_NODE_COUNT
        {
            return Err(LoweredPlanError::UnexpectedNodeCount {
                expected: COMPATIBILITY_NODE_COUNT,
                actual: consumed.len(),
            });
        }
        let mixer_node = nodes[&mixer];
        if mixer_node.color_space != ColorSpace::LinearSrgb {
            return Err(LoweredPlanError::UnsupportedColorSpace(
                mixer_node.color_space,
            ));
        }
        let extent = mixer_node.extent;
        if [master, output]
            .into_iter()
            .any(|node| nodes[&node].extent != extent)
        {
            return Err(LoweredPlanError::ExtentMismatch);
        }

        Ok(Self(Arc::new(LoweredPlanInner {
            revision: plan.revision(),
            extent,
            stages: vec![
                BuiltInRenderStage::FourDeckComposite { decks, mixer },
                BuiltInRenderStage::MasterEffects { node: master },
                BuiltInRenderStage::ProgramOutput { node: output },
            ]
            .into(),
        })))
    }

    pub fn revision(&self) -> GraphRevision {
        self.0.revision
    }

    pub fn extent(&self) -> [u32; 2] {
        self.0.extent
    }

    pub fn stages(&self) -> &[BuiltInRenderStage] {
        &self.0.stages
    }

    pub fn has_program_output(&self) -> bool {
        self.stages()
            .iter()
            .any(|stage| matches!(stage, BuiltInRenderStage::ProgramOutput { .. }))
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LoweredPlanError {
    #[error("built-in renderer does not support graph node {node:?} ({kind})")]
    UnsupportedNode { node: NodeId, kind: String },
    #[error("built-in renderer does not support implicit rate adapter on edge {0}")]
    UnsupportedRateAdapter(usize),
    #[error("expected exactly one {kind} node, found {count}")]
    NodeCardinality { kind: &'static str, count: usize },
    #[error("node {node:?} must be {expected}, found {actual}")]
    WrongNodeKind {
        node: NodeId,
        expected: &'static str,
        actual: String,
    },
    #[error("required connection into {node:?}.{port} is missing")]
    MissingInput { node: NodeId, port: String },
    #[error("required connection {from:?}.{from_port} -> {to:?}.{to_port} is missing")]
    MissingConnection {
        from: NodeId,
        from_port: &'static str,
        to: NodeId,
        to_port: &'static str,
    },
    #[error("lowered plan expected {expected} nodes, found {actual}")]
    UnexpectedNodeCount { expected: usize, actual: usize },
    #[error("built-in renderer requires linear-sRGB composition, found {0:?}")]
    UnsupportedColorSpace(ColorSpace),
    #[error("mixer, master and output nodes must use the same extent")]
    ExtentMismatch,
}

fn unique_node(plan: &RenderPlan, kind: &'static str) -> Result<NodeId, LoweredPlanError> {
    let matches: Vec<_> = plan
        .nodes()
        .iter()
        .filter(|node| node.kind == kind)
        .map(|node| node.id)
        .collect();
    if matches.len() != 1 {
        return Err(LoweredPlanError::NodeCardinality {
            kind,
            count: matches.len(),
        });
    }
    Ok(matches[0])
}

fn input_source(plan: &RenderPlan, node: NodeId, port: &str) -> Result<NodeId, LoweredPlanError> {
    plan.edges()
        .iter()
        .find(|edge| edge.to.node == node && edge.to.port == port)
        .map(|edge| edge.from.node)
        .ok_or_else(|| LoweredPlanError::MissingInput {
            node,
            port: port.to_owned(),
        })
}

fn require_kind(
    nodes: &BTreeMap<NodeId, &oneiroi_graph::CompiledNode>,
    node: NodeId,
    expected: &'static str,
) -> Result<(), LoweredPlanError> {
    let actual = nodes
        .get(&node)
        .map(|node| node.kind.clone())
        .unwrap_or_default();
    if actual != expected {
        return Err(LoweredPlanError::WrongNodeKind {
            node,
            expected,
            actual,
        });
    }
    Ok(())
}

fn require_connection(
    plan: &RenderPlan,
    from: NodeId,
    from_port: &'static str,
    to: NodeId,
    to_port: &'static str,
) -> Result<(), LoweredPlanError> {
    if plan.edges().iter().any(|edge| {
        edge.from.node == from
            && edge.from.port == from_port
            && edge.to.node == to
            && edge.to.port == to_port
    }) {
        Ok(())
    } else {
        Err(LoweredPlanError::MissingConnection {
            from,
            from_port,
            to,
            to_port,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use oneiroi_graph::{
        CompileBudget, FallbackBehavior, GraphCompiler, NodeContract, NodeInstance, PortType,
        ProjectGraph, RateDomain, ResolutionPolicy, builtin_registry, four_deck_performance_graph,
    };

    use super::*;

    #[test]
    fn lowers_the_compatibility_graph_to_three_proven_render_stages() {
        let registry = builtin_registry();
        let plan = GraphCompiler::new(&registry, CompileBudget::default())
            .compile(&four_deck_performance_graph())
            .unwrap();

        let lowered = LoweredRenderPlan::lower(&plan).unwrap();

        assert_eq!(lowered.revision(), GraphRevision(1));
        assert_eq!(lowered.extent(), [1920, 1080]);
        assert_eq!(lowered.stages().len(), 3);
        assert!(matches!(
            lowered.stages()[0],
            BuiltInRenderStage::FourDeckComposite { .. }
        ));
        assert!(lowered.has_program_output());
    }

    #[test]
    fn rejects_a_compiled_node_without_a_built_in_executor() {
        let mut registry = builtin_registry();
        registry
            .register(NodeContract {
                kind: "third_party.generator".to_owned(),
                version: 1,
                domain: RateDomain::VideoFrame,
                ports: vec![oneiroi_graph::PortContract::output(
                    "video",
                    PortType::Texture2d,
                )],
                latency_frames: 0,
                deterministic: true,
                stateful: false,
                realtime_safe: true,
                temporal_break: false,
                quality_levels: Vec::new(),
                fallback: FallbackBehavior::Bypass,
                permissions: BTreeSet::new(),
                resolution: ResolutionPolicy::Inherit,
                color_space: ColorSpace::LinearSrgb,
                estimated_gpu_us: 1,
            })
            .unwrap();
        let mut graph: ProjectGraph = four_deck_performance_graph();
        graph
            .nodes
            .push(NodeInstance::new(100, "third_party.generator"));
        let plan = GraphCompiler::new(&registry, CompileBudget::default())
            .compile(&graph)
            .unwrap();

        assert!(matches!(
            LoweredRenderPlan::lower(&plan),
            Err(LoweredPlanError::UnsupportedNode { .. })
        ));
    }

    #[test]
    fn rejects_a_shared_branch_that_cannot_represent_four_independent_decks() {
        let registry = builtin_registry();
        let mut graph = four_deck_performance_graph();
        graph.edges.retain(|edge| {
            !(edge.to.node == NodeId(20)
                && matches!(edge.to.port.as_str(), "deck_b" | "deck_c" | "deck_d"))
        });
        graph.edges.extend([
            oneiroi_graph::Edge::new(2, "video", 20, "deck_b"),
            oneiroi_graph::Edge::new(2, "video", 20, "deck_c"),
            oneiroi_graph::Edge::new(2, "video", 20, "deck_d"),
        ]);
        let plan = GraphCompiler::new(&registry, CompileBudget::default())
            .compile(&graph)
            .unwrap();

        assert_eq!(
            LoweredRenderPlan::lower(&plan).unwrap_err(),
            LoweredPlanError::UnexpectedNodeCount {
                expected: 11,
                actual: 5,
            }
        );
    }
}
