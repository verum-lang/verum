//! Subsystems for meta context
//!
//! This module provides various subsystem components that support
//! meta function execution.
//!
//! ## Module Structure
//!
//! - [`runtime_info`] - Compile-time and build-time information
//! - [`build_assets`] - File access during meta execution
//! - [`macro_state`] - Caching and invocation tracking
//! - [`stage_info`] - N-level staging support
//! - [`code_search`] - Type and usage tracking
//! - [`project_info`] - Project metadata from Verum.toml
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

pub mod build_assets;
pub mod code_search;
pub mod macro_state;
pub mod project_info;
pub mod runtime_info;
pub mod stage_info;

// Re-export main types
pub use build_assets::{AssetMetadata, BuildAssetsInfo};
pub use code_search::{CodeSearchTypeInfo, ItemInfo, ItemKind, ModuleInfo, UsageInfo};
pub use macro_state::{CacheStats, MacroStateInfo};
pub use project_info::ProjectInfoData;
pub use runtime_info::RuntimeInfo;
pub use stage_info::{BenchResult, StageRecord};
