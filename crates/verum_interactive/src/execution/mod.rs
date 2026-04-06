//! Execution module for the Verum Playground.
//!
//! This module provides the bridge between parsed Verum code and the VBC interpreter,
//! enabling actual code execution in the playbook environment.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                    EXECUTION PIPELINE                               │
//! ├─────────────────────────────────────────────────────────────────────┤
//! │  Source → Parse → Codegen → VbcModule → Interpreter → Value        │
//! │                                                                     │
//! │  Components:                                                        │
//! │  - ExecutionPipeline: Orchestrates parse → execute flow             │
//! │  - ExecutionContext:  Preserves state across cells                  │
//! │  - ValueFormatter:    Renders Values for display                    │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```

mod async_exec;
mod context;
mod pipeline;
pub mod value_format;

pub use async_exec::{
    AsyncExecutor, ExecutionHandle, ExecutionMessage, ExecutionStatus,
    StreamingOutput, OutputLine, ProgressDisplay, ProgressStyle,
};
pub use context::{BindingInfo, ExecutionContext, FunctionInfo as ExecFunctionInfo};
pub use pipeline::{CompiledCell, ExecutionError, ExecutionPipeline, ExecutionResult};
pub use value_format::{format_value, format_value_with_type, ValueDisplayOptions};
