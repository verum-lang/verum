//! Declaration-owned type substitutions during source elaboration.
//!
//! A substituted binder has no callable witness slot. Temporary indices exist
//! only inside this elaborator and are eliminated by the shared, capture-avoiding
//! substitution before a TypeRef can reach metadata or an instruction.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{CodegenError, CodegenErrorKind, CodegenResult, ImplReceiverWitness};
use crate::{
    mono::TypeSubstitution,
    types::{TypeId, TypeParamId, TypeRef},
};
use verum_ast::{Expr, ExprKind, FunctionDecl, ty::GenericParamKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SourceTypeSubstitution {
    bindings: BTreeMap<String, SourceBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceBinding {
    /// A declared compile-time level has no runtime witness.
    Level,
    Type(ImplReceiverWitness),
    Value {
        witness: ImplReceiverWitness,
        value_type: TypeRef,
    },
}

impl SourceBinding {
    fn witness(&self) -> Option<&ImplReceiverWitness> {
        match self {
            Self::Type(witness) | Self::Value { witness, .. } => Some(witness),
            Self::Level => None,
        }
    }
}

impl SourceTypeSubstitution {
    pub(super) fn receiver(name: String, witness: ImplReceiverWitness) -> Self {
        Self {
            bindings: BTreeMap::from([(name, SourceBinding::Type(witness))]),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub(super) fn get(&self, name: &str) -> Option<&ImplReceiverWitness> {
        self.bindings.get(name).and_then(SourceBinding::witness)
    }

    pub(super) fn has_level(&self, name: &str) -> bool {
        matches!(self.bindings.get(name), Some(SourceBinding::Level))
    }

    pub(super) fn insert(&mut self, name: String, witness: ImplReceiverWitness) {
        self.bindings.insert(name, SourceBinding::Type(witness));
    }

    pub(super) fn get_value(&self, name: &str) -> Option<(&ImplReceiverWitness, &TypeRef)> {
        match self.bindings.get(name)? {
            SourceBinding::Value {
                witness,
                value_type,
            } => Some((witness, value_type)),
            SourceBinding::Type(_) | SourceBinding::Level => None,
        }
    }

    pub(super) fn value_witness_domain(&self, witness: &TypeRef) -> Option<&TypeRef> {
        self.bindings.values().find_map(|binding| match binding {
            SourceBinding::Value { witness: owned, value_type } if owned.semantic && &owned.ty == witness => Some(value_type),
            _ => None,
        })
    }

    pub(super) fn insert_value(
        &mut self,
        name: String,
        witness: ImplReceiverWitness,
        value_type: TypeRef,
    ) {
        self.bindings.insert(
            name,
            SourceBinding::Value {
                witness,
                value_type,
            },
        );
    }

    /// Function-owned binders shadow only the corresponding lexical binding.
    /// Locally quantified type binders are handled by normal AST elaboration.
    pub(super) fn for_function(&self, function: &FunctionDecl) -> Self {
        self.for_parameters(&function.generics)
    }

    pub(super) fn for_parameters(&self, parameters: &[verum_ast::ty::GenericParam]) -> Self {
        let mut result = self.clone();
        for parameter in parameters {
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
            if matches!(parameter.kind, GenericParamKind::Level { .. }) {
                result.bindings.insert(name.name.to_string(), SourceBinding::Level);
            }
        }
        result
    }

    pub(super) fn elaborate(
        &self,
        scope: &HashMap<String, u16>,
        lower: impl FnOnce(&HashMap<String, u16>, &SourceValueScope) -> CodegenResult<TypeRef>,
    ) -> CodegenResult<TypeRef> {
        if self.bindings.is_empty() {
            return lower(scope, &SourceValueScope::default());
        }
        // Also reserve every index in replacement types, including local ones.
        // This conservative superset prevents private indices from occurring in
        // a replacement; none can survive a simultaneous substitution.
        let mut reserved: BTreeSet<u16> = scope.values().copied().collect();
        for binding in self.bindings.values() {
            if let Some(witness) = binding.witness() {
                reserve_indices(&witness.ty, &mut reserved);
            }
        }
        let mut scope = scope.clone();
        let mut substitution = TypeSubstitution::empty();
        let mut values = SourceValueScope::default();
        let mut next = 0u32;
        for (name, binding) in &self.bindings {
            let Some(witness) = binding.witness() else {
                scope.remove(name);
                continue;
            };
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
            if let SourceBinding::Value { witness, value_type } = binding {
                let value = match &witness.ty { TypeRef::ConstValue(value) if witness.semantic && *value_type == TypeRef::Concrete(TypeId::INT) => Some(*value), _ => None };
                values.0.insert(index, value);
            }
        }
        substitution
            .apply(&lower(&scope, &values)?)
            .map_err(|error| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(error.to_string()))
            })
    }
}

/// Scalar facts are indexed by the elaborator's private binder IDs, not names.
/// Lifting this environment alongside type binders prevents capture under rank2.
#[derive(Default)]
pub(super) struct SourceValueScope(BTreeMap<u16, Option<i64>>);

impl SourceValueScope {
    pub(super) fn contains(&self, id: u16) -> bool { self.0.contains_key(&id) }

    pub(super) fn lifted(&self, count: u16, span: verum_ast::Span) -> CodegenResult<Self> {
        self.0
            .iter()
            .map(|(id, value)| {
                id.checked_add(count).map(|id| (id, *value)).ok_or_else(|| {
                    invalid(
                        "source value scope exceeds the VBC binder index space",
                        span,
                    )
                })
            })
            .collect::<CodegenResult<BTreeMap<_, _>>>()
            .map(Self)
    }

    pub(super) fn argument(
        &self,
        expr: &Expr,
        scope: &HashMap<String, u16>,
    ) -> CodegenResult<TypeRef> {
        let expr = super::VbcCodegen::normalized_place(expr);
        if let ExprKind::Path(path) = &expr.kind
            && let Some(name) = path.as_ident()
            && let Some(&id) = scope.get(name.name.as_str())
        {
            if !self.contains(id) {
                return Err(invalid("type parameter cannot occupy a value slot", expr.span));
            }
            return Ok(TypeRef::Generic(TypeParamId(id)));
        }
        checked_scalar_expression(expr, &mut |leaf| {
            let ExprKind::Path(path) = &leaf.kind else {
                return Ok(None);
            };
            Ok(path
                .as_ident()
                .and_then(|name| scope.get(name.name.as_str()))
                .and_then(|id| self.0.get(id))
                .copied().flatten())
        })
        .map(TypeRef::ConstValue)
    }
}

pub(super) fn checked_scalar_expression(
    expression: &Expr,
    resolve: &mut impl FnMut(&Expr) -> CodegenResult<Option<i64>>,
) -> CodegenResult<i64> {
    verum_ast::checked_const::validate_i64_expression(expression).map_err(|span| invalid(
        "scalar expression has an unsupported literal type or exceeds its structural limit", span,
    ))?;
    verum_ast::checked_const::evaluate(expression, 0, &mut |leaf, _| {
        resolve(leaf).map(|value| value.map(verum_ast::checked_const::Scalar::Int))
    })
    .map_err(|error| match error {
        verum_ast::checked_const::EvalError::Resolver(error) => error,
        _ => invalid(
            "value argument is not an exactly representable checked scalar expression",
            expression.span,
        ),
    })?
    .as_i64()
    .ok_or_else(|| {
        invalid(
            "value argument has an incompatible scalar type",
            expression.span,
        )
    })
}


fn invalid(message: &str, span: verum_ast::Span) -> CodegenError {
    CodegenError::with_span(
        CodegenErrorKind::InvalidSourceSignature(message.into()),
        span,
    )
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
        | TypeRef::Array { element: inner, .. } => reserve_indices(inner, reserved),
        TypeRef::AssociatedProjection { base, qualifier, args, .. } => {
            reserve_indices(base, reserved);
            for argument in args { reserve_indices(argument, reserved); }
            if let Some(application) = qualifier {
                for argument in &application.args { reserve_indices(argument, reserved); }
            }
        },
        TypeRef::Dynamic(object) => {
            for ty in object.types() { reserve_indices(ty, reserved); }
        }
        TypeRef::Concrete(_) | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) | TypeRef::RecordFieldProjection { .. } => {}
    }
}

