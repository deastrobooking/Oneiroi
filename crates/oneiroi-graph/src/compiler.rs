use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use thiserror::Error;

use crate::{
    ColorSpace, Edge, GraphRevision, NodeContract, NodeId, NodeRegistry, PortDirection, PortRef,
    PortType, ProjectGraph, RateDomain, ResolutionPolicy,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompileBudget {
    pub composition_extent: [u32; 2],
    pub maximum_gpu_us: u64,
    pub maximum_texture_bytes: u64,
}

impl Default for CompileBudget {
    fn default() -> Self {
        Self {
            composition_extent: [1920, 1080],
            maximum_gpu_us: 16_000,
            maximum_texture_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImplicitRateAdapter {
    pub edge_index: usize,
    pub from: RateDomain,
    pub to: RateDomain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceAllocation {
    pub producer: NodeId,
    pub port: String,
    pub slot: u32,
    pub first_pass: usize,
    pub last_pass: usize,
    pub estimated_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledNode {
    pub id: NodeId,
    pub kind: String,
    pub domain: RateDomain,
    pub pass_index: usize,
    pub extent: [u32; 2],
    pub color_space: ColorSpace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledEdge {
    pub from: PortRef,
    pub to: PortRef,
}

#[derive(Clone, Debug)]
struct RenderPlanInner {
    revision: GraphRevision,
    nodes: Arc<[CompiledNode]>,
    edges: Arc<[CompiledEdge]>,
    rate_adapters: Arc<[ImplicitRateAdapter]>,
    resources: Arc<[ResourceAllocation]>,
    estimated_gpu_us: u64,
    estimated_texture_bytes: u64,
}

/// An immutable, shareable result. A live renderer can retain this while a
/// shadow graph is edited and compiled.
#[derive(Clone, Debug)]
pub struct RenderPlan(Arc<RenderPlanInner>);

impl RenderPlan {
    pub fn revision(&self) -> GraphRevision {
        self.0.revision
    }

    pub fn nodes(&self) -> &[CompiledNode] {
        &self.0.nodes
    }

    pub fn rate_adapters(&self) -> &[ImplicitRateAdapter] {
        &self.0.rate_adapters
    }

    pub fn edges(&self) -> &[CompiledEdge] {
        &self.0.edges
    }

    pub fn resources(&self) -> &[ResourceAllocation] {
        &self.0.resources
    }

    pub fn estimated_gpu_us(&self) -> u64 {
        self.0.estimated_gpu_us
    }

    pub fn estimated_texture_bytes(&self) -> u64 {
        self.0.estimated_texture_bytes
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CompileError {
    #[error("composition extent {0:?} is invalid")]
    InvalidCompositionExtent([u32; 2]),
    #[error("graph contains duplicate node id {0:?}")]
    DuplicateNode(NodeId),
    #[error("node {node:?} references unknown contract {kind}@{version}")]
    UnknownContract {
        node: NodeId,
        kind: String,
        version: u32,
    },
    #[error("node {node:?} requires unavailable permission {permission}")]
    MissingPermission { node: NodeId, permission: String },
    #[error("node {node:?} declares an invalid resolution policy")]
    InvalidResolution { node: NodeId },
    #[error("edge {edge} references missing {direction} port {node:?}.{port}")]
    MissingPort {
        edge: usize,
        direction: &'static str,
        node: NodeId,
        port: String,
    },
    #[error("edge {edge} connects incompatible types {output:?} and {input:?}")]
    TypeMismatch {
        edge: usize,
        output: PortType,
        input: PortType,
    },
    #[error("input {node:?}.{port} has more than one connection")]
    DuplicateInput { node: NodeId, port: String },
    #[error("required input {node:?}.{port} is not connected")]
    MissingRequiredInput { node: NodeId, port: String },
    #[error("graph cycle requires an explicit delay or state node")]
    CycleWithoutDelay,
    #[error("estimated GPU work {estimated_us} us exceeds the {budget_us} us show budget")]
    GpuBudget { estimated_us: u64, budget_us: u64 },
    #[error(
        "estimated transient texture use {estimated_bytes} bytes exceeds the {budget_bytes} byte budget"
    )]
    TextureBudget {
        estimated_bytes: u64,
        budget_bytes: u64,
    },
}

pub struct GraphCompiler<'a> {
    registry: &'a NodeRegistry,
    budget: CompileBudget,
    granted_permissions: BTreeSet<String>,
}

impl<'a> GraphCompiler<'a> {
    pub fn new(registry: &'a NodeRegistry, budget: CompileBudget) -> Self {
        Self {
            registry,
            budget,
            granted_permissions: BTreeSet::new(),
        }
    }

    pub fn with_permissions(
        mut self,
        permissions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.granted_permissions = permissions.into_iter().map(Into::into).collect();
        self
    }

    pub fn compile(&self, graph: &ProjectGraph) -> Result<RenderPlan, CompileError> {
        if self.budget.composition_extent.contains(&0) {
            return Err(CompileError::InvalidCompositionExtent(
                self.budget.composition_extent,
            ));
        }
        let mut instances = BTreeMap::new();
        let mut contracts = BTreeMap::new();
        for node in &graph.nodes {
            if instances.insert(node.id, node).is_some() {
                return Err(CompileError::DuplicateNode(node.id));
            }
            let contract = self
                .registry
                .get(&node.kind, node.contract_version)
                .ok_or_else(|| CompileError::UnknownContract {
                    node: node.id,
                    kind: node.kind.clone(),
                    version: node.contract_version,
                })?;
            if let Some(permission) = contract
                .permissions
                .difference(&self.granted_permissions)
                .next()
            {
                return Err(CompileError::MissingPermission {
                    node: node.id,
                    permission: permission.clone(),
                });
            }
            if !valid_resolution(contract.resolution) {
                return Err(CompileError::InvalidResolution { node: node.id });
            }
            contracts.insert(node.id, contract);
        }

        let mut connected_inputs = BTreeSet::new();
        let mut adapters = Vec::new();
        for (edge_index, edge) in graph.edges.iter().enumerate() {
            let output = port_for(edge, edge_index, true, &contracts)?;
            let input = port_for(edge, edge_index, false, &contracts)?;
            if output.port_type != input.port_type {
                return Err(CompileError::TypeMismatch {
                    edge: edge_index,
                    output: output.port_type.clone(),
                    input: input.port_type.clone(),
                });
            }
            if !connected_inputs.insert((edge.to.node, edge.to.port.clone())) {
                return Err(CompileError::DuplicateInput {
                    node: edge.to.node,
                    port: edge.to.port.clone(),
                });
            }
            let from = contracts[&edge.from.node].domain;
            let to = contracts[&edge.to.node].domain;
            if from != to {
                adapters.push(ImplicitRateAdapter {
                    edge_index,
                    from,
                    to,
                });
            }
        }

        for (node, contract) in &contracts {
            for port in contract
                .ports
                .iter()
                .filter(|port| port.direction == PortDirection::Input && port.required)
            {
                if !connected_inputs.contains(&(*node, port.name.clone())) {
                    return Err(CompileError::MissingRequiredInput {
                        node: *node,
                        port: port.name.clone(),
                    });
                }
            }
        }

        let order = topological_order(graph, &contracts)?;
        let pass_by_node: BTreeMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(pass, node)| (*node, pass))
            .collect();
        let mut nodes = Vec::with_capacity(order.len());
        let mut estimated_gpu_us = 0_u64;
        for (pass_index, node_id) in order.iter().enumerate() {
            let instance = instances[node_id];
            let contract = contracts[node_id];
            estimated_gpu_us =
                estimated_gpu_us.saturating_add(u64::from(contract.estimated_gpu_us));
            nodes.push(CompiledNode {
                id: *node_id,
                kind: instance.kind.clone(),
                domain: contract.domain,
                pass_index,
                extent: resolve_extent(contract.resolution, self.budget.composition_extent),
                color_space: contract.color_space,
            });
        }
        if estimated_gpu_us > self.budget.maximum_gpu_us {
            return Err(CompileError::GpuBudget {
                estimated_us: estimated_gpu_us,
                budget_us: self.budget.maximum_gpu_us,
            });
        }

        let resources = allocate_resources(
            graph,
            &contracts,
            &pass_by_node,
            self.budget.composition_extent,
        );
        let estimated_texture_bytes = peak_texture_bytes(&resources);
        if estimated_texture_bytes > self.budget.maximum_texture_bytes {
            return Err(CompileError::TextureBudget {
                estimated_bytes: estimated_texture_bytes,
                budget_bytes: self.budget.maximum_texture_bytes,
            });
        }

        Ok(RenderPlan(Arc::new(RenderPlanInner {
            revision: graph.revision,
            nodes: nodes.into(),
            edges: graph
                .edges
                .iter()
                .map(|edge| CompiledEdge {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                })
                .collect::<Vec<_>>()
                .into(),
            rate_adapters: adapters.into(),
            resources: resources.into(),
            estimated_gpu_us,
            estimated_texture_bytes,
        })))
    }
}

fn valid_resolution(policy: ResolutionPolicy) -> bool {
    match policy {
        ResolutionPolicy::Inherit => true,
        ResolutionPolicy::Fixed(extent) => !extent.contains(&0),
        ResolutionPolicy::Scale(scale) => scale.is_finite() && scale > 0.0 && scale <= 4.0,
    }
}

fn port_for<'a>(
    edge: &Edge,
    edge_index: usize,
    output: bool,
    contracts: &'a BTreeMap<NodeId, &'a NodeContract>,
) -> Result<&'a crate::PortContract, CompileError> {
    let endpoint = if output { &edge.from } else { &edge.to };
    let direction = if output {
        PortDirection::Output
    } else {
        PortDirection::Input
    };
    contracts
        .get(&endpoint.node)
        .and_then(|contract| contract.port(&endpoint.port, direction))
        .ok_or_else(|| CompileError::MissingPort {
            edge: edge_index,
            direction: if output { "output" } else { "input" },
            node: endpoint.node,
            port: endpoint.port.clone(),
        })
}

fn topological_order(
    graph: &ProjectGraph,
    contracts: &BTreeMap<NodeId, &NodeContract>,
) -> Result<Vec<NodeId>, CompileError> {
    let mut incoming: BTreeMap<NodeId, usize> = contracts.keys().map(|node| (*node, 0)).collect();
    let mut outgoing: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for edge in &graph.edges {
        // The output of an explicit delay/state node belongs to an earlier
        // tick, so it does not create a same-tick scheduling dependency.
        if contracts
            .get(&edge.from.node)
            .is_some_and(|contract| contract.temporal_break)
        {
            continue;
        }
        *incoming.entry(edge.to.node).or_default() += 1;
        outgoing
            .entry(edge.from.node)
            .or_default()
            .push(edge.to.node);
    }
    let mut ready: BTreeSet<_> = incoming
        .iter()
        .filter_map(|(node, count)| (*count == 0).then_some(*node))
        .collect();
    let mut order = Vec::with_capacity(incoming.len());
    while let Some(node) = ready.pop_first() {
        order.push(node);
        for destination in outgoing.get(&node).into_iter().flatten() {
            let count = incoming
                .get_mut(destination)
                .expect("validated destination is present");
            *count -= 1;
            if *count == 0 {
                ready.insert(*destination);
            }
        }
    }
    (order.len() == incoming.len())
        .then_some(order)
        .ok_or(CompileError::CycleWithoutDelay)
}

fn resolve_extent(policy: ResolutionPolicy, composition: [u32; 2]) -> [u32; 2] {
    match policy {
        ResolutionPolicy::Inherit => composition,
        ResolutionPolicy::Fixed(extent) => extent,
        ResolutionPolicy::Scale(scale) => [
            ((composition[0] as f32 * scale).round() as u32).max(1),
            ((composition[1] as f32 * scale).round() as u32).max(1),
        ],
    }
}

fn allocate_resources(
    graph: &ProjectGraph,
    contracts: &BTreeMap<NodeId, &NodeContract>,
    passes: &BTreeMap<NodeId, usize>,
    composition: [u32; 2],
) -> Vec<ResourceAllocation> {
    let mut lifetimes = Vec::new();
    for (node, contract) in contracts {
        let first = passes[node];
        for port in contract.ports.iter().filter(|port| {
            port.direction == PortDirection::Output && port.port_type.is_texture_like()
        }) {
            let last = graph
                .edges
                .iter()
                .filter(|edge| edge.from.node == *node && edge.from.port == port.name)
                .filter_map(|edge| passes.get(&edge.to.node).copied())
                .max()
                .unwrap_or(first);
            let extent = resolve_extent(contract.resolution, composition);
            let bytes_per_pixel = match port.port_type {
                PortType::Depth => 4,
                PortType::MotionVectors | PortType::Normals => 8,
                _ => 8, // conservative RGBA16F working representation
            };
            lifetimes.push((
                *node,
                port.name.clone(),
                first,
                last,
                u64::from(extent[0]) * u64::from(extent[1]) * bytes_per_pixel,
            ));
        }
    }
    lifetimes.sort_by_key(|item| (item.2, item.0, item.1.clone()));
    let mut slots: Vec<(usize, u64)> = Vec::new();
    let mut allocations = Vec::new();
    for (producer, port, first, last, bytes) in lifetimes {
        let reusable = slots
            .iter()
            .enumerate()
            .find(|(_, (previous_last, capacity))| *previous_last < first && *capacity >= bytes)
            .map(|(index, _)| index);
        let slot = if let Some(slot) = reusable {
            slots[slot] = (last, slots[slot].1);
            slot
        } else {
            slots.push((last, bytes));
            slots.len() - 1
        };
        allocations.push(ResourceAllocation {
            producer,
            port,
            slot: slot as u32,
            first_pass: first,
            last_pass: last,
            estimated_bytes: bytes,
        });
    }
    allocations
}

fn peak_texture_bytes(resources: &[ResourceAllocation]) -> u64 {
    let last_pass = resources
        .iter()
        .map(|resource| resource.last_pass)
        .max()
        .unwrap_or(0);
    (0..=last_pass)
        .map(|pass| {
            resources
                .iter()
                .filter(|resource| resource.first_pass <= pass && pass <= resource.last_pass)
                .map(|resource| resource.estimated_bytes)
                .sum()
        })
        .max()
        .unwrap_or(0)
}
