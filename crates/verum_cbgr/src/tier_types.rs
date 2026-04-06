//! Unified Tier Types for CBGR Analysis
//!
//! This module provides the canonical type definitions for reference tiers
//! used throughout the compilation pipeline.
//!
//! # Architecture
//!
//! ```text
//! verum_cbgr::tier_types (this module)
//!         │
//!         ├── ReferenceTier      ← Tier enum with detailed reasons
//!         ├── Tier0Reason        ← Why a reference stayed at Tier 0
//!         ├── TierStatistics     ← Analysis statistics
//!         │
//!         ├── Conversions:
//!         │   └── to_vbc_tier()  → verum_vbc::CbgrTier
//!         │
//!         └── Used by:
//!             ├── tier_analysis.rs (main analyzer)
//!             ├── session.rs (cache)
//!             └── vbc_codegen.rs (code generation)
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_cbgr::tier_types::{ReferenceTier, Tier0Reason};
//!
//! let tier = ReferenceTier::tier0(Tier0Reason::Escapes);
//! assert_eq!(tier.tier_number(), 0);
//! assert_eq!(tier.overhead_ns(), 15);
//!
//! let promoted = ReferenceTier::tier1();
//! assert!(promoted.is_promoted());
//! assert_eq!(promoted.to_vbc_tier(), CbgrTier::Tier1);
//! ```
//!
//! Canonical tier definitions for the CBGR-VBC integration pipeline. Tier 0
//! uses full CBGR validation (~15ns), Tier 1 is compiler-verified safe (0ns),
//! Tier 2 is unsafe/manual (0ns). VBC codegen converts these to instruction-level
//! CbgrTier values for Ref/RefChecked/RefUnsafe instruction emission.

use std::fmt;
use verum_common::Map;

// ============================================================================
// CBGR Tier (canonical definition)
// ============================================================================

/// CBGR reference tier for memory safety validation.
///
/// This is the canonical definition used throughout the compiler.
/// VBC codegen should convert this to its internal tier representation.
///
/// | Tier | Overhead | Description |
/// |------|----------|-------------|
/// | Tier0 | ~15ns | Runtime CBGR validation |
/// | Tier1 | 0ns | Compiler-proven safe |
/// | Tier2 | 0ns | Manual safety proof |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum CbgrTier {
    /// Tier 0: Runtime checked (~15ns overhead).
    #[default]
    Tier0 = 0,
    /// Tier 1: Compiler proven safe (0ns overhead).
    Tier1 = 1,
    /// Tier 2: Manual proof required (0ns overhead, unsafe).
    Tier2 = 2,
}

// ============================================================================
// Unified Reference Tier
// ============================================================================

/// Unified reference tier with detailed reason tracking.
///
/// This is the canonical tier representation used throughout the compiler.
/// It combines the simplicity of `CbgrTier` (Tier0/Tier1/Tier2) with
/// detailed reason tracking for Tier0 decisions.
///
/// # Performance Impact
///
/// | Tier | Overhead | Description |
/// |------|----------|-------------|
/// | Tier0 | ~15ns | Runtime CBGR validation |
/// | Tier1 | 0ns | Compiler-proven safe |
/// | Tier2 | 0ns | Manual safety proof |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceTier {
    /// Tier 0: CBGR-managed reference with runtime validation (~15ns overhead).
    ///
    /// This is the conservative default. References are kept at Tier 0 when:
    /// - Escape analysis cannot prove safety
    /// - Reference crosses async boundaries
    /// - Reference is on exception paths
    /// - Dominance analysis fails
    Tier0 {
        /// Reason why this reference couldn't be promoted.
        reason: Tier0Reason,
    },

    /// Tier 1: Compiler-proven safe reference (0ns overhead).
    ///
    /// References are promoted to Tier 1 when escape analysis proves
    /// they don't escape the current scope and dominance is satisfied.
    Tier1,

    /// Tier 2: Unsafe reference with manual safety proof (0ns overhead).
    ///
    /// Only used when explicitly marked as `&unsafe T` in source code.
    /// The programmer takes responsibility for memory safety.
    Tier2,
}

impl ReferenceTier {
    /// Create a Tier 0 reference with the given reason.
    #[must_use]
    pub fn tier0(reason: Tier0Reason) -> Self {
        Self::Tier0 { reason }
    }

