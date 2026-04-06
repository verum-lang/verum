//! Contract Integration Helper
//!
//! This module provides helper functions for integrating verified contracts
//! between Phase 3a (Contract Verification) and Phase 4 (Semantic Analysis).

use std::time::Duration;
use verum_ast::Span;
use verum_smt::{ContractSpec, ProofResult, VerificationCost};
use verum_common::Text;

use crate::phases::{ContractTarget, VerifiedContract, VerifiedContractRegistry};

/// Create a verified contract from verification results
pub fn create_verified_contract(
    target_name: Text,
    spec: ContractSpec,
    duration: Duration,
    success: bool,
    span: Span,
    target_kind: ContractTarget,
) -> VerifiedContract {
    let cost = VerificationCost::new(target_name.clone(), duration, success);
    let proof = ProofResult::new(cost);

    VerifiedContract::new(target_name, spec, proof, span, target_kind)
}

/// Register a successfully verified function contract
pub fn register_function_contract(
    registry: &mut VerifiedContractRegistry,
    func_name: &str,
    spec: ContractSpec,
    duration: Duration,
    span: Span,
) {
    let contract = create_verified_contract(
        Text::from(func_name),
        spec,
        duration,
        true,
        span,
        ContractTarget::Function,
    );

    registry.register(contract);
}

/// Register a successfully verified type invariant
pub fn register_type_invariant(
    registry: &mut VerifiedContractRegistry,
    type_name: &str,
    spec: ContractSpec,
    duration: Duration,
    span: Span,
) {
    let contract = create_verified_contract(
        Text::from(type_name),
        spec,
        duration,
        true,
        span,
        ContractTarget::Type,
    );

    registry.register(contract);
}

/// Register a successfully verified protocol method contract
pub fn register_protocol_contract(
    registry: &mut VerifiedContractRegistry,
    protocol_name: &str,
    method_name: &str,
    spec: ContractSpec,
    duration: Duration,
    span: Span,
) {
    let full_name = format!("{}.{}", protocol_name, method_name);
    let contract = create_verified_contract(
        Text::from(full_name),
        spec,
        duration,
        true,
        span,
        ContractTarget::ProtocolMethod,
    );

    registry.register(contract);
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::Span;

    #[test]
    fn test_create_verified_contract() {
        let spec = ContractSpec::new(Span::dummy());
        let contract = create_verified_contract(
            "test_func".into(),
            spec,
            Duration::from_millis(10),
            true,
            Span::dummy(),
            ContractTarget::Function,
        );

        assert_eq!(contract.target_name.as_str(), "test_func");
        assert_eq!(contract.target_kind, ContractTarget::Function);
    }

    #[test]
    fn test_register_function_contract() {
        let mut registry = VerifiedContractRegistry::new();
        let spec = ContractSpec::new(Span::dummy());

        register_function_contract(
            &mut registry,
            "test_func",
            spec,
            Duration::from_millis(5),
            Span::dummy(),
        );

        assert_eq!(registry.count(), 1);
        let contracts = registry.get_function_contracts(&"test_func".into());
        assert_eq!(contracts.len(), 1);
    }
}
