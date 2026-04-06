//! Escape Analysis Categories for CBGR Optimization
//!
//! SBGL (Scope-Bound Generation-Less) optimization is ONLY applicable to NoEscape
//! references. A NoEscape reference dies in scope, so CBGR can use raw pointers
//! internally (0ns). LocalEscape references must return ThinRef/FatRef to satisfy
//! the &T contract (~15ns). HeapEscape references need generation tracking for
//! heap lifetime management. ThreadEscape references require atomic CBGR checks
//! for cross-thread safety. Unknown defaults to conservative CBGR (~15ns).
//!
//! This module defines the four escape categories used for CBGR optimization:
//! - **`NoEscape`**: Reference dies in scope (SBGL applicable)
//! - **`LocalEscape`**: Reference returns to caller (CBGR required)
//! - **`HeapEscape`**: Reference stored in heap (CBGR required)
//! - **`ThreadEscape`**: Reference crosses thread boundaries (CBGR required)
//!
//! # SBGL Applicability
//!
//! **CRITICAL**: SBGL (Stack-Based Generation Lifting) is **ONLY** applicable
//! to `NoEscape` references. This is a fundamental limitation based on semantic
//! honesty.
//!
//! | Category | CBGR Cost | SBGL Applicable | Reason |
//! |----------|-----------|-----------------|--------|
//! | `NoEscape` | 0ns (optimized) | ✅ Yes | Reference dies in scope |
//! | `LocalEscape` | ~15ns | ❌ No | Returns to caller |
//! | `HeapEscape` | ~15ns | ❌ No | Stored in heap |
//! | `ThreadEscape` | ~15ns | ❌ No | Crosses threads |
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_cbgr::escape_categories::{EscapeCategory, categorize_escape};
//!
//! // NoEscape: Reference dies in scope
//! fn sum_list(data: &List<Int>) -> Int {
//!     let mut total = 0;
//!     for item in data {
//!         total += item;  // 'item' is NoEscape (SBGL applicable)
//!     }
//!     total  // Returns Int, not a reference
//! }
//!
//! // LocalEscape: Reference returns to caller
//! fn first_element(data: &List<Int>) -> &Int {
//!     &data[0]  // Returns reference (SBGL NOT applicable)
//! }
//! ```

use crate::analysis::{EscapeResult, RefId};
use std::fmt;
use verum_common::{List, Text};

/// Escape category for reference optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapeCategory {
    /// Reference dies in scope
    ///
    /// - CBGR cost: 0ns (optimized via SBGL)
    /// - SBGL applicable: ✅ Yes
    /// - Example: Loop iteration variables
    /// - Guarantee: Reference never outlives function scope
    NoEscape,

    /// Reference returns to caller
    ///
    /// - CBGR cost: ~15ns (required for safety)
    /// - SBGL applicable: ❌ No (must return ThinRef/FatRef)
    /// - Example: Function return values
    /// - Limitation: Cannot use raw pointer optimization
    LocalEscape,

    /// Reference stored in heap
    ///
    /// - CBGR cost: ~15ns (required for safety)
    /// - SBGL applicable: ❌ No (heap outlives stack)
    /// - Example: References stored in Box, Heap, Arc
    /// - Requirement: Must preserve generation tracking
    HeapEscape,

    /// Reference crosses thread boundaries
    ///
    /// - CBGR cost: ~15ns (required for safety)
    /// - SBGL applicable: ❌ No (thread safety required)
    /// - Example: References sent to other threads
    /// - Requirement: Full CBGR synchronization
    ThreadEscape,

    /// Unknown escape behavior (conservative)
    ///
    /// - CBGR cost: ~15ns (conservative approach)
    /// - SBGL applicable: ❌ No (safety first)
    /// - Example: Opaque function calls, FFI
    /// - Fallback: Assume worst case
    Unknown,
}

impl EscapeCategory {
    /// Get CBGR check cost in nanoseconds
    #[must_use]
    pub const fn cbgr_cost_ns(&self) -> u32 {
        match self {
            EscapeCategory::NoEscape => 0,
            EscapeCategory::LocalEscape => 15,
            EscapeCategory::HeapEscape => 15,
            EscapeCategory::ThreadEscape => 15,
            EscapeCategory::Unknown => 15,
        }
    }

