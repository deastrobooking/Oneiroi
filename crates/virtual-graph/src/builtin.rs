use std::collections::BTreeSet;

use crate::{
    ColorSpace, Edge, FallbackBehavior, GraphRevision, NodeContract, NodeInstance, NodeRegistry,
    PortContract, PortType, ProjectGraph, RateDomain, ResolutionPolicy,
};

pub const DECK_SOURCE_NODE: &str = "oneiroi.deck_source";
pub const DECK_EFFECTS_NODE: &str = "oneiroi.deck_effects";
pub const FOUR_DECK_MIXER_NODE: &str = "oneiroi.four_deck_mixer";
pub const MASTER_EFFECTS_NODE: &str = "oneiroi.master_effects";
pub const PROGRAM_OUTPUT_NODE: &str = "oneiroi.program_output";
pub const FRAME_DELAY_NODE: &str = "oneiroi.frame_delay";

pub fn builtin_registry() -> NodeRegistry {
    let mut registry = NodeRegistry::default();
    for contract in [
        contract(
            DECK_SOURCE_NODE,
            vec![PortContract::output("video", PortType::Texture2d)],
            500,
        ),
        contract(
            DECK_EFFECTS_NODE,
            vec![
                PortContract::input("video", PortType::Texture2d, true),
                PortContract::output("video", PortType::Texture2d),
            ],
            900,
        ),
        contract(
            FOUR_DECK_MIXER_NODE,
            vec![
                PortContract::input("deck_a", PortType::Texture2d, true),
                PortContract::input("deck_b", PortType::Texture2d, true),
                PortContract::input("deck_c", PortType::Texture2d, true),
                PortContract::input("deck_d", PortType::Texture2d, true),
                PortContract::output("program", PortType::Texture2d),
            ],
            1_200,
        ),
        contract(
            MASTER_EFFECTS_NODE,
            vec![
                PortContract::input("video", PortType::Texture2d, true),
                PortContract::output("video", PortType::Texture2d),
            ],
            1_500,
        ),
        contract(
            PROGRAM_OUTPUT_NODE,
            vec![PortContract::input("video", PortType::Texture2d, true)],
            300,
        ),
        NodeContract {
            temporal_break: true,
            stateful: true,
            latency_frames: 1,
            fallback: FallbackBehavior::HoldPrevious,
            ..contract(
                FRAME_DELAY_NODE,
                vec![
                    PortContract::input("current", PortType::Texture2d, true),
                    PortContract::output("previous", PortType::Texture2d),
                ],
                100,
            )
        },
    ] {
        registry
            .register(contract)
            .expect("built-in contracts have unique identities");
    }
    registry
}

/// Describes the existing renderer without changing its proven execution path.
/// This macro is the compatibility bridge for the Perform view.
pub fn four_deck_performance_graph() -> ProjectGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for deck in 0..4_u64 {
        let source = 1 + deck * 2;
        let effects = source + 1;
        let mut source_node = NodeInstance::new(source, DECK_SOURCE_NODE);
        source_node.label = format!("Deck {} source", (b'A' + deck as u8) as char);
        source_node.parameters.insert(
            "deck_index".to_owned(),
            crate::ParameterValue::Integer(deck as i64),
        );
        let mut effects_node = NodeInstance::new(effects, DECK_EFFECTS_NODE);
        effects_node.label = format!("Deck {} effects", (b'A' + deck as u8) as char);
        nodes.extend([source_node, effects_node]);
        edges.push(Edge::new(source, "video", effects, "video"));
        edges.push(Edge::new(
            effects,
            "video",
            20,
            ["deck_a", "deck_b", "deck_c", "deck_d"][deck as usize],
        ));
    }
    nodes.extend([
        NodeInstance::new(20, FOUR_DECK_MIXER_NODE),
        NodeInstance::new(21, MASTER_EFFECTS_NODE),
        NodeInstance::new(22, PROGRAM_OUTPUT_NODE),
    ]);
    edges.extend([
        Edge::new(20, "program", 21, "video"),
        Edge::new(21, "video", 22, "video"),
    ]);
    ProjectGraph {
        revision: GraphRevision(1),
        nodes,
        edges,
    }
}

fn contract(kind: &str, ports: Vec<PortContract>, estimated_gpu_us: u32) -> NodeContract {
    NodeContract {
        kind: kind.to_owned(),
        version: 1,
        domain: RateDomain::VideoFrame,
        ports,
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
        estimated_gpu_us,
    }
}