    /// Create a Tier 1 (promoted) reference.
    #[must_use]
    pub fn tier1() -> Self {
        Self::Tier1
    }

    /// Create a Tier 2 (unsafe) reference.
    #[must_use]
    pub fn tier2() -> Self {
        Self::Tier2
    }

    /// Check if this is a promoted tier (Tier 1 or Tier 2).
    #[must_use]
    pub fn is_promoted(&self) -> bool {
        matches!(self, Self::Tier1 | Self::Tier2)
    }

    /// Get the tier number (0, 1, or 2).
    #[must_use]
    pub fn tier_number(&self) -> u8 {
        match self {
            Self::Tier0 { .. } => 0,
            Self::Tier1 => 1,
            Self::Tier2 => 2,
        }
    }

    /// Get estimated overhead in nanoseconds per dereference.
    #[must_use]
    pub fn overhead_ns(&self) -> u64 {
        match self {
            Self::Tier0 { .. } => 15,
            Self::Tier1 | Self::Tier2 => 0,
        }
    }

    /// Convert to VBC's CbgrTier for code generation.
    #[must_use]
    pub fn to_vbc_tier(&self) -> CbgrTier {
        match self {
            Self::Tier0 { .. } => CbgrTier::Tier0,
            Self::Tier1 => CbgrTier::Tier1,
            Self::Tier2 => CbgrTier::Tier2,
        }
    }

    /// Get the reason if this is Tier 0.
    #[must_use]
    pub fn reason(&self) -> Option<&Tier0Reason> {
        match self {
            Self::Tier0 { reason } => Some(reason),
            _ => None,
        }
    }
}

impl Default for ReferenceTier {
    fn default() -> Self {
        Self::Tier0 { reason: Tier0Reason::NotAnalyzed }
    }
}

impl fmt::Display for ReferenceTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tier0 { reason } => write!(f, "Tier0 ({})", reason),
            Self::Tier1 => write!(f, "Tier1 (promoted)"),
            Self::Tier2 => write!(f, "Tier2 (unsafe)"),
        }
    }
}

// ============================================================================
// Tier 0 Reasons
// ============================================================================

/// Reason why a reference is kept at Tier 0.
///
/// This provides detailed diagnostics for understanding why a reference
/// couldn't be promoted to Tier 1 (zero-overhead).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier0Reason {
    /// Reference escapes the function scope (via return, heap, closure, etc.).
    Escapes,

    /// Allocation site doesn't dominate all use sites.
    DominanceFailure,

    /// Conservative decision due to analysis limitations or complexity.
    Conservative,

    /// Reference crosses async/await boundary.
    AsyncBoundary,

    /// Reference is used on exception handling path.
    ExceptionPath,

    /// Escape analysis confidence score below threshold.
    LowConfidence,

    /// Reference was not analyzed (default state).
    NotAnalyzed,

    /// Reference has concurrent access (shared across threads/tasks).
    ConcurrentAccess,

    /// Reference stored to mutable field that may escape.
    MutableFieldStore,

    /// Reference passed to external function (FFI, callback).
    ExternalCall,

    /// Use-after-free detected by ownership analysis.
    UseAfterFree,

    /// Lifetime violation detected by lifetime analysis.
    LifetimeViolation,

    /// Borrow violation detected by NLL analysis.
    BorrowViolation,

    /// Double-free detected by ownership analysis.
    DoubleFree,

    /// Data race detected by concurrency analysis.
    DataRace,

    /// Analysis timed out — safe fallback to full runtime checks.
    AnalysisTimeout,
}

impl Tier0Reason {
    /// Human-readable description of the reason.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Escapes => "reference escapes function scope",
            Self::DominanceFailure => "allocation doesn't dominate all uses",
            Self::Conservative => "conservative analysis decision",
            Self::AsyncBoundary => "crosses async/await boundary",
            Self::ExceptionPath => "used on exception handling path",
            Self::LowConfidence => "analysis confidence below threshold",
            Self::NotAnalyzed => "not analyzed",
            Self::ConcurrentAccess => "concurrent access detected",
            Self::MutableFieldStore => "stored to escaping mutable field",
            Self::ExternalCall => "passed to external function",
            Self::UseAfterFree => "use-after-free detected",
            Self::LifetimeViolation => "lifetime violation detected",
            Self::BorrowViolation => "borrow violation detected",
            Self::DoubleFree => "double-free detected",
            Self::DataRace => "data race detected",
            Self::AnalysisTimeout => "analysis timed out, using safe default",
        }
    }

    /// Check if this reason is recoverable with more analysis.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Self::Conservative | Self::LowConfidence | Self::NotAnalyzed
        )
    }
}