    /// Check if SBGL optimization is applicable
    #[must_use]
    pub const fn sbgl_applicable(&self) -> bool {
        matches!(self, EscapeCategory::NoEscape)
    }

    /// Get category name
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            EscapeCategory::NoEscape => "NoEscape",
            EscapeCategory::LocalEscape => "LocalEscape",
            EscapeCategory::HeapEscape => "HeapEscape",
            EscapeCategory::ThreadEscape => "ThreadEscape",
            EscapeCategory::Unknown => "Unknown",
        }
    }

    /// Get category description
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            EscapeCategory::NoEscape => "Reference dies in scope (SBGL optimizable)",
            EscapeCategory::LocalEscape => "Reference returns to caller (CBGR required)",
            EscapeCategory::HeapEscape => "Reference stored in heap (CBGR required)",
            EscapeCategory::ThreadEscape => "Reference crosses threads (CBGR required)",
            EscapeCategory::Unknown => "Unknown escape (conservative CBGR)",
        }
    }

    /// Get optimization recommendation
    #[must_use]
    pub const fn optimization_hint(&self) -> &'static str {
        match self {
            EscapeCategory::NoEscape => "SBGL: 0ns (reference dies in scope)",
            EscapeCategory::LocalEscape => "CBGR: ~15ns (cannot return raw pointer from SBGL)",
            EscapeCategory::HeapEscape => "CBGR: ~15ns (heap storage requires generation tracking)",
            EscapeCategory::ThreadEscape => "CBGR: ~15ns (thread safety requires full checks)",
            EscapeCategory::Unknown => "CBGR: ~15ns (conservative default)",
        }
    }
}

impl fmt::Display for EscapeCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}ns)", self.name(), self.cbgr_cost_ns())
    }
}

/// Categorize escape result into optimization category
///
/// This function maps the detailed `EscapeResult` analysis into one of
/// the four escape categories for optimization decisions.
///
/// # Example
///
/// ```rust,ignore
/// use verum_cbgr::escape_categories::categorize_escape;
/// use verum_cbgr::analysis::EscapeResult;
///
/// let result = EscapeResult::DoesNotEscape;
/// let category = categorize_escape(result);
/// assert_eq!(category, EscapeCategory::NoEscape);
/// ```
#[must_use]
pub fn categorize_escape(result: EscapeResult) -> EscapeCategory {
    match result {
        EscapeResult::DoesNotEscape => EscapeCategory::NoEscape,
        EscapeResult::EscapesViaReturn => EscapeCategory::LocalEscape,
        EscapeResult::EscapesViaHeap => EscapeCategory::HeapEscape,
        EscapeResult::EscapesViaThread => EscapeCategory::ThreadEscape,
        EscapeResult::EscapesViaClosure => EscapeCategory::HeapEscape, // Closures capture to heap
        EscapeResult::ConcurrentAccess => EscapeCategory::ThreadEscape,
        EscapeResult::NonDominatingAllocation => EscapeCategory::Unknown,
        EscapeResult::ExceedsStackBounds => EscapeCategory::Unknown,
    }
}

/// Escape analysis decision for optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationDecision {
    /// Apply SBGL optimization (`NoEscape` only)
    ApplySbgl,
    /// Use CBGR with runtime checks
    UseCbgr,
    /// Fall back to conservative CBGR
    ConservativeCbgr,
}

impl OptimizationDecision {
    /// Make optimization decision based on escape category
    #[must_use]
    pub fn for_category(category: EscapeCategory) -> Self {
        match category {
            EscapeCategory::NoEscape => OptimizationDecision::ApplySbgl,
            EscapeCategory::LocalEscape
            | EscapeCategory::HeapEscape
            | EscapeCategory::ThreadEscape => OptimizationDecision::UseCbgr,
            EscapeCategory::Unknown => OptimizationDecision::ConservativeCbgr,
        }
    }

    /// Get expected CBGR cost
    #[must_use]
    pub const fn expected_cost_ns(&self) -> u32 {
        match self {
            OptimizationDecision::ApplySbgl => 0,
            OptimizationDecision::UseCbgr => 15,
            OptimizationDecision::ConservativeCbgr => 15,
        }
    }
}

