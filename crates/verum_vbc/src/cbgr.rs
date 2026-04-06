//! CBGR Codegen Abstractions for VBC
//!
//! This module provides abstract code generation strategies for CBGR (Counter-Based
//! Garbage Rejection) memory safety operations. These abstractions are used by:
//!
//! - VBC interpreter for inline CBGR checks (dispatch.rs)
//! - VBC → MLIR lowering for generating optimized memory operations
//! - Escape analysis integration for tier decisions
//!
//! # Three-Tier Safety Model
//!
//! - **Tier 0 (Managed)**: Runtime CBGR validation (~15ns overhead)
//! - **Tier 1 (Checked)**: Compiler-proven safe (0ns overhead)
//! - **Tier 2 (Unsafe)**: Manual safety proof (0ns overhead)
//!
//! # Architecture
//!
//! ```text
//! verum_cbgr (compile-time analysis)
//!       │
//!       │ produces tier decisions
//!       ▼
//! DereferenceCodegen / CapabilityCheckCodegen
//!       │
//!       │ consumed by
//!       ▼
//! ┌─────────────────────────────────────────┐
//! │ VBC Interpreter    │ VBC → MLIR Lowering │
//! │ (inline checks)    │ (optimized IR)      │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust
//! use verum_vbc::cbgr::{DereferenceCodegen, CbgrTier};
//!
//! // Tier decision from escape analysis
//! let tier = CbgrTier::Tier1; // compiler proved safe
//!
//! // Select codegen strategy
//! let strategy = DereferenceCodegen::for_tier(tier);
//! assert!(matches!(strategy, DereferenceCodegen::DirectAccess));
//! ```

// Re-export CbgrTier for convenience
pub use crate::types::CbgrTier;

// ============================================================================
// Dereference Code Generation Strategy
// ============================================================================

/// Code generation strategy for dereference operations.
///
/// Describes HOW to generate code for dereferencing a reference,
/// based on the tier decision from escape analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DereferenceCodegen {
    /// Inline CBGR validation (Tier 0 - Managed)
    ///
    /// Generates code that:
    /// 1. Loads current generation from allocation header
    /// 2. Compares with expected generation in reference
    /// 3. Loads current epoch and compares
    /// 4. Branches to panic on mismatch
    /// 5. Proceeds with dereference on success
    ///
    /// Overhead: ~15ns (5-7 instructions)
    InlineCbgrCheck {
        /// Expected generation (filled during codegen)
        expected_generation: u32,
        /// Expected epoch (filled during codegen)
        expected_epoch: u16,
        /// Whether to panic on validation failure
        panic_on_failure: bool,
    },

    /// Direct pointer access (Tier 1 - Checked)
    ///
    /// Generates a single pointer load - no validation needed
    /// because the compiler proved this reference is safe.
    ///
    /// Overhead: 0ns (1 instruction)
    DirectAccess,

    /// Unchecked pointer access (Tier 2 - Unsafe)
    ///
    /// Generates a single pointer load with no metadata.
    /// Developer has provided proof of safety.
    ///
    /// Overhead: 0ns (1 instruction)
    UncheckedAccess,
}

impl DereferenceCodegen {
    /// Create strategy for a given CBGR tier.
    #[must_use]
    pub fn for_tier(tier: CbgrTier) -> Self {
        match tier {
            CbgrTier::Tier0 => Self::pending_cbgr(),
            CbgrTier::Tier1 => Self::DirectAccess,
            CbgrTier::Tier2 => Self::UncheckedAccess,
        }
    }

    /// Create inline CBGR check strategy with known values.
    #[must_use]
    pub fn inline_cbgr(generation: u32, epoch: u16) -> Self {
        Self::InlineCbgrCheck {
            expected_generation: generation,
            expected_epoch: epoch,
            panic_on_failure: true,
        }
    }

    /// Create pending CBGR check strategy (values filled during codegen).
    ///
    /// Used during static analysis when generation/epoch values
    /// are not yet known. Values will be filled via `with_values()`.
    #[must_use]
    pub fn pending_cbgr() -> Self {
        Self::InlineCbgrCheck {
            expected_generation: 0,
            expected_epoch: 0,
            panic_on_failure: true,
        }
    }