impl fmt::Display for Tier0Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

// ============================================================================
// Unified Statistics
// ============================================================================

/// Unified statistics for tier analysis.
///
/// This replaces both `PluginStatistics` (old API) and `TierAnalysisStats` (new API)
/// with a single, comprehensive statistics structure.
#[derive(Debug, Clone, Default)]
pub struct TierStatistics {
    /// Total functions analyzed.
    pub functions_analyzed: u64,

    /// Total references analyzed.
    pub total_refs: u64,

    /// References at Tier 0 (CBGR-managed).
    pub tier0_count: u64,

    /// References at Tier 1 (promoted).
    pub tier1_count: u64,

    /// References at Tier 2 (unsafe).
    pub tier2_count: u64,

    /// Breakdown of Tier 0 reasons.
    pub tier0_reasons: Map<Tier0Reason, u64>,

    /// Estimated time saved per execution (nanoseconds).
    pub estimated_savings_ns: u64,

    /// Total analysis duration (microseconds).
    pub analysis_duration_us: u64,
}

impl TierStatistics {
    /// Create new empty statistics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate promotion rate (0.0 to 1.0).
    #[must_use]
    pub fn promotion_rate(&self) -> f64 {
        if self.total_refs == 0 {
            0.0
        } else {
            (self.tier1_count + self.tier2_count) as f64 / self.total_refs as f64
        }
    }

    /// Calculate average promotions per function.
    #[must_use]
    pub fn avg_promotions_per_function(&self) -> f64 {
        if self.functions_analyzed == 0 {
            0.0
        } else {
            self.tier1_count as f64 / self.functions_analyzed as f64
        }
    }

    /// Record a tier decision.
    pub fn record(&mut self, tier: &ReferenceTier) {
        self.total_refs += 1;
        match tier {
            ReferenceTier::Tier0 { reason } => {
                self.tier0_count += 1;
                *self.tier0_reasons.entry(*reason).or_insert(0) += 1;
            }
            ReferenceTier::Tier1 => {
                self.tier1_count += 1;
                // Estimate: 10 dereferences per reference, 15ns saved each
                self.estimated_savings_ns += 150;
            }
            ReferenceTier::Tier2 => {
                self.tier2_count += 1;
            }
        }
    }

    /// Merge statistics from another instance.
    pub fn merge(&mut self, other: &TierStatistics) {
        self.functions_analyzed += other.functions_analyzed;
        self.total_refs += other.total_refs;
        self.tier0_count += other.tier0_count;
        self.tier1_count += other.tier1_count;
        self.tier2_count += other.tier2_count;
        self.estimated_savings_ns += other.estimated_savings_ns;
        self.analysis_duration_us += other.analysis_duration_us;

        for (reason, count) in &other.tier0_reasons {
            *self.tier0_reasons.entry(*reason).or_insert(0) += count;
        }
    }
}

impl fmt::Display for TierStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Tier Analysis Statistics:")?;
        writeln!(f, "  Functions analyzed:  {}", self.functions_analyzed)?;
        writeln!(f, "  Total references:    {}", self.total_refs)?;
        writeln!(f, "  Tier 0 (managed):    {}", self.tier0_count)?;
        writeln!(f, "  Tier 1 (promoted):   {}", self.tier1_count)?;
        writeln!(f, "  Tier 2 (unsafe):     {}", self.tier2_count)?;
        writeln!(f, "  Promotion rate:      {:.1}%", self.promotion_rate() * 100.0)?;
        writeln!(f, "  Est. savings/exec:   ~{}ns", self.estimated_savings_ns)?;
        writeln!(f, "  Analysis time:       {}μs", self.analysis_duration_us)?;

        if !self.tier0_reasons.is_empty() {
            writeln!(f, "  Tier 0 breakdown:")?;
            for (reason, count) in &self.tier0_reasons {
                writeln!(f, "    - {}: {}", reason, count)?;
            }
        }

        Ok(())
    }
}

