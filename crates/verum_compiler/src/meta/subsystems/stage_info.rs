//! Stage information for N-level staging
//!
//! Provides tracking for staged metaprogramming and code generation provenance.

use verum_ast::Span;
use verum_common::Text;

/// Stage record for tracking generation chain in N-level staging
#[derive(Debug, Clone)]
pub struct StageRecord {
    /// The stage level where code was generated
    pub stage: u32,
    /// The function that generated the code
    pub function: Text,
    /// The source span of the generation site
    pub span: Span,
}

impl StageRecord {
    /// Create a new stage record
    pub fn new(stage: u32, function: Text, span: Span) -> Self {
        Self {
            stage,
            function,
            span,
        }
    }
}

/// Benchmark result for MetaBench context
#[derive(Debug, Clone)]
pub struct BenchResult {
    /// Duration in nanoseconds
    pub duration_ns: u64,
    /// Optional context description
    pub context: Option<Text>,
}

impl BenchResult {
    /// Create a new benchmark result
    pub fn new(duration_ns: u64) -> Self {
        Self {
            duration_ns,
            context: None,
        }
    }

    /// Create a benchmark result with context
    pub fn with_context(duration_ns: u64, context: Text) -> Self {
        Self {
            duration_ns,
            context: Some(context),
        }
    }
}
