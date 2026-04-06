//! Meta Context - Compile-time execution environment
//!
//! This module provides the execution context for meta functions,
//! including variable bindings and constant value representation.
//!
//! ## Module Structure
//!
//! - [`error`] - Meta execution errors
//! - [`types`] - Core type definitions (TypeDefinition, ProtocolImplementation)
//! - [`reflection`] - Type introspection API
//! - [`ir`] - Meta expression/statement intermediate representation
//! - [`subsystems`] - Context subsystems (runtime, assets, macro state, etc.)
//! - [`builtins`] - Builtin meta functions (reflection, arithmetic, collections, etc.)
//! - [`context`] - MetaContext execution environment
//! - [`evaluator`] - Meta expression/statement evaluation
//! - [`async_executor`] - Parallel meta task execution
//! - [`linter`] - Meta code linting
//! - [`registry`] - Macro and meta function registry
//! - [`sandbox`] - Safe meta execution sandbox
//! - [`value_ops`] - MetaValue arithmetic/comparison operations
//!
//! ## Type Properties
//!
//! Verum uses **Type Properties** for compile-time type introspection:
//!
//! | Property      | Description                    |
//! |---------------|--------------------------------|
//! | `T.size`      | Memory size in bytes           |
//! | `T.alignment` | Memory alignment requirement   |
//! | `T.stride`    | Array element stride           |
//! | `T.bits`      | Size in bits                   |
//! | `T.min`       | Minimum value (numeric types)  |
//! | `T.max`       | Maximum value (numeric types)  |
//! | `T.name`      | Type name as Text              |
//! | `T.id`        | Unique type identifier (u64)   |
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

// Core modules
pub mod cache;
pub mod error;
pub mod ir;
pub mod metrics;
pub mod reflection;
pub mod subsystems;
pub mod types;

// Sub-context modules (Phase 1 refactoring)
pub mod contexts;

// Phase 1 refactored modules
pub mod builtins;
pub mod context;
pub mod evaluator;

// Phase 2 consolidated modules (formerly meta_*.rs)
pub mod async_executor;
pub mod linter;
pub mod registry;
pub mod sandbox;
pub mod value_ops;

// VBC execution for staged metaprogramming
pub mod vbc_executor;

// Re-export main types for convenient access
pub use error::MetaError;
pub use types::{ProtocolImplementation, TypeDefinition};

// Re-export reflection types
pub use reflection::{
    AssociatedTypeInfo, FieldInfo, FieldOffset, FunctionInfo, GenericParam, GenericParamKind,
    LifetimeParam, MethodResolution, MethodSource, OwnershipInfo, ParamInfo, PrimitiveType,
    ProtocolInfo, SelfKind, TraitBound, TypeInfo, TypeKind, VariantInfo, VariantKind, Visibility,
};

// Re-export IR types
pub use ir::{MetaArm, MetaExpr, MetaPattern, MetaStmt, MetaType};

// Re-export subsystem types
pub use subsystems::{
    AssetMetadata, BenchResult, BuildAssetsInfo, CacheStats, CodeSearchTypeInfo, ItemInfo,
    ItemKind, MacroStateInfo, ModuleInfo, ProjectInfoData, RuntimeInfo, StageRecord, UsageInfo,
};

// Re-export context types
// Note: ConstValue is deprecated, use MetaValue directly
pub use context::{ConstValue, MetaContext};

// Re-export sub-context types for advanced usage
pub use contexts::{
    BuildConfiguration, DiagnosticsCollector, ExecutionState, SecurityContext, TypeIntrospection,
};
pub use contexts::execution_state::CallFrame;
pub use contexts::type_introspection::TypeAttribute;
pub use contexts::security::ResourceLimits;

// Re-export TypeProperty from verum_ast for convenience
pub use verum_ast::expr::TypeProperty;

// Re-export builtin function type
pub use builtins::BuiltinMetaFn;

// Re-export metrics types
pub use metrics::{BuiltinStats, BuiltinTimingGuard, MetaEvalMetrics, MetricsSummary, RecursionGuard};

// Re-export async executor types
pub use async_executor::{MetaAsyncExecutor, ParallelTaskBuilder, TaskDependencyGraph};

// Re-export registry types
pub use registry::{MacroDefinition, MacroKind, MetaFunction, MetaRegistry};

// Re-export sandbox types
pub use sandbox::{MetaSandbox, Operation as SandboxOperation, SandboxError};

// Re-export value_ops types
pub use value_ops::MetaValueOps;

// Re-export cache types
pub use cache::{MetaCacheConfig, MetaCacheStats, MetaEvalCache};
