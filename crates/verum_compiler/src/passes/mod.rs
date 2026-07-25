//! Compiler Optimization Passes
//!
//! This module contains specialized optimization passes that run during
//! the compilation pipeline.

pub mod cbgr_integration;

pub use cbgr_integration::{
    CbgrOptimizationPass, CbgrPassConfig, CbgrPassStatistics, ModulePassResult, PassResult,
};