    /// Update CBGR check with actual generation/epoch values.
    #[must_use]
    pub fn with_values(self, generation: u32, epoch: u16) -> Self {
        match self {
            Self::InlineCbgrCheck { panic_on_failure, .. } => Self::InlineCbgrCheck {
                expected_generation: generation,
                expected_epoch: epoch,
                panic_on_failure,
            },
            other => other,
        }
    }

    /// Check if this is a pending CBGR check (needs values filled).
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(
            self,
            Self::InlineCbgrCheck {
                expected_generation: 0,
                expected_epoch: 0,
                ..
            }
        )
    }

    /// Get estimated instruction count.
    #[must_use]
    pub const fn instruction_count(&self) -> usize {
        match self {
            Self::InlineCbgrCheck { .. } => 7, // load + cmp + load + cmp + branch + panic
            Self::DirectAccess => 1,           // single load
            Self::UncheckedAccess => 1,        // single load
        }
    }

    /// Get the CBGR tier this strategy corresponds to.
    #[must_use]
    pub const fn tier(&self) -> CbgrTier {
        match self {
            Self::InlineCbgrCheck { .. } => CbgrTier::Tier0,
            Self::DirectAccess => CbgrTier::Tier1,
            Self::UncheckedAccess => CbgrTier::Tier2,
        }
    }
}

// ============================================================================
// Capability System
// ============================================================================

/// Required capability for an operation.
///
/// Capabilities are fine-grained permissions that can be checked
/// at compile-time or runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequiredCapability {
    /// Read access (dereference)
    Read,
    /// Write access (mutable dereference)
    Write,
    /// Execute access (function call through reference)
    Execute,
    /// Delegate capability (create sub-references)
    Delegate,
    /// Revoke capability (invalidate derived references)
    Revoke,
}

impl RequiredCapability {
    /// Get capability bit mask.
    ///
    /// Matches the bit layout in CBGR runtime:
    /// - Read: 0x01
    /// - Write: 0x02
    /// - Execute: 0x04
    /// - Delegate: 0x08
    /// - Revoke: 0x10
    #[must_use]
    pub const fn bit_mask(self) -> u16 {
        match self {
            Self::Read => 0x01,
            Self::Write => 0x02,
            Self::Execute => 0x04,
            Self::Delegate => 0x08,
            Self::Revoke => 0x10,
        }
    }

    /// Check if capability is present in a capability set.
    #[must_use]
    pub const fn is_present_in(self, caps: u16) -> bool {
        (caps & self.bit_mask()) != 0
    }
}

/// Capability check code generation strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityCheckCodegen {
    /// Inline capability check (runtime ~1ns).
    InlineCheck {
        /// Required capability bits.
        required_caps: u16,
        /// Panic on failure.
        panic_on_failure: bool,
    },

    /// Skip capability check (static proof available).
    SkipCheck,

    /// Compile-time error (capability definitely missing).
    CompileError {
        /// Missing capability.
        missing: RequiredCapability,
    },
}

impl CapabilityCheckCodegen {
    /// Create inline check for read access.
    #[must_use]
    pub fn read_check() -> Self {
        Self::InlineCheck {
            required_caps: RequiredCapability::Read.bit_mask(),
            panic_on_failure: true,
        }
    }

    /// Create inline check for write access.
    #[must_use]
    pub fn write_check() -> Self {
        Self::InlineCheck {
            required_caps: RequiredCapability::Read.bit_mask() | RequiredCapability::Write.bit_mask(),
            panic_on_failure: true,
        }
    }

    /// Create inline check for execute access.
    #[must_use]
    pub fn execute_check() -> Self {
        Self::InlineCheck {
            required_caps: RequiredCapability::Execute.bit_mask(),
            panic_on_failure: true,
        }
    }

