//! Distribution metadata for distributed tensor operations.
//!
//! Supports Monarch-inspired single-controller semantics with:
//! - N-dimensional mesh topology
//! - Tensor sharding specifications
//! - Collective operations
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                     DISTRIBUTION METADATA                                │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │  MeshTopology:                                                          │
//! │    dims: [("hosts", 32), ("gpus", 8)]  // 256 total devices             │
//! │                                                                         │
//! │  ShardingSpec per tensor:                                               │
//! │    tensor_dims: [None, Some("gpus")]  // Shard dim 1 across GPUs        │
//! │                                                                         │
//! │  CollectiveOps:                                                         │
//! │    AllReduce, AllGather, ReduceScatter, Broadcast                       │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::device::ValueId;

/// Mesh dimension identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeshDim(pub String);

impl MeshDim {
    /// Creates a new mesh dimension.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the dimension name.
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// N-dimensional mesh topology.
///
/// Defines the shape of the device mesh for distributed execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshTopology {
    /// Dimensions with names and sizes: [("hosts", 32), ("gpus", 8)].
    pub dims: Vec<(String, usize)>,
}

impl MeshTopology {
    /// Creates a new mesh topology.
    pub fn new(dims: Vec<(impl Into<String>, usize)>) -> Self {
        Self {
            dims: dims.into_iter().map(|(n, s)| (n.into(), s)).collect(),
        }
    }

    /// Creates a simple 1D mesh.
    pub fn flat(size: usize) -> Self {
        Self::new(vec![("devices", size)])
    }

    /// Creates a 2D mesh (hosts × gpus).
    pub fn grid(hosts: usize, gpus_per_host: usize) -> Self {
        Self::new(vec![("hosts", hosts), ("gpus", gpus_per_host)])
    }

    /// Returns total number of devices.
    pub fn total_size(&self) -> usize {
        self.dims.iter().map(|(_, s)| s).product()
    }

    /// Returns the size of a dimension.
    pub fn dim_size(&self, name: &str) -> Option<usize> {
        self.dims
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| *s)
    }

    /// Returns dimension names.
    pub fn dim_names(&self) -> impl Iterator<Item = &str> {
        self.dims.iter().map(|(n, _)| n.as_str())
    }

    /// Validates a mesh slice range.
    pub fn validate_slice(&self, dim: &str, start: usize, end: usize) -> Result<(), String> {
        if let Some(size) = self.dim_size(dim) {
            if end > size {
                return Err(format!(
                    "Mesh slice out of bounds: {}[{}..{}] exceeds size {}",
                    dim, start, end, size
                ));
            }
            Ok(())
        } else {
            Err(format!("Unknown mesh dimension: {}", dim))
        }
    }
}

impl Default for MeshTopology {
    fn default() -> Self {
        Self::flat(1)
    }
}

/// Sharding specification for a tensor.
///
/// Maps tensor dimensions to mesh dimensions for distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardingSpec {
    /// For each tensor dimension, which mesh dimension to shard across (if any).
    /// `None` = replicated, `Some("gpus")` = sharded across gpus dimension.
    pub tensor_dims: Vec<Option<MeshDim>>,
}

impl ShardingSpec {
    /// Creates a new sharding spec.
    pub fn new(tensor_dims: Vec<Option<MeshDim>>) -> Self {
        Self { tensor_dims }
    }

    /// Creates a fully replicated sharding (no sharding).
    pub fn replicated(ndim: usize) -> Self {
        Self {
            tensor_dims: vec![None; ndim],
        }
    }

    /// Creates a sharding that shards only the specified dimension.
    pub fn shard_dim(ndim: usize, dim: usize, mesh_dim: impl Into<String>) -> Self {
        let mut tensor_dims = vec![None; ndim];
        if dim < ndim {
            tensor_dims[dim] = Some(MeshDim::new(mesh_dim));
        }
        Self { tensor_dims }
    }

    /// Returns true if tensor is fully replicated (no sharding).
    pub fn is_replicated(&self) -> bool {
        self.tensor_dims.iter().all(|d| d.is_none())
    }