/// Compiler diagnostic for SBGL applicability
#[derive(Debug, Clone)]
pub struct SbglDiagnostic {
    /// Reference being analyzed
    pub reference: RefId,
    /// Escape category
    pub category: EscapeCategory,
    /// Optimization decision
    pub decision: OptimizationDecision,
    /// Reason for decision
    pub reason: Text,
}

impl SbglDiagnostic {
    /// Create diagnostic for escape analysis
    #[must_use]
    pub fn new(reference: RefId, result: EscapeResult) -> Self {
        let category = categorize_escape(result);
        let decision = OptimizationDecision::for_category(category);
        let reason = match category {
            EscapeCategory::NoEscape => {
                "Reference dies in scope, SBGL optimization applied".to_string()
            }
            EscapeCategory::LocalEscape => {
                format!(
                    "Reference escapes via return, SBGL disabled: {}",
                    result.reason()
                )
            }
            EscapeCategory::HeapEscape => {
                format!(
                    "Reference stored in heap, SBGL disabled: {}",
                    result.reason()
                )
            }
            EscapeCategory::ThreadEscape => {
                format!(
                    "Reference crosses threads, SBGL disabled: {}",
                    result.reason()
                )
            }
            EscapeCategory::Unknown => {
                format!(
                    "Unknown escape pattern, conservative CBGR: {}",
                    result.reason()
                )
            }
        };

        Self {
            reference,
            category,
            decision,
            reason: reason.into(),
        }
    }

    /// Check if SBGL is applicable
    #[must_use]
    pub fn sbgl_applicable(&self) -> bool {
        matches!(self.decision, OptimizationDecision::ApplySbgl)
    }

    /// Get compiler warning message
    #[must_use]
    pub fn warning_message(&self) -> Option<Text> {
        if !self.sbgl_applicable() && self.category != EscapeCategory::Unknown {
            Some(format!(
                "⚠️ WARNING: SBGL optimization disabled for reference {:?}\n\
                 Category: {}\n\
                 Reason: {}\n\
                 Impact: CBGR cost ~{}ns per access",
                self.reference,
                self.category.name(),
                self.reason,
                self.decision.expected_cost_ns()
            ).into())
        } else {
            None
        }
    }
}

/// Escape pattern detector
pub struct EscapePatternDetector;

impl EscapePatternDetector {
    /// Detect common escape patterns
    #[must_use]
    pub fn detect_pattern(category: EscapeCategory) -> &'static str {
        match category {
            EscapeCategory::NoEscape => "Loop iteration, local variables, temporary values",
            EscapeCategory::LocalEscape => "Function return values, borrowed parameters",
            EscapeCategory::HeapEscape => "Box::new, Heap::new, Arc::new, storing in structs",
            EscapeCategory::ThreadEscape => "thread::spawn, channel send, shared state",
            EscapeCategory::Unknown => "Opaque calls, FFI, complex control flow",
        }
    }

    /// Get optimization tips for category
    #[must_use]
    pub fn optimization_tips(category: EscapeCategory) -> List<&'static str> {
        match category {
            EscapeCategory::NoEscape => {
                vec![
                    "✅ Perfect! Reference is optimally used",
                    "✅ SBGL optimization eliminates CBGR overhead",
                    "✅ Zero-cost abstraction achieved",
                ].into()
            }
            EscapeCategory::LocalEscape => {
                vec![
                    "Consider returning owned values instead of references",
                    "Use &checked T if lifetime can be statically proven",
                    "Profile to verify ~15ns overhead is acceptable",
                ].into()
            }
            EscapeCategory::HeapEscape => {
                vec![
                    "Consider stack allocation if possible",
                    "Use arena allocators for temporary heap data",
                    "Batch allocations to amortize CBGR cost",
                ].into()
            }
            EscapeCategory::ThreadEscape => {
                vec![
                    "Consider message passing instead of shared state",
                    "Use Arc for shared ownership across threads",
                    "Minimize cross-thread reference sharing",
                ].into()
            }
            EscapeCategory::Unknown => {
                vec![
                    "Improve type annotations for better analysis",
                    "Avoid opaque function calls in hot paths",
                    "Consider explicit &checked T for zero-cost access",
                ].into()
            }
        }
    }
}
