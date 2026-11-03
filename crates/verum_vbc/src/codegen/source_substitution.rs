//! Declaration-owned type substitutions during source elaboration.
//!
//! A substituted binder has no callable witness slot. Temporary indices exist
//! only inside this elaborator and are eliminated by the shared, capture-avoiding
//! substitution before a TypeRef can reach metadata or an instruction.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{CodegenError, CodegenErrorKind, CodegenResult, ImplReceiverWitness};
use crate::{
    mono::TypeSubstitution,
    types::{TypeParamId, TypeRef},
};
use verum_ast::{FunctionDecl, ty::GenericParamKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SourceTypeSubstitution {
    bindings: BTreeMap<String, ImplReceiverWitness>,
}

impl SourceTypeSubstitution {
    pub(super) fn receiver(name: String, witness: ImplReceiverWitness) -> Self {
        Self {
            bindings: BTreeMap::from([(name, witness)]),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub(super) fn get(&self, name: &str) -> Option<&ImplReceiverWitness> {
        self.bindings.get(name)
    }

    pub(super) fn insert(&mut self, name: String, witness: ImplReceiverWitness) {
        self.bindings.insert(name, witness);
    }

    /// Function-owned binders shadow only the corresponding lexical binding.
    /// Locally quantified type binders are handled by normal AST elaboration.
    pub(super) fn for_function(&self, function: &FunctionDecl) -> Self {
        let mut result = self.clone();
        for parameter in &function.generics {
            let name = match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Context { name }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::Level { name }
                | GenericParamKind::KindAnnotated { name, .. } => name,
                GenericParamKind::Lifetime { .. } => continue,
            };
            result.bindings.remove(name.name.as_str());
        }
        result
    }

    pub(super) fn elaborate(
        &self,
        scope: &HashMap<String, u16>,
        lower: impl FnOnce(&HashMap<String, u16>) -> CodegenResult<TypeRef>,
    ) -> CodegenResult<TypeRef> {
        if self.bindings.is_empty() {
            return lower(scope);
        }
        // Also reserve every index in replacement types, including local ones.
        // This conservative superset prevents private indices from occurring in
        // a replacement; none can survive a simultaneous substitution.
        let mut reserved: BTreeSet<u16> = scope.values().copied().collect();
        for witness in self.bindings.values() {
            reserve_indices(&witness.ty, &mut reserved);
        }
        let mut scope = scope.clone();
        let mut substitution = TypeSubstitution::empty();
        let mut next = 0u32;
        for (name, witness) in &self.bindings {
            while u16::try_from(next).is_ok_and(|index| reserved.contains(&index)) {
                next += 1;
            }
            let index = u16::try_from(next).map_err(|_| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "source type substitution exceeds the VBC binder index space".into(),
                ))
            })?;
            next += 1;
            scope.insert(name.clone(), index);
            substitution.bind(TypeParamId(index), witness.ty.clone());
        }
        substitution.apply(&lower(&scope)?).map_err(|error| {
            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(error.to_string()))
        })
    }
}

fn reserve_indices(ty: &TypeRef, reserved: &mut BTreeSet<u16>) {
    match ty {
        TypeRef::Generic(id) => {
            reserved.insert(id.0);
        }
        TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
            for arg in args {
                reserve_indices(arg, reserved);
            }
        }
        TypeRef::Function {
            params,
            return_type,
            ..
        }
        | TypeRef::Rank2Function {
            params,
            return_type,
            ..
        } => {
            for parameter in params {
                reserve_indices(parameter, reserved);
            }
            reserve_indices(return_type, reserved);
        }
        TypeRef::Reference { inner, .. }
        | TypeRef::Slice(inner)
        | TypeRef::Array { element: inner, .. }
        | TypeRef::AssociatedProjection { base: inner, .. } => reserve_indices(inner, reserved),
        TypeRef::Concrete(_) | TypeRef::ConstValue(_) => {}
    }
}
