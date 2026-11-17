//! Exact declaration equations and their instantiation obligations.
use super::{TypeChecker, call_obligations::CallProjectionScope};
use crate::{
    Result, TypeError,
    context::TypeEquality,
    protocol::{ProtocolBound, WhereClause},
    ty::{Substitution, SubstitutionExt, Type, TypeVar},
};
use verum_ast::{
    Span,
    ty::{TypeBound, TypeBoundKind},
};
use verum_common::{List, Map, Text};

fn invalid(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

impl TypeChecker {
    pub(super) fn declared_where_equalities(
        &mut self,
        subject: &verum_ast::Type,
        bounds: &[TypeBound],
    ) -> Result<List<TypeEquality>> {
        let mut equalities = List::new();
        for bound in bounds {
            let TypeBoundKind::Equality(expected) = &bound.kind else {
                continue;
            };
            equalities.push(TypeEquality {
                subject: self.declared_type(subject)?,
                expected: self.declared_type(expected)?,
            });
        }
        Ok(equalities)
    }

    /// The caller's own projected equations refine only the family selected
    /// by its declared protocol premises. A same-named member adds no evidence.
    fn equality_projection_premises(
        &self,
        premises: &mut List<WhereClause>,
        span: Span,
    ) -> Result<()> {
        for equality in &self.source_type_registration.type_equalities {
            let Some((receiver, member, arguments)) =
                super::associated_family_applications::checked_projection_parts(&equality.subject)
            else {
                continue;
            };
            // Applied families need their full argument telescope, which a
            // bare associated-type binding does not represent.
            if !arguments.is_empty() {
                continue;
            }
            let owner = self
                .protocol_checker
                .read()
                .associated_projection_owner(receiver, &member, premises)
                .map_err(|message| invalid(span, message))?;
            let Some(mut bound) = owner else { continue };
            if let Some(previous) = bound
                .associated_types
                .insert(member, equality.expected.clone())
                && !self.types_equivalent(&previous, &equality.expected)
            {
                return Err(invalid(
                    span,
                    "contradictory declared associated type equality",
                ));
            }
            premises.push(WhereClause {
                ty: receiver.clone(),
                bounds: List::from_single(bound),
            });
        }
        Ok(())
    }

    /// Equations may rename a caller binder or fix its type, but cannot solve
    /// unrelated rigid parameters. This substitution is local proof evidence;
    /// it never mutates the caller's inference state or published signature.
    fn caller_equality_substitution(&self, span: Span) -> Result<Substitution> {
        let mut substitution = Substitution::new();
        for equality in &self.source_type_registration.type_equalities {
            let left = equality
                .subject
                .try_apply_subst(&substitution)
                .map_err(|message| invalid(span, message))?;
            let right = equality
                .expected
                .try_apply_subst(&substitution)
                .map_err(|message| invalid(span, message))?;
            if self.types_equivalent(&left, &right) {
                continue;
            }
            let (variable, value) = match (&left, &right) {
                (Type::Var(variable), value) | (value, Type::Var(variable)) => (*variable, value),
                _ => continue,
            };
            if value.free_vars().contains(&variable) {
                return Err(invalid(span, "recursive declared type equality"));
            }
            let replacement = [(variable, value.clone())].into_iter().collect();
            substitution = substitution
                .compose(&replacement)
                .map_err(|message| invalid(span, message))?;
        }
        Ok(substitution)
    }

    /// Check values under this body's declared equations without solving its
    /// rigid binders in the shared unifier or rewriting a published header.
    pub(super) fn type_under_caller_equalities(&self, ty: &Type, span: Span) -> Result<Type> {
        if self.source_type_registration.type_equalities.is_empty() {
            return Ok(ty.clone());
        }
        self.unifier
            .try_apply(ty, span)?
            .try_apply_subst(&self.caller_equality_substitution(span)?)
            .map_err(|message| invalid(span, message))
    }

    /// An equation fixes a fresh callee slot before ordinary argument
    /// conversions run. This is inference, not proof: concrete/rigid operands
    /// and all associated-family obligations still reach the strict check.
    pub(super) fn seed_call_type_equalities(
        &mut self,
        equalities: &[TypeEquality],
        variables: &[TypeVar],
        span: Span,
    ) -> Result<()> {
        let saved = self.unifier.save_substitution();
        let result = (|| {
            for equality in equalities {
                let left = self.unifier.try_apply(&equality.subject, span)?;
                let right = self.unifier.try_apply(&equality.expected, span)?;
                let owned = |variable: &TypeVar| {
                    variables.contains(variable) && !self.unifier.is_rigid(*variable)
                };
                let (variable, value) = match (&left, &right) {
                    (Type::Var(variable), value) if owned(variable) => (*variable, value),
                    (value, Type::Var(variable)) if owned(variable) => (*variable, value),
                    _ => continue,
                };
                // A family is selected after its receiver is known; its
                // application is not an injective type constructor. In
                // particular F.Item=F does not justify solving F=F.Item.
                if value.free_vars().contains(&variable)
                    || value.contains_type(|ty| {
                        matches!(ty, Type::Unknown | Type::RecordField { .. })
                            || super::associated_family_applications::checked_projection_parts(ty)
                                .is_some()
                    })
                {
                    continue;
                }
                self.unifier.unify(&Type::Var(variable), value, span)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.unifier.restore_substitution(saved);
        }
        result
    }

    pub(super) fn enforce_call_type_equalities(
        &mut self,
        equalities: &[TypeEquality],
        bounds: &Map<TypeVar, List<ProtocolBound>>,
        clauses: &[WhereClause],
        span: Span,
    ) -> Result<()> {
        if equalities.is_empty() {
            return Ok(());
        }
        let saved = self.unifier.save_substitution();
        let rigid = self.unifier.rigid_snapshot();
        // A bound is not permission to identify a universally quantified
        // caller parameter with a callee's required concrete type.
        self.unifier.enter_rigid_scope(
            rigid
                .iter()
                .map(|(variable, info)| (*variable, info.name.clone(), false, None)),
        );
        let result = (|| {
            let mut caller = self.caller_protocol_premises();
            self.equality_projection_premises(&mut caller, span)?;
            let caller_equalities = self.caller_equality_substitution(span)?;
            let declaration = super::call_obligations::declaration_premises(bounds, clauses);
            for equality in equalities {
                let arguments = self.unifier.save_substitution();
                let scope = CallProjectionScope {
                    declaration: &declaration,
                    caller: Some(&caller),
                    arguments: &arguments,
                    span,
                };
                let left = self
                    .resolve_call_output_projections(&equality.subject, &scope)?
                    .try_apply_subst(&caller_equalities)
                    .map_err(|message| invalid(span, message))?;
                let right = self
                    .resolve_call_output_projections(&equality.expected, &scope)?
                    .try_apply_subst(&caller_equalities)
                    .map_err(|message| invalid(span, message))?;
                if left.contains_type(|ty| matches!(ty, Type::Unknown))
                    || right.contains_type(|ty| matches!(ty, Type::Unknown))
                {
                    return Err(invalid(
                        span,
                        "unknown type cannot establish a call equality",
                    ));
                }
                if !self.types_equivalent(&left, &right) {
                    self.unifier.unify(&left, &right, span)?;
                }
                let left = self.unifier.try_apply(&left, span)?;
                let right = self.unifier.try_apply(&right, span)?;
                // Unification may admit conversions. Exact equations require
                // the same normalized type after inference has finished.
                if !self.types_equivalent(&left, &right) {
                    return Err(invalid(
                        span,
                        format!("call does not establish type equality `{left} = {right}`"),
                    ));
                }
            }
            Ok(())
        })();
        self.unifier.exit_rigid_scope(rigid);
        if result.is_err() {
            self.unifier.restore_substitution(saved);
        }
        result
    }
}
