//! Typed, device-neutral audiovisual graphs and transactional compilation.
//!
//! This crate describes work; it never owns a GPU device, media decoder, or
//! operating-system input. A validated [`RenderPlan`] is immutable and safe to
//! hand to a renderer while another graph is edited and compiled.

mod builtin;
mod compiler;
mod model;
mod transaction;

pub use builtin::{builtin_registry, four_deck_performance_graph};
pub use compiler::{
    CompileBudget, CompileError, CompiledNode, GraphCompiler, ImplicitRateAdapter, RenderPlan,
    ResourceAllocation,
};
pub use model::{
    ColorSpace, Edge, FallbackBehavior, GraphRevision, NodeContract, NodeId, NodeInstance,
    NodeRegistry, ParameterValue, PortContract, PortDirection, PortRef, PortType, ProjectGraph,
    QualityLevel, RateDomain, ResolutionPolicy, SchemaId, Units,
};
pub use transaction::{
    CommitPoint, CommitReceipt, GraphTransaction, TimelinePosition, TransactionError,
    TransactionManager, TransactionState,
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn contract(
        kind: &str,
        domain: RateDomain,
        ports: Vec<PortContract>,
        temporal_break: bool,
    ) -> NodeContract {
        NodeContract {
            kind: kind.to_owned(),
            version: 1,
            domain,
            ports,
            latency_frames: u32::from(temporal_break),
            deterministic: true,
            stateful: temporal_break,
            realtime_safe: true,
            temporal_break,
            quality_levels: Vec::new(),
            fallback: FallbackBehavior::Bypass,
            permissions: BTreeSet::new(),
            resolution: ResolutionPolicy::Inherit,
            color_space: ColorSpace::LinearSrgb,
            estimated_gpu_us: 10,
        }
    }

    #[test]
    fn compiles_current_four_deck_macro_into_an_immutable_plan() {
        let registry = builtin_registry();
        let graph = four_deck_performance_graph();
        let plan = GraphCompiler::new(&registry, CompileBudget::default())
            .compile(&graph)
            .unwrap();

        assert_eq!(plan.revision(), GraphRevision(1));
        assert_eq!(plan.nodes().len(), 11);
        assert_eq!(plan.nodes().last().unwrap().kind, "oneiroi.program_output");
        assert!(plan.resources().iter().any(|resource| resource.slot == 0));
        assert_eq!(plan.estimated_gpu_us(), 8_600);
    }

    #[test]
    fn rejects_an_implicit_feedback_cycle() {
        let mut registry = NodeRegistry::default();
        registry
            .register(contract(
                "pass",
                RateDomain::VideoFrame,
                vec![
                    PortContract::input("in", PortType::Texture2d, true),
                    PortContract::output("out", PortType::Texture2d),
                ],
                false,
            ))
            .unwrap();
        let graph = ProjectGraph {
            revision: GraphRevision(1),
            nodes: vec![NodeInstance::new(1, "pass"), NodeInstance::new(2, "pass")],
            edges: vec![Edge::new(1, "out", 2, "in"), Edge::new(2, "out", 1, "in")],
        };

        assert!(matches!(
            GraphCompiler::new(&registry, CompileBudget::default()).compile(&graph),
            Err(CompileError::CycleWithoutDelay)
        ));
    }

    #[test]
    fn explicit_delay_makes_feedback_schedulable() {
        let mut registry = NodeRegistry::default();
        registry
            .register(contract(
                "pass",
                RateDomain::VideoFrame,
                vec![
                    PortContract::input("in", PortType::Texture2d, true),
                    PortContract::output("out", PortType::Texture2d),
                ],
                false,
            ))
            .unwrap();
        registry
            .register(contract(
                "delay",
                RateDomain::VideoFrame,
                vec![
                    PortContract::input("current", PortType::Texture2d, true),
                    PortContract::output("previous", PortType::Texture2d),
                ],
                true,
            ))
            .unwrap();
        let graph = ProjectGraph {
            revision: GraphRevision(1),
            nodes: vec![NodeInstance::new(1, "pass"), NodeInstance::new(2, "delay")],
            edges: vec![
                Edge::new(1, "out", 2, "current"),
                Edge::new(2, "previous", 1, "in"),
            ],
        };

        let plan = GraphCompiler::new(&registry, CompileBudget::default())
            .compile(&graph)
            .unwrap();
        assert_eq!(plan.nodes().len(), 2);
    }

    #[test]
    fn inserts_an_explicit_rate_adapter_in_the_plan() {
        let mut registry = NodeRegistry::default();
        registry
            .register(contract(
                "control",
                RateDomain::Control,
                vec![PortContract::output(
                    "value",
                    PortType::Scalar(Units::Normalized),
                )],
                false,
            ))
            .unwrap();
        registry
            .register(contract(
                "video",
                RateDomain::VideoFrame,
                vec![PortContract::input(
                    "value",
                    PortType::Scalar(Units::Normalized),
                    true,
                )],
                false,
            ))
            .unwrap();
        let graph = ProjectGraph {
            revision: GraphRevision(1),
            nodes: vec![
                NodeInstance::new(1, "control"),
                NodeInstance::new(2, "video"),
            ],
            edges: vec![Edge::new(1, "value", 2, "value")],
        };

        let plan = GraphCompiler::new(&registry, CompileBudget::default())
            .compile(&graph)
            .unwrap();
        assert_eq!(
            plan.rate_adapters(),
            &[ImplicitRateAdapter {
                edge_index: 0,
                from: RateDomain::Control,
                to: RateDomain::VideoFrame,
            }]
        );
    }

    #[test]
    fn node_permissions_must_be_granted_before_compilation() {
        let mut registry = NodeRegistry::default();
        let mut camera = contract(
            "camera",
            RateDomain::External,
            vec![PortContract::output("video", PortType::Texture2d)],
            false,
        );
        camera.permissions.insert("camera.capture".to_owned());
        registry.register(camera).unwrap();
        let graph = ProjectGraph {
            revision: GraphRevision(1),
            nodes: vec![NodeInstance::new(1, "camera")],
            edges: Vec::new(),
        };
        let compiler = GraphCompiler::new(&registry, CompileBudget::default());
        assert!(matches!(
            compiler.compile(&graph),
            Err(CompileError::MissingPermission { .. })
        ));

        GraphCompiler::new(&registry, CompileBudget::default())
            .with_permissions(["camera.capture"])
            .compile(&graph)
            .unwrap();
    }

    #[test]
    fn failed_shadow_compile_never_changes_the_active_plan() {
        let registry = builtin_registry();
        let graph = four_deck_performance_graph();
        let compiler = GraphCompiler::new(&registry, CompileBudget::default());
        let plan = compiler.compile(&graph).unwrap();
        let mut transactions = TransactionManager::new(graph, plan);
        let original_revision = transactions.active_plan().revision();
        transactions.begin().unwrap().edges.clear();

        assert!(matches!(
            transactions.prepare(&compiler),
            Err(TransactionError::Compile(
                CompileError::MissingRequiredInput { .. }
            ))
        ));
        assert_eq!(transactions.active_plan().revision(), original_revision);
    }

    #[test]
    fn prepared_shadow_graph_commits_on_the_next_bar() {
        let registry = builtin_registry();
        let graph = four_deck_performance_graph();
        let compiler = GraphCompiler::new(&registry, CompileBudget::default());
        let plan = compiler.compile(&graph).unwrap();
        let mut transactions = TransactionManager::new(graph, plan);
        transactions.begin().unwrap();
        transactions.prepare(&compiler).unwrap();
        let now = TimelinePosition {
            frame_id: 10,
            beat_ticks: 1_000,
            timecode_frames: None,
        };
        let target = transactions
            .schedule(
                CommitPoint::NextBar {
                    ticks_per_beat: 960,
                    beats_per_bar: 4,
                },
                now,
            )
            .unwrap();

        assert_eq!(target.beat_ticks, 3_840);
        assert!(
            transactions
                .advance(TimelinePosition {
                    beat_ticks: 3_839,
                    ..now
                })
                .is_none()
        );
        let receipt = transactions
            .advance(TimelinePosition {
                frame_id: 200,
                beat_ticks: 3_840,
                timecode_frames: None,
            })
            .unwrap();
        assert_eq!(receipt.revision, GraphRevision(2));
    }
}