    /// Create inline check for delegate access.
    #[must_use]
    pub fn delegate_check() -> Self {
        Self::InlineCheck {
            required_caps: RequiredCapability::Delegate.bit_mask(),
            panic_on_failure: true,
        }
    }

    /// Get estimated instruction count.
    #[must_use]
    pub const fn instruction_count(&self) -> usize {
        match self {
            Self::InlineCheck { .. } => 3, // load + and + br
            Self::SkipCheck => 0,
            Self::CompileError { .. } => 0,
        }
    }

    /// Check if this is a compile-time error.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::CompileError { .. })
    }
}

// ============================================================================
// Combined Codegen Strategy
// ============================================================================

/// Combined dereference + capability check strategy.
///
/// This represents the complete code generation decision for
/// a reference operation, combining CBGR validation with
/// capability checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CbgrDereferenceStrategy {
    /// CBGR dereference strategy.
    pub deref: DereferenceCodegen,
    /// Capability check strategy.
    pub capability: CapabilityCheckCodegen,
    /// The tier this strategy targets.
    pub tier: CbgrTier,
}

impl CbgrDereferenceStrategy {
    /// Create strategy for managed read (Tier 0).
    #[must_use]
    pub fn managed_read(generation: u32, epoch: u16) -> Self {
        Self {
            deref: DereferenceCodegen::inline_cbgr(generation, epoch),
            capability: CapabilityCheckCodegen::read_check(),
            tier: CbgrTier::Tier0,
        }
    }

    /// Create strategy for managed write (Tier 0).
    #[must_use]
    pub fn managed_write(generation: u32, epoch: u16) -> Self {
        Self {
            deref: DereferenceCodegen::inline_cbgr(generation, epoch),
            capability: CapabilityCheckCodegen::write_check(),
            tier: CbgrTier::Tier0,
        }
    }

    /// Create strategy for checked read (Tier 1).
    #[must_use]
    pub fn checked_read() -> Self {
        Self {
            deref: DereferenceCodegen::DirectAccess,
            capability: CapabilityCheckCodegen::SkipCheck,
            tier: CbgrTier::Tier1,
        }
    }

    /// Create strategy for checked write (Tier 1).
    #[must_use]
    pub fn checked_write() -> Self {
        Self {
            deref: DereferenceCodegen::DirectAccess,
            capability: CapabilityCheckCodegen::SkipCheck,
            tier: CbgrTier::Tier1,
        }
    }

    /// Create strategy for unsafe access (Tier 2).
    #[must_use]
    pub fn unsafe_access() -> Self {
        Self {
            deref: DereferenceCodegen::UncheckedAccess,
            capability: CapabilityCheckCodegen::SkipCheck,
            tier: CbgrTier::Tier2,
        }
    }

    /// Get total instruction count.
    #[must_use]
    pub const fn total_instruction_count(&self) -> usize {
        self.deref.instruction_count() + self.capability.instruction_count()
    }

    /// Get estimated overhead in nanoseconds.
    #[must_use]
    pub const fn estimated_overhead_ns(&self) -> u64 {
        let cbgr_overhead = match self.tier {
            CbgrTier::Tier0 => 15,
            CbgrTier::Tier1 => 0,
            CbgrTier::Tier2 => 0,
        };
        let cap_overhead = match &self.capability {
            CapabilityCheckCodegen::InlineCheck { .. } => 1,
            _ => 0,
        };
        cbgr_overhead + cap_overhead
    }
}

// ============================================================================
// CBGR Codegen Statistics
// ============================================================================

/// Statistics for CBGR code generation.
///
/// Tracks how many references of each tier were generated.
#[derive(Debug, Clone, Default)]
pub struct CbgrCodegenStats {
    /// Tier 0 dereferences (managed, ~15ns).
    pub tier0_derefs: usize,
    /// Tier 1 dereferences (checked, 0ns).
    pub tier1_derefs: usize,
    /// Tier 2 dereferences (unsafe, 0ns).
    pub tier2_derefs: usize,
    /// CBGR checks eliminated by escape analysis.
    pub checks_eliminated: usize,
    /// Capability checks generated.
    pub capability_checks: usize,
}