    /// Returns the sharded dimensions.
    pub fn sharded_dims(&self) -> impl Iterator<Item = (usize, &MeshDim)> {
        self.tensor_dims
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.as_ref().map(|m| (i, m)))
    }
}

/// Reduction operation for collective operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReduceOp {
    /// Sum reduction.
    Sum,
    /// Product reduction.
    Prod,
    /// Minimum reduction.
    Min,
    /// Maximum reduction.
    Max,
    /// Average (sum / count).
    Avg,
}

/// Collective operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectiveOp {
    /// All-reduce: reduce across all devices, result on all devices.
    AllReduce {
        /// The tensor value to reduce.
        value: ValueId,
        /// The reduction operation (Sum, Prod, Min, Max, Avg).
        op: ReduceOp,
        /// Mesh dimensions to reduce across (empty = all).
        mesh_dims: Vec<MeshDim>,
    },
    /// All-gather: gather from all devices, result on all devices.
    AllGather {
        /// The tensor value to gather.
        value: ValueId,
        /// The tensor dimension to gather along.
        tensor_dim: usize,
        /// Mesh dimensions to gather across.
        mesh_dims: Vec<MeshDim>,
    },
    /// Reduce-scatter: reduce and scatter across devices.
    ReduceScatter {
        /// The tensor value to reduce and scatter.
        value: ValueId,
        /// The reduction operation.
        op: ReduceOp,
        /// The tensor dimension to scatter along.
        tensor_dim: usize,
        /// Mesh dimensions for the operation.
        mesh_dims: Vec<MeshDim>,
    },
    /// Broadcast: send from one device to all.
    Broadcast {
        /// The tensor value to broadcast.
        value: ValueId,
        /// The source rank for the broadcast.
        source_rank: usize,
        /// Mesh dimensions to broadcast across.
        mesh_dims: Vec<MeshDim>,
    },
    /// Scatter: split tensor and send to different devices.
    Scatter {
        /// The tensor value to scatter.
        value: ValueId,
        /// The tensor dimension to split along.
        tensor_dim: usize,
        /// The source rank that has the full tensor.
        source_rank: usize,
        /// Mesh dimensions to scatter across.
        mesh_dims: Vec<MeshDim>,
    },
    /// Gather: collect tensor slices from devices to one.
    Gather {
        /// The tensor value to gather.
        value: ValueId,
        /// The tensor dimension to concatenate along.
        tensor_dim: usize,
        /// The destination rank that receives the full tensor.
        dest_rank: usize,
        /// Mesh dimensions to gather from.
        mesh_dims: Vec<MeshDim>,
    },
    /// Point-to-point send.
    Send {
        /// The tensor value to send.
        value: ValueId,
        /// The destination rank.
        dest_rank: usize,
    },
    /// Point-to-point receive.
    Recv {
        /// The destination value ID to receive into.
        dest: ValueId,
        /// The source rank to receive from.
        source_rank: usize,
    },
    /// Barrier synchronization.
    Barrier {
        /// Mesh dimensions to synchronize across.
        mesh_dims: Vec<MeshDim>,
    },
}

impl CollectiveOp {
    /// Creates an all-reduce operation across all mesh dimensions.
    pub fn all_reduce(value: ValueId, op: ReduceOp) -> Self {
        Self::AllReduce {
            value,
            op,
            mesh_dims: vec![],
        }
    }

    /// Creates an all-reduce across specific mesh dimensions.
    pub fn all_reduce_on(
        value: ValueId,
        op: ReduceOp,
        mesh_dims: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::AllReduce {
            value,
            op,
            mesh_dims: mesh_dims.into_iter().map(|s| MeshDim::new(s)).collect(),
        }
    }

    /// Creates an all-gather operation.
    pub fn all_gather(value: ValueId, tensor_dim: usize) -> Self {
        Self::AllGather {
            value,
            tensor_dim,
            mesh_dims: vec![],
        }
    }

    /// Creates a broadcast from rank 0.
    pub fn broadcast(value: ValueId) -> Self {
        Self::Broadcast {
            value,
            source_rank: 0,
            mesh_dims: vec![],
        }
    }
}

