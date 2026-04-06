//! Level 1: Static Verification (Proof-Based Safety)
//!
//! Level 1 uses SMT-based verification (Z3) to prove properties at compile time.
//! Functions annotated with `@verify` directives have their preconditions,
//! postconditions, and invariants checked by the solver. Three verification modes
//! are supported: `@verify(full)` for complete proof, `@verify(partial)` for
//! best-effort checking, and `@verify(assume)` for trusted assertions. When the
//! solver finds a counterexample, it is included in the error diagnostic. Cost
//! transparency annotations (`@cost(O(n))`) are also verified at this level.
//!
//! Errors at this level indicate verification failures during compile-time proof checking:
//! - **SMT solver timeouts** - solver couldn't complete in time
//! - **Proof obligations unsatisfied** - code doesn't meet preconditions
//! - **Verification counterexamples** - SMT found a case violating assertions
//! - **Insufficient annotations** - need more `@verify` directives
//!
//! # When These Errors Occur
//!
//! Level 1 errors happen at compile-time when you use `@verify` annotations:
//!
//! ```rust,ignore
//! fn divide(a: i32, b: i32) -> i32
//! @verify(b != 0)  // Must prove b is non-zero
//! {
//!     a / b  // Error if b could be 0
//! }
//! ```
//!
//! # Recovery Strategies
//!
//! If verification fails:
//! 1. **Strengthen preconditions** - add more constraints
//! 2. **Add invariants** - help the solver with hints
//! 3. **Use @unchecked** - explicitly mark as trusted (requires safety proof)
//! 4. **Relax assertions** - if constraint was too strict
//! 5. **Enable manual verification** - let proof system guide implementation
//!
//! # Integration with Refinement Types
//!
//! Level 1 verification works with refinement types:
//!
//! ```rust,ignore
//! type Positive = Int{> 0};
//!
//! fn divide(a: i32, b: Positive) -> i32
//! // b is proven > 0 by refinement type, no @verify needed
//! {
//!     a / b  // Safe
//! }
//! ```
//!
//! These are compile-time errors that indicate the compiler could not
//! prove the code satisfies its formal specifications.

use crate::{ErrorKind, VerumError};
use verum_common::{List, Text};

/// SMT verification error
///
/// Indicates the SMT solver could not verify a property.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Verification failed: {property}")]
pub struct VerificationError {
    /// Property that failed verification
    pub property: Text,
    /// Counterexample (if found)
    pub counterexample: Option<Text>,
    /// Verification trace
    pub trace: Option<List<Text>>,
}

impl VerificationError {
    /// Create a new verification error
    pub fn new(property: impl Into<Text>) -> Self {
        Self {
            property: property.into(),
            counterexample: None,
            trace: None,
        }
    }

    /// Add a counterexample
    pub fn with_counterexample(mut self, counterexample: impl Into<Text>) -> Self {
        self.counterexample = Some(counterexample.into());
        self
    }

    /// Add verification trace
    pub fn with_trace(mut self, trace: List<Text>) -> Self {
        self.trace = Some(trace);
        self
    }
}

impl From<VerificationError> for VerumError {
    fn from(err: VerificationError) -> Self {
        let mut message = format!("Verification failed: {}", err.property);
        if let Some(ce) = err.counterexample {
            message.push_str(&format!("\nCounterexample: {}", ce));
        }
        VerumError::new(Text::from(message), ErrorKind::Verification)
    }
}

/// Proof obligation not satisfied
#[derive(Debug, Clone, thiserror::Error)]
#[error("Proof obligation not satisfied: {obligation}")]
pub struct ProofError {
    /// Proof obligation description
    pub obligation: Text,
    /// Why it failed
    pub reason: Option<Text>,
}

impl ProofError {
    /// Create a new proof error
    pub fn new(obligation: impl Into<Text>) -> Self {
        Self {
            obligation: obligation.into(),
            reason: None,
        }
    }

    /// Add failure reason
    pub fn with_reason(mut self, reason: impl Into<Text>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl From<ProofError> for VerumError {
    fn from(err: ProofError) -> Self {
        let mut message = format!("Proof obligation not satisfied: {}", err.obligation);
        if let Some(reason) = err.reason {
            message.push_str(&format!(" ({})", reason));
        }
        VerumError::new(Text::from(message), ErrorKind::Proof)
    }
}