impl CbgrCodegenStats {
    /// Create new empty statistics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a dereference for a tier.
    pub fn record_deref(&mut self, tier: CbgrTier) {
        match tier {
            CbgrTier::Tier0 => self.tier0_derefs += 1,
            CbgrTier::Tier1 => self.tier1_derefs += 1,
            CbgrTier::Tier2 => self.tier2_derefs += 1,
        }
    }

    /// Record a CBGR check elimination.
    pub fn record_elimination(&mut self) {
        self.checks_eliminated += 1;
    }

    /// Record a capability check.
    pub fn record_capability_check(&mut self) {
        self.capability_checks += 1;
    }

    /// Get total dereferences.
    #[must_use]
    pub fn total_derefs(&self) -> usize {
        self.tier0_derefs + self.tier1_derefs + self.tier2_derefs
    }

    /// Get optimization rate (percentage of non-Tier0 derefs).
    #[must_use]
    pub fn optimization_rate(&self) -> f64 {
        let total = self.total_derefs();
        if total == 0 {
            0.0
        } else {
            (self.tier1_derefs + self.tier2_derefs) as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deref_codegen_for_tier() {
        assert!(matches!(
            DereferenceCodegen::for_tier(CbgrTier::Tier0),
            DereferenceCodegen::InlineCbgrCheck { .. }
        ));
        assert!(matches!(
            DereferenceCodegen::for_tier(CbgrTier::Tier1),
            DereferenceCodegen::DirectAccess
        ));
        assert!(matches!(
            DereferenceCodegen::for_tier(CbgrTier::Tier2),
            DereferenceCodegen::UncheckedAccess
        ));
    }

    #[test]
    fn test_deref_codegen_with_values() {
        let pending = DereferenceCodegen::pending_cbgr();
        assert!(pending.is_pending());

        let filled = pending.with_values(42, 1);
        assert!(!filled.is_pending());

        if let DereferenceCodegen::InlineCbgrCheck { expected_generation, expected_epoch, .. } = filled {
            assert_eq!(expected_generation, 42);
            assert_eq!(expected_epoch, 1);
        } else {
            panic!("Expected InlineCbgrCheck");
        }
    }

    #[test]
    fn test_capability_bit_mask() {
        assert_eq!(RequiredCapability::Read.bit_mask(), 0x01);
        assert_eq!(RequiredCapability::Write.bit_mask(), 0x02);
        assert_eq!(RequiredCapability::Execute.bit_mask(), 0x04);

        let caps = 0x03; // Read + Write
        assert!(RequiredCapability::Read.is_present_in(caps));
        assert!(RequiredCapability::Write.is_present_in(caps));
        assert!(!RequiredCapability::Execute.is_present_in(caps));
    }

    #[test]
    fn test_cbgr_strategy() {
        let managed = CbgrDereferenceStrategy::managed_read(42, 1);
        assert_eq!(managed.tier, CbgrTier::Tier0);
        assert_eq!(managed.estimated_overhead_ns(), 16); // 15ns + 1ns cap check

        let checked = CbgrDereferenceStrategy::checked_read();
        assert_eq!(checked.tier, CbgrTier::Tier1);
        assert_eq!(checked.estimated_overhead_ns(), 0);

        let unsafe_access = CbgrDereferenceStrategy::unsafe_access();
        assert_eq!(unsafe_access.tier, CbgrTier::Tier2);
        assert_eq!(unsafe_access.estimated_overhead_ns(), 0);
    }

    #[test]
    fn test_cbgr_stats() {
        let mut stats = CbgrCodegenStats::new();
        stats.record_deref(CbgrTier::Tier0);
        stats.record_deref(CbgrTier::Tier0);
        stats.record_deref(CbgrTier::Tier1);
        stats.record_elimination();

        assert_eq!(stats.tier0_derefs, 2);
        assert_eq!(stats.tier1_derefs, 1);
        assert_eq!(stats.total_derefs(), 3);
        assert!((stats.optimization_rate() - 0.333).abs() < 0.01);
    }
}