/// Distribution metadata for a VBC module.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DistributionMetadata {
    /// Mesh topology (if distributed).
    pub mesh: Option<MeshTopology>,
    /// Sharding specifications per tensor value.
    pub sharding: HashMap<ValueId, ShardingSpec>,
    /// Collective operations in execution order.
    pub collectives: Vec<CollectiveOp>,
    /// Process group definitions (for hierarchical communication).
    pub process_groups: Vec<ProcessGroup>,
}

/// Process group for hierarchical communication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessGroup {
    /// Group name.
    pub name: String,
    /// Mesh dimensions included in this group.
    pub mesh_dims: Vec<MeshDim>,
    /// Communication backend (NCCL, Gloo, MPI).
    pub backend: CommBackend,
}

/// Communication backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommBackend {
    /// NVIDIA Collective Communication Library.
    NCCL,
    /// Facebook Gloo library.
    Gloo,
    /// Message Passing Interface.
    MPI,
    /// Raw TCP sockets.
    TCP,
    /// RDMA (InfiniBand, RoCE).
    RDMA,
}

impl DistributionMetadata {
    /// Creates empty distribution metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates distribution metadata with a mesh topology.
    pub fn with_mesh(mesh: MeshTopology) -> Self {
        Self {
            mesh: Some(mesh),
            ..Self::default()
        }
    }

    /// Sets the mesh topology.
    pub fn set_mesh(&mut self, mesh: MeshTopology) {
        self.mesh = Some(mesh);
    }

    /// Adds a sharding specification for a tensor.
    pub fn add_sharding(&mut self, value: ValueId, spec: ShardingSpec) {
        self.sharding.insert(value, spec);
    }

    /// Gets the sharding specification for a tensor.
    pub fn get_sharding(&self, value: ValueId) -> Option<&ShardingSpec> {
        self.sharding.get(&value)
    }

    /// Adds a collective operation.
    pub fn add_collective(&mut self, op: CollectiveOp) {
        self.collectives.push(op);
    }

    /// Returns true if this is a distributed module.
    pub fn is_distributed(&self) -> bool {
        self.mesh
            .as_ref()
            .map(|m| m.total_size() > 1)
            .unwrap_or(false)
    }

    /// Adds a process group.
    pub fn add_process_group(&mut self, group: ProcessGroup) {
        self.process_groups.push(group);
    }

    /// Returns true if this metadata is empty (no mesh, sharding, collectives, or process groups).
    pub fn is_empty(&self) -> bool {
        self.mesh.is_none()
            && self.sharding.is_empty()
            && self.collectives.is_empty()
            && self.process_groups.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mesh_topology() {
        let mesh = MeshTopology::grid(32, 8);
        assert_eq!(mesh.total_size(), 256);
        assert_eq!(mesh.dim_size("hosts"), Some(32));
        assert_eq!(mesh.dim_size("gpus"), Some(8));
        assert!(mesh.validate_slice("hosts", 0, 32).is_ok());
        assert!(mesh.validate_slice("hosts", 0, 64).is_err());
    }

    #[test]
    fn test_sharding_spec() {
        // Fully replicated
        let spec = ShardingSpec::replicated(3);
        assert!(spec.is_replicated());

        // Shard dimension 1 across gpus
        let spec = ShardingSpec::shard_dim(3, 1, "gpus");
        assert!(!spec.is_replicated());
        let sharded: Vec<_> = spec.sharded_dims().collect();
        assert_eq!(sharded.len(), 1);
        assert_eq!(sharded[0], (1, &MeshDim::new("gpus")));
    }

    #[test]
    fn test_distribution_metadata() {
        let mut dist = DistributionMetadata::with_mesh(MeshTopology::grid(4, 8));
        assert!(dist.is_distributed());

        dist.add_sharding(ValueId(0), ShardingSpec::shard_dim(2, 0, "hosts"));
        dist.add_collective(CollectiveOp::all_reduce(ValueId(1), ReduceOp::Sum));

        assert_eq!(dist.collectives.len(), 1);
        assert!(dist.get_sharding(ValueId(0)).is_some());
    }
}
