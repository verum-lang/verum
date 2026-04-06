//! VBC Tensor Metadata Sections.
//!
//! This module provides compile-time metadata for tensor operations as defined
//! in the tensor-GPU architecture specification. These metadata sections enable:
//!
//! - **Shape verification**: Compile-time shape checking with symbolic dimensions
//! - **Device placement**: Hints for CPU/GPU/TPU execution
//! - **Distribution topology**: Mesh topology and sharding specifications
//! - **MLIR lowering**: Hints for VBC → MLIR optimization passes
//! - **Autodiff graph**: Forward/backward mapping and checkpointing
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                          VBC MODULE (.vbca)                              │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │  Standard Sections:                                                     │
//! │  • functions, constants, types, bytecode, strings                       │
//! │                                                                         │
//! │  Tensor Metadata Sections (NEW):                                        │
//! │  • shape_metadata: Static/symbolic shapes per instruction               │
//! │  • device_hints: Preferred device placement per block                   │
//! │  • distribution: Mesh topology, sharding, collectives                   │
//! │  • mlir_hints: Fusion groups, target optimizations                      │
//! │  • autodiff_graph: Forward→backward mapping, checkpoints                │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use verum_vbc::metadata::{ShapeMetadata, DeviceHints, DistributionMetadata};
//!
//! // Shape metadata for compile-time verification
//! let mut shapes = ShapeMetadata::new();
//! shapes.add_static_shape(instr_id, StaticShape::new(vec![
//!     ShapeDim::Symbolic(batch_size),
//!     ShapeDim::Static(1024),
//! ], DType::F32));
//!
//! // Device placement hints
//! let mut devices = DeviceHints::new();
//! devices.set_placement(block_id, DevicePreference::PreferGPU);
//! ```

pub mod autodiff;
pub mod device;
pub mod distribution;
pub mod mlir;
pub mod shape;

pub use autodiff::{AutodiffGraph, CheckpointBoundary, TapeStructure};
pub use device::{DeviceHints, DevicePreference, DeviceTransfer, DeviceType};
pub use distribution::{
    CollectiveOp, DistributionMetadata, MeshDim, MeshTopology, ReduceOp, ShardingSpec,
};
pub use mlir::{FusionGroup, MlirHints, RegionId, TargetOptHints, TargetOptStrategy};
pub use shape::{InstructionId, ShapeConstraint, ShapeDim, ShapeMetadata, StaticShape, SymbolId};