#[cfg(test)]
mod record_projection_tests {
    use super::*;
    use crate::types::TypeId;

    #[test]
    fn record_projection_does_not_reserve_generic_indices() {
        let mut reserved = BTreeSet::new();
        reserve_indices(
            &TypeRef::Tuple(vec![
                TypeRef::RecordFieldProjection {
                    owner: TypeId(7),
                    ordinal: 0,
                },
                TypeRef::RecordFieldProjection {
                    owner: TypeId(2048),
                    ordinal: u32::MAX,
                },
                TypeRef::Generic(TypeParamId(3)),
            ]),
            &mut reserved,
        );
        assert_eq!(reserved, BTreeSet::from([3]));
    }
}

#[cfg(test)]
mod scalar_scope_tests {
    use super::*;

    #[test]
    fn scalar_scope_lifting_checks_the_entire_index_space() {
        let scope = SourceValueScope(BTreeMap::from([(u16::MAX, Some(73))]));
        assert_eq!(
            scope.lifted(0, verum_ast::Span::default()).unwrap().0,
            scope.0
        );
        assert!(scope.lifted(1, verum_ast::Span::default()).is_err());
        let scope = SourceValueScope(BTreeMap::from([(0, Some(73))]));
        let lifted = scope.lifted(1, verum_ast::Span::default()).unwrap();
        assert_eq!(lifted.0.get(&1), Some(&Some(73)));
        assert_eq!(
            lifted.0.get(&0),
            None,
            "local type slot cannot acquire an outer scalar fact"
        );
    }
}