// ============================================================================
// Reference ID
// ============================================================================

/// Reference identifier for tier tracking.
///
/// This is a unified ID that can represent references from different
/// analysis stages (AST span, MIR local, VBC register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceId(pub u64);

impl ReferenceId {
    /// Create from an AST span (start, end).
    #[must_use]
    pub fn from_span(start: u32, end: u32) -> Self {
        Self(((start as u64) << 32) | (end as u64))
    }

    /// Create from a local variable index.
    #[must_use]
    pub fn from_local(index: u32) -> Self {
        Self(index as u64)
    }

    /// Create from the analysis RefId.
    #[must_use]
    pub fn from_analysis_ref(ref_id: crate::analysis::RefId) -> Self {
        Self(ref_id.0)
    }
}

impl From<crate::analysis::RefId> for ReferenceId {
    fn from(ref_id: crate::analysis::RefId) -> Self {
        Self::from_analysis_ref(ref_id)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_tier_basics() {
        let t0 = ReferenceTier::tier0(Tier0Reason::Escapes);
        assert_eq!(t0.tier_number(), 0);
        assert!(!t0.is_promoted());
        assert_eq!(t0.overhead_ns(), 15);

        let t1 = ReferenceTier::tier1();
        assert_eq!(t1.tier_number(), 1);
        assert!(t1.is_promoted());
        assert_eq!(t1.overhead_ns(), 0);

        let t2 = ReferenceTier::tier2();
        assert_eq!(t2.tier_number(), 2);
        assert!(t2.is_promoted());
        assert_eq!(t2.overhead_ns(), 0);
    }

    #[test]
    fn test_vbc_conversion() {
        assert_eq!(
            ReferenceTier::tier0(Tier0Reason::Escapes).to_vbc_tier(),
            CbgrTier::Tier0
        );
        assert_eq!(ReferenceTier::tier1().to_vbc_tier(), CbgrTier::Tier1);
        assert_eq!(ReferenceTier::tier2().to_vbc_tier(), CbgrTier::Tier2);
    }

    #[test]
    fn test_tier0_reason_display() {
        assert_eq!(
            Tier0Reason::Escapes.description(),
            "reference escapes function scope"
        );
        assert!(Tier0Reason::Conservative.is_recoverable());
        assert!(!Tier0Reason::Escapes.is_recoverable());
    }

    #[test]
    fn test_statistics() {
        let mut stats = TierStatistics::new();

        stats.record(&ReferenceTier::tier0(Tier0Reason::Escapes));
        stats.record(&ReferenceTier::tier0(Tier0Reason::Escapes));
        stats.record(&ReferenceTier::tier1());
        stats.record(&ReferenceTier::tier2());

        assert_eq!(stats.total_refs, 4);
        assert_eq!(stats.tier0_count, 2);
        assert_eq!(stats.tier1_count, 1);
        assert_eq!(stats.tier2_count, 1);
        assert_eq!(stats.promotion_rate(), 0.5);
        assert_eq!(*stats.tier0_reasons.get(&Tier0Reason::Escapes).unwrap(), 2);
    }

    #[test]
    fn test_statistics_merge() {
        let mut stats1 = TierStatistics::new();
        stats1.record(&ReferenceTier::tier1());
        stats1.functions_analyzed = 1;

        let mut stats2 = TierStatistics::new();
        stats2.record(&ReferenceTier::tier0(Tier0Reason::Escapes));
        stats2.functions_analyzed = 1;

        stats1.merge(&stats2);

        assert_eq!(stats1.functions_analyzed, 2);
        assert_eq!(stats1.total_refs, 2);
        assert_eq!(stats1.tier0_count, 1);
        assert_eq!(stats1.tier1_count, 1);
    }

    #[test]
    fn test_reference_id() {
        let from_span = ReferenceId::from_span(100, 200);
        assert_eq!(from_span.0, (100u64 << 32) | 200);

        let from_local = ReferenceId::from_local(42);
        assert_eq!(from_local.0, 42);
    }
}
