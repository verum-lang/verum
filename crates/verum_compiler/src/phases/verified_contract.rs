//! Verified contract representation for Phase 3a → Phase 4 handoff
//!
//! This module defines the data structures for passing verified contracts
//! from the Contract Verification phase to the Semantic Analysis phase.

use std::collections::HashMap;
use verum_ast::Span;
use verum_smt::{ContractSpec, ProofResult};
use verum_common::{List, Text};

/// A contract that has been successfully verified by the SMT solver
#[derive(Debug, Clone)]
pub struct VerifiedContract {
    /// Name of the function or type this contract applies to
    pub target_name: Text,

    /// The contract specification (pre/post conditions, invariants)
    pub spec: ContractSpec,

    /// Proof result with timing information
    pub proof: ProofResult,

    /// Source span of the contract
    pub span: Span,

    /// What kind of target this contract applies to
    pub target_kind: ContractTarget,
}

/// Kind of target a contract applies to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractTarget {
    /// Function precondition/postcondition
    Function,

    /// Type invariant
    Type,

    /// Loop invariant
    Loop,

    /// Protocol method contract
    ProtocolMethod,
}

impl VerifiedContract {
    /// Create a new verified contract
    pub fn new(
        target_name: Text,
        spec: ContractSpec,
        proof: ProofResult,
        span: Span,
        target_kind: ContractTarget,
    ) -> Self {
        Self {
            target_name,
            spec,
            proof,
            span,
            target_kind,
        }
    }

    /// Get a summary of the verification
    pub fn summary(&self) -> Text {
        format!(
            "{} contract for '{}': {} preconditions, {} postconditions, {} invariants (verified in {:.2}ms)",
            match self.target_kind {
                ContractTarget::Function => "Function",
                ContractTarget::Type => "Type",
                ContractTarget::Loop => "Loop",
                ContractTarget::ProtocolMethod => "Protocol",
            },
            self.target_name,
            self.spec.preconditions.len(),
            self.spec.postconditions.len(),
            self.spec.invariants.len(),
            self.proof.cost.duration.as_millis()
        )
        .into()
    }

    /// Check if this contract has any preconditions
    pub fn has_preconditions(&self) -> bool {
        !self.spec.preconditions.is_empty()
    }

    /// Check if this contract has any postconditions
    pub fn has_postconditions(&self) -> bool {
        !self.spec.postconditions.is_empty()
    }

    /// Check if this contract has any invariants
    pub fn has_invariants(&self) -> bool {
        !self.spec.invariants.is_empty()
    }
}

/// Registry of verified contracts for semantic analysis
///
/// Optimized with HashMap for O(1) lookup by function/type name.
#[derive(Debug, Clone, Default)]
pub struct VerifiedContractRegistry {
    /// All verified contracts (ordered for iteration)
    contracts: List<VerifiedContract>,

    /// Function contracts indexed by name (O(1) lookup)
    function_index: HashMap<String, Vec<usize>>,

    /// Type invariants indexed by type name (O(1) lookup)
    type_index: HashMap<String, Vec<usize>>,

    /// Protocol contracts indexed by protocol.method name (O(1) lookup)
    protocol_index: HashMap<String, Vec<usize>>,
}

impl VerifiedContractRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            contracts: List::new(),
            function_index: HashMap::new(),
            type_index: HashMap::new(),
            protocol_index: HashMap::new(),
        }
    }

    /// Add a verified contract to the registry
    pub fn register(&mut self, contract: VerifiedContract) {
        let index = self.contracts.len();
        let name_key = contract.target_name.to_string();

        // Index by contract target kind for O(1) lookup
        match contract.target_kind {
            ContractTarget::Function => {
                self.function_index
                    .entry(name_key)
                    .or_insert_with(Vec::new)
                    .push(index);
            }
            ContractTarget::Type => {
                self.type_index
                    .entry(name_key)
                    .or_insert_with(Vec::new)
                    .push(index);
            }
            ContractTarget::ProtocolMethod => {
                self.protocol_index
                    .entry(name_key)
                    .or_insert_with(Vec::new)
                    .push(index);
            }
            ContractTarget::Loop => {
                // Loop invariants are not indexed by name
                // They're associated with specific code locations
            }
        }

        self.contracts.push(contract);
    }

    /// Get all contracts for a specific function (O(1) lookup)
    pub fn get_function_contracts(&self, name: &Text) -> List<&VerifiedContract> {
        if let Some(indices) = self.function_index.get(name.as_str()) {
            indices
                .iter()
                .filter_map(|&i| self.contracts.get(i))
                .collect()
        } else {
            List::new()
        }
    }

    /// Get all type invariants for a specific type (O(1) lookup)
    pub fn get_type_invariants(&self, name: &Text) -> List<&VerifiedContract> {
        if let Some(indices) = self.type_index.get(name.as_str()) {
            indices
                .iter()
                .filter_map(|&i| self.contracts.get(i))
                .collect()
        } else {
            List::new()
        }
    }

    /// Get all protocol contracts for a specific protocol method (O(1) lookup)
    pub fn get_protocol_contracts(&self, full_name: &Text) -> List<&VerifiedContract> {
        if let Some(indices) = self.protocol_index.get(full_name.as_str()) {
            indices
                .iter()
                .filter_map(|&i| self.contracts.get(i))
                .collect()
        } else {
            List::new()
        }
    }

    /// Get all verified contracts
    pub fn all_contracts(&self) -> &List<VerifiedContract> {
        &self.contracts
    }

    /// Get total number of verified contracts
    pub fn count(&self) -> usize {
        self.contracts.len()
    }

    /// Get statistics
    pub fn stats(&self) -> RegistryStats {
        let mut stats = RegistryStats::default();

        for contract in &self.contracts {
            match contract.target_kind {
                ContractTarget::Function => stats.function_contracts += 1,
                ContractTarget::Type => stats.type_invariants += 1,
                ContractTarget::Loop => stats.loop_invariants += 1,
                ContractTarget::ProtocolMethod => stats.protocol_contracts += 1,
            }

            stats.total_preconditions += contract.spec.preconditions.len();
            stats.total_postconditions += contract.spec.postconditions.len();
            stats.total_invariants += contract.spec.invariants.len();
        }

        stats
    }
}

/// Statistics about verified contracts in the registry
#[derive(Debug, Clone, Default)]
pub struct RegistryStats {
    /// Number of function contracts
    pub function_contracts: usize,
    /// Number of type invariants
    pub type_invariants: usize,
    /// Number of loop invariants
    pub loop_invariants: usize,
    /// Number of protocol contracts
    pub protocol_contracts: usize,
    /// Total preconditions
    pub total_preconditions: usize,
    /// Total postconditions
    pub total_postconditions: usize,
    /// Total invariants
    pub total_invariants: usize,
}

impl RegistryStats {
    /// Format as a human-readable summary
    pub fn summary(&self) -> Text {
        format!(
            "Verified Contracts Summary:\n\
             - Functions: {} contracts\n\
             - Types: {} invariants\n\
             - Loops: {} invariants\n\
             - Protocols: {} contracts\n\
             - Total: {} preconditions, {} postconditions, {} invariants",
            self.function_contracts,
            self.type_invariants,
            self.loop_invariants,
            self.protocol_contracts,
            self.total_preconditions,
            self.total_postconditions,
            self.total_invariants
        )
        .into()
    }
}
