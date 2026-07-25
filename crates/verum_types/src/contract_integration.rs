//! Contract Integration for Type Checking
//!
//! This module enables the type checker to leverage verified contracts from Phase 3a
//! to strengthen type inference and validate call sites.
//!
//! ## Features
//!
//! - Store verified contracts from SMT-based verification phase
//! - Strengthen parameter types using verified preconditions
//! - Strengthen return types using verified postconditions
//! - Validate call sites against function preconditions
//! - Check return values against function postconditions
//!
//! ## Integration Flow
//!
//! 1. Phase 3a verifies contracts with Z3
//! 2. Verified contracts passed to Phase 4 via VerifiedContractRegistry
//! 3. TypeChecker stores registry and uses it during type checking
//! 4. Call sites and returns are validated against contracts
//!
//! Verification system: three levels - @verify(runtime) for assertions, @verify(static) for dataflow analysis, @verify(proof) for SMT-based proofs — Contract integration
//! Compilation pipeline: parse -> type check -> verify -> lower -> codegen phases — Phase 4

use verum_common::{List, Text};

/// Placeholder for VerifiedContract (will be imported from verum_compiler)
///
/// In practice, verum_types should not depend on verum_compiler.
/// Instead, we use a minimal trait-based interface to avoid circular dependencies.
pub trait VerifiedContractLike {
    /// Get the target name (function/type name)
    fn target_name(&self) -> &Text;

    /// Check if contract has preconditions
    fn has_preconditions(&self) -> bool;

    /// Check if contract has postconditions
    fn has_postconditions(&self) -> bool;

    /// Check if contract has invariants
    fn has_invariants(&self) -> bool;
}

/// Registry of verified contracts for type checking
///
/// This is a trait to avoid circular dependencies between verum_types and verum_compiler.
/// The actual implementation lives in verum_compiler/src/phases/verified_contract.rs
pub trait VerifiedContractRegistryLike {
    /// Get all contracts for a specific function
    fn get_function_contracts(&self, name: &Text) -> List<&dyn VerifiedContractLike>;

    /// Get all type invariants for a specific type
    fn get_type_invariants(&self, name: &Text) -> List<&dyn VerifiedContractLike>;

    /// Get total number of verified contracts
    fn count(&self) -> usize;
}

/// Contract-aware type checking context
///
/// This structure is added to TypeChecker to enable contract-based type strengthening.
#[derive(Debug, Clone)]
pub struct ContractContext {
    /// Whether contracts are enabled for this type checking session
    pub enabled: bool,

    /// Number of contracts available (for metrics)
    pub contract_count: usize,

    /// Number of precondition checks performed
    pub precondition_checks: usize,

    /// Number of postcondition checks performed
    pub postcondition_checks: usize,
}

impl ContractContext {
    /// Create a new contract context
    pub fn new(enabled: bool, contract_count: usize) -> Self {
        Self {
            enabled,
            contract_count,
            precondition_checks: 0,
            postcondition_checks: 0,
        }
    }

    /// Create a disabled contract context (no contracts available)
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            contract_count: 0,
            precondition_checks: 0,
            postcondition_checks: 0,
        }
    }

    /// Record a precondition check
    pub fn record_precondition_check(&mut self) {
        self.precondition_checks += 1;
    }

    /// Record a postcondition check
    pub fn record_postcondition_check(&mut self) {
        self.postcondition_checks += 1;
    }
}

impl Default for ContractContext {
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_context_creation() {
        let ctx = ContractContext::new(true, 10);
        assert!(ctx.enabled);
        assert_eq!(ctx.contract_count, 10);
        assert_eq!(ctx.precondition_checks, 0);
    }

    #[test]
    fn test_contract_context_disabled() {
        let ctx = ContractContext::disabled();
        assert!(!ctx.enabled);
        assert_eq!(ctx.contract_count, 0);
    }

    #[test]
    fn test_contract_context_tracking() {
        let mut ctx = ContractContext::new(true, 5);
        ctx.record_precondition_check();
        ctx.record_precondition_check();
        ctx.record_postcondition_check();

        assert_eq!(ctx.precondition_checks, 2);
        assert_eq!(ctx.postcondition_checks, 1);
    }
}
