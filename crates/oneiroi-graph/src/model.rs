use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct NodeId(pub u64);

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct GraphRevision(pub u64);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchemaId(pub String);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    Unitless,
    Normalized,
    Pixels,
    Seconds,
    Hertz,
    Beats,
    Degrees,
    Decibels,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "schema")]
pub enum PortType {
    Texture2d,
    TextureArray,
    TextureCube,
    Mask,
    Depth,
    Normals,
    MotionVectors,
    ObjectIds,
    Geometry,
    PointCloud,
    ParticleBuffer,
    Volume,
    AudioBlock,
    Spectrum,
    Scalar(Units),
    Vec2(Units),
    Vec3(Units),
    Color,
    Event(SchemaId),
    Text,
    CameraPose,
    Skeleton,
}

impl PortType {
    pub fn is_texture_like(&self) -> bool {
        matches!(
            self,
            Self::Texture2d
                | Self::TextureArray
                | Self::TextureCube
                | Self::Mask
                | Self::Depth
                | Self::Normals
                | Self::MotionVectors
                | Self::ObjectIds
                | Self::Volume
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateDomain {
    AudioRealtime,
    Control,
    VideoFrame,
    Event,
    AsyncCpu,
    External,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpace {
    LinearSrgb,
    Srgb,
    DisplayP3,
    Rec709,
    Rec2020,
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolutionPolicy {
    Inherit,
    Fixed([u32; 2]),
    Scale(f32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortContract {
    pub name: String,
    pub direction: PortDirection,
    pub port_type: PortType,
    pub required: bool,
}

impl PortContract {
    pub fn input(name: impl Into<String>, port_type: PortType, required: bool) -> Self {
        Self {
            name: name.into(),
            direction: PortDirection::Input,
            port_type,
            required,
        }
    }

    pub fn output(name: impl Into<String>, port_type: PortType) -> Self {
        Self {
            name: name.into(),
            direction: PortDirection::Output,
            port_type,
            required: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualityLevel {
    pub name: String,
    pub estimated_gpu_us: u32,
    pub resolution_scale: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackBehavior {
    Bypass,
    HoldPrevious,
    Transparent,
    Black,
    FailTransaction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeContract {
    pub kind: String,
    pub version: u32,
    pub domain: RateDomain,
    pub ports: Vec<PortContract>,
    pub latency_frames: u32,
    pub deterministic: bool,
    pub stateful: bool,
    pub realtime_safe: bool,
    /// True only for explicit delay/state nodes whose output represents prior state.
    pub temporal_break: bool,
    pub quality_levels: Vec<QualityLevel>,
    pub fallback: FallbackBehavior,
    pub permissions: BTreeSet<String>,
    pub resolution: ResolutionPolicy,
    pub color_space: ColorSpace,
    pub estimated_gpu_us: u32,
}

impl NodeContract {
    pub fn port(&self, name: &str, direction: PortDirection) -> Option<&PortContract> {
        self.ports
            .iter()
            .find(|port| port.name == name && port.direction == direction)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum ParameterValue {
    Bool(bool),
    Integer(i64),
    Scalar(f64),
    Text(String),
    Color([f32; 4]),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeInstance {
    pub id: NodeId,
    pub kind: String,
    pub contract_version: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParameterValue>,
}

impl NodeInstance {
    pub fn new(id: u64, kind: impl Into<String>) -> Self {
        Self {
            id: NodeId(id),
            kind: kind.into(),
            contract_version: 1,
            label: String::new(),
            parameters: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PortRef {
    pub node: NodeId,
    pub port: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: PortRef,
    pub to: PortRef,
}

impl Edge {
    pub fn new(from_node: u64, from_port: &str, to_node: u64, to_port: &str) -> Self {
        Self {
            from: PortRef {
                node: NodeId(from_node),
                port: from_port.to_owned(),
            },
            to: PortRef {
                node: NodeId(to_node),
                port: to_port.to_owned(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectGraph {
    pub revision: GraphRevision,
    pub nodes: Vec<NodeInstance>,
    pub edges: Vec<Edge>,
}

impl Default for ProjectGraph {
    fn default() -> Self {
        Self {
            revision: GraphRevision(1),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NodeRegistry {
    contracts: BTreeMap<(String, u32), NodeContract>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RegistryError {
    #[error("node contract {kind}@{version} is already registered")]
    Duplicate { kind: String, version: u32 },
}

impl NodeRegistry {
    pub fn register(&mut self, contract: NodeContract) -> Result<(), RegistryError> {
        let key = (contract.kind.clone(), contract.version);
        if self.contracts.contains_key(&key) {
            return Err(RegistryError::Duplicate {
                kind: key.0,
                version: key.1,
            });
        }
        self.contracts.insert(key, contract);
        Ok(())
    }

    pub fn get(&self, kind: &str, version: u32) -> Option<&NodeContract> {
        self.contracts.get(&(kind.to_owned(), version))
    }

    pub fn len(&self) -> usize {
        self.contracts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contracts.is_empty()
    }
}
