//! Instantiation and discharge of declaration-owned call obligations.

use super::TypeChecker;
use crate::context::{InstantiatedScheme, TypeScheme};
use crate::ty::{Type, TypeVar};
use crate::{Result, TypeError};
use verum_ast::Span;
use verum_common::{List, Map, Text, ToText};

pub(super) struct PendingValueObligation {
    type_bounds: Map<TypeVar, List<Type>>,
    protocol_bounds: Map<TypeVar, List<crate::protocol::ProtocolBound>>,
    where_clauses: List<crate::protocol::WhereClause>,
    span: Span,
}

impl TypeChecker {
    /// Nested entry points reuse a frame; a named function owns a fresh one.
    /// Restore the parent even when inference or obligation checking fails.
    pub(super) fn with_value_obligation_scope<T>(
        &mut self,
        force_nested: bool,
        infer: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if !force_nested && self.value_obligation_scope_depth != 0 {
            return infer(self);
        }
        let previous_depth = self.value_obligation_scope_depth;
        let depth = previous_depth
            .checked_add(1)
            .ok_or_else(|| TypeError::Other("value-obligation scope depth exhausted".into()))?;
        let parent = std::mem::take(&mut self.pending_value_obligations);
        self.value_obligation_scope_depth = depth;
        let result = infer(self).and_then(|value| {
            self.discharge_value_obligations()?;
            Ok(value)
        });
        self.pending_value_obligations = parent;
        self.value_obligation_scope_depth = previous_depth;
        result
    }

    /// Instantiate a value once, retaining its obligations until the
    /// surrounding inference frame has constrained its arguments and result.
    pub(super) fn instantiate_value_scheme(
        &mut self,
        scheme: &TypeScheme,
        span: Span,
    ) -> Result<Type> {
        self.instantiate_value_instance(scheme, span)
            .map(|instance| instance.ty)
    }

    pub(super) fn instantiate_value_instance(
        &mut self,
        scheme: &TypeScheme,
        span: Span,
    ) -> Result<InstantiatedScheme> {
        let instance = scheme
            .try_instantiate()
            .map_err(|message| TypeError::UniverseBinding { message, span })?;
        self.retain_value_instance_obligations(&instance, span)?;
        Ok(instance)
    }

    /// A selected method candidate retains the same fresh signature and
    /// requirements that participated in overload resolution.
    pub(super) fn retain_value_instance_obligations(
        &mut self,
        instance: &InstantiatedScheme,
        span: Span,
    ) -> Result<()> {
        if !instance.type_bounds.is_empty()
            || !instance.protocol_bounds.is_empty()
            || !instance.where_clauses.is_empty()
        {
            if self.value_obligation_scope_depth == 0 {
                return Err(TypeError::UniverseBinding {
                    message: "bounded value instantiation requires an inference obligation scope"
                        .into(),
                    span,
                });
            }
            for (var, bounds) in &instance.type_bounds {
                for bound in bounds {
                    self.register_type_var_type_bound(*var, bound.clone());
                }
            }
            self.pending_value_obligations.push(PendingValueObligation {
                type_bounds: instance.type_bounds.clone(),
                protocol_bounds: instance.protocol_bounds.clone(),
                where_clauses: instance.where_clauses.clone(),
                span,
            });
        }
        Ok(())
    }

    pub(super) fn discharge_value_obligations(&mut self) -> Result<()> {
        // Resolution may lazily check another declaration. Its forced frame
        // is isolated; any obligations added to this frame must also be checked.
        while !self.pending_value_obligations.is_empty() {
            let pending = std::mem::take(&mut self.pending_value_obligations);
            for obligation in pending {
                self.enforce_call_type_bounds(&obligation.type_bounds, obligation.span)?;
                self.enforce_call_protocol_bounds(&obligation.protocol_bounds, obligation.span)?;
                self.enforce_call_where_clauses(&obligation.where_clauses, obligation.span)?;
            }
        }
        Ok(())
    }

    pub(super) fn instantiate_call_scheme(
        &mut self,
        scheme: &TypeScheme,
        span: Span,
    ) -> Result<InstantiatedScheme> {
        let instance = scheme
            .try_instantiate()
            .map_err(|message| TypeError::UniverseBinding { message, span })?;
        if !instance.where_clauses.is_empty() {
            if self.value_obligation_scope_depth == 0 {
                return Err(TypeError::UniverseBinding {
                    message: "conditional call requires an inference obligation scope".into(),
                    span,
                });
            }
            self.pending_value_obligations.push(PendingValueObligation {
                type_bounds: Map::new(),
                protocol_bounds: Map::new(),
                where_clauses: instance.where_clauses.clone(),
                span,
            });
        }
        // These are hints for checking closures. They do not discharge the
        // obligations, which are checked after all arguments have been inferred.
        for (var, bounds) in &instance.type_bounds {
            for bound in bounds {
                self.register_type_var_type_bound(*var, bound.clone());
            }
        }
        Ok(instance)
    }

    /// Conditional method requirements remain obligations of this call. They
    /// are proved from the caller's declared assumptions and actual impls, not
    /// introduced as new assumptions or attached to every implementation.
    pub(super) fn enforce_call_where_clauses(
        &mut self,
        clauses: &[crate::protocol::WhereClause],
        span: Span,
    ) -> Result<()> {
        if clauses.is_empty() {
            return Ok(());
        }
        let substitution = self.unifier.save_substitution();
        let normalize = |checker: &Self, ty: &Type| -> Result<Type> {
            let ty = ty
                .try_apply_subst(&substitution)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
            Ok(checker.resolve_assoc_projections_deep(&ty))
        };
        let mut assumptions = List::new();
        for (variable, bounds) in &self.type_var_bounds {
            // Only active universal caller binders introduce assumptions. A
            // flexible call variable carries obligations, never its own proof.
            if !self.unifier.is_rigid(*variable) {
                continue;
            }
            assumptions.push(crate::protocol::WhereClause {
                ty: normalize(self, &Type::Var(*variable))?,
                bounds: bounds
                    .iter()
                    .map(|bound| {
                        bound
                            .try_apply_subst(&substitution)
                            .map_err(|message| TypeError::UniverseBinding { message, span })
                    })
                    .collect::<Result<_>>()?,
            });
        }
        for clause in clauses {
            let subject = normalize(self, &clause.ty)?;
            if let Some(owner) = self.get_type_name(&subject) {
                self.ensure_stdlib_type_loaded_transitive(&owner);
            }
            let bounds: List<_> = clause
                .bounds
                .iter()
                .map(|bound| {
                    let mut bound = bound
                        .try_apply_subst(&substitution)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?;
                    bound.args = bound
                        .args
                        .iter()
                        .map(|ty| normalize(self, ty))
                        .collect::<Result<_>>()?;
                    for ty in bound.associated_types.values_mut() {
                        *ty = normalize(self, ty)?;
                    }
                    Ok(bound)
                })
                .collect::<Result<_>>()?;
            for bound in &bounds {
                self.ensure_stdlib_type_loaded_transitive(&self.path_to_string(&bound.protocol));
            }
            self.protocol_checker
                .read()
                .check_universal_bounds(&subject, &bounds, &assumptions)
                .map_err(|reason| TypeError::OtherWithCodeSpanned {
                    code: "E405".into(),
                    msg: format!(
                        "cannot establish method call requirement for `{subject}`: {reason}"
                    )
                    .into(),
                    span,
                })?;
        }
        Ok(())
    }

    /// Shared legacy protocol-bound check for calls and value instances.
    /// Unresolved variables retain the existing deferral behavior; returning
    /// Ok is not a certificate that every unresolved protocol bound was proved.
    /// Concrete failures continue to emit error diagnostics as before.
    pub(super) fn enforce_call_protocol_bounds(
        &mut self,
        protocol_bounds: &Map<TypeVar, List<crate::protocol::ProtocolBound>>,
        span: Span,
    ) -> Result<()> {
        for (type_var, bounds) in protocol_bounds {
            let resolved_ty = self.unifier.apply(&Type::Var(*type_var));
            self.enforce_protocol_bound_evidence(&resolved_ty, bounds, span)?;
            let substitution = self.unifier.save_substitution();
            let resolved_bounds: List<_> = bounds
                .iter()
                .map(|bound| bound.apply_subst(&substitution))
                .collect();
            let bounds = &resolved_bounds;
            // Only check bounds when the type var resolved to a concrete type
            // (not still a type variable — those are checked at their own call sites)
            //
            // "Still a type variable" has to look THROUGH reference
            // layers. `&_` is `Reference { inner: Var }`, which is not
            // `Type::Var(_)`, so a top-level-only test let it past and
            // the checker announced that `&_` does not implement `Fn`
            // — a verdict about a type it does not know. Five such
            // diagnostics appeared in `core/math/calculus.vr` the
            // moment `where`-clause bounds started reaching this
            // check, all of them on `&_`, none of them a real
            // conformance gap. Saying "does not implement" about an
            // unknown type is the same error as saying it about a
            // module path nobody recognises: absence of evidence.
            let mut unresolved_core = &resolved_ty;
            loop {
                match unresolved_core {
                    Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. }
                    | Type::Pointer { inner, .. }
                    | Type::VolatilePointer { inner, .. }
                    | Type::Ownership { inner, .. } => unresolved_core = inner,
                    _ => break,
                }
            }
            // HKT-BOUND-ASKS-THE-CONSTRUCTOR (T1075).  A bound on
            // `F<_>` resolves to a TYPE CONSTRUCTOR (`Bad`, arity 1),
            // not to an inhabited type.  `check_bounds` keys impls by
            // the WHOLE type, and a constructor's key is `tycon:Bad`,
            // which never equals the key of
            // `implement<T> Mappable for Good<T>` — so it answered
            // "does not implement" for EVERY constructor, the
            // satisfying ones included.  Measured: once the bound was
            // merely recorded, BOTH the valid and the violating call
            // were refused, which is not a bound check.
            //
            // A `F<_>` bound can only ever be about the constructor's
            // NAME, and `type_constructor_implements_protocol` is the
            // authority that compares by name across `TypeConstructor`,
            // `Named` and `Generic` impl heads.  Load first, then ask:
            // a constructor reaching a bound has usually never been a
            // method receiver, so its impls are not registered yet
            // (the receiver-oriented lazy load, T0701).
            if let Type::TypeConstructor { name, .. } = unresolved_core {
                self.ensure_stdlib_type_loaded_transitive(name);
                for b in bounds.iter() {
                    let Some(proto_ident) = b.protocol.as_ident() else {
                        continue;
                    };
                    let proto: Text = proto_ident.as_str().into();
                    self.ensure_stdlib_type_loaded_transitive(&proto);
                    let holds = self
                        .protocol_checker
                        .read()
                        .type_constructor_implements_protocol(name, &proto);
                    if holds != b.is_negative {
                        continue;
                    }
                    let msg = if b.is_negative {
                        format!(
                            "type constructor `{}` must NOT implement `{}` here, but it does",
                            name, proto
                        )
                    } else {
                        format!(
                            "type constructor `{}` does not implement `{}`, required by \
                             this call's bound",
                            name, proto
                        )
                    };
                    self.push_diagnostic_for(TypeError::OtherWithCodeSpanned {
                        code: Text::from("E405"),
                        msg: Text::from(msg),
                        span: span,
                    });
                }
                continue;
            }
            if !matches!(unresolved_core, Type::Var(_)) {
                // LAZY-LOAD FIRST, THEN JUDGE. The verdict used to be
                // thrown away into `tracing::debug!` with the note
                // «some stdlib impls may not yet be loaded depending on
                // compilation order. The runtime will catch actual
                // violations». Both halves were wrong: for a statically
                // typed language the runtime must not be the arbiter of
                // a compile-time bound, and the load-order worry has a
                // real fix — ensure the receiver's impls are loaded and
                // re-ask. Silencing it made EVERY generic bound
                // decorative: `fn f<T: Ord>(…)` accepted a type with no
                // `Ord` and no `cmp` at all, and `T: Semiring` accepted
                // a plain record (measured on 2026-08-07).
                let first = self
                    .protocol_checker
                    .read()
                    .check_bounds(&resolved_ty, bounds);
                // Retry AFTER ensuring the receiver's and the
                // protocols' impls are loaded: the lazy stdlib loader
                // is receiver-driven, so a bound can look unsatisfied
                // purely because nothing had asked for that type yet.
                // (This load-order worry is why the verdict used to be
                // discarded entirely — the fix is to remove the
                // uncertainty, not the check.)
                let satisfied = if first.is_err() {
                    let leaf: Text = self.get_type_name(&resolved_ty).unwrap_or_default();
                    if leaf.as_str().is_empty() {
                        first
                    } else {
                        self.ensure_stdlib_type_loaded_transitive(&leaf);
                        for b in bounds.iter() {
                            let rendered = format!("{}", b.protocol);
                            let proto_leaf: Text = rendered
                                .rsplit('.')
                                .next()
                                .unwrap_or(rendered.as_str())
                                .into();
                            self.ensure_stdlib_type_loaded_transitive(&proto_leaf);
                        }
                        self.protocol_checker
                            .read()
                            .check_bounds(&resolved_ty, bounds)
                    }
                } else {
                    first
                };
                if let Err(e) = satisfied {
                    let (proto, is_negative) = match &e {
                        crate::protocol::ProtocolError::BoundNotSatisfied { protocol, .. } => {
                            (Text::from(format!("{}", protocol)), false)
                        }
                        crate::protocol::ProtocolError::NegativeBoundViolated {
                            protocol, ..
                        } => (Text::from(format!("{}", protocol)), true),
                        _ => (Text::from(""), false),
                    };
                    if !proto.as_str().is_empty() {
                        let msg = if is_negative {
                            format!(
                                "type `{}` must NOT implement `{}` here, but it does",
                                resolved_ty, proto
                            )
                        } else {
                            format!(
                                "type `{}` does not implement `{}`, required by this \
                                 call's bound",
                                resolved_ty, proto
                            )
                        };
                        self.push_diagnostic_for(TypeError::OtherWithCodeSpanned {
                            code: Text::from("E405"),
                            msg: Text::from(msg),
                            span: span,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn enforce_call_type_bounds(
        &mut self,
        bounds: &Map<TypeVar, List<Type>>,
        span: Span,
    ) -> Result<()> {
        let saved = self.unifier.save_substitution();
        for (var, obligations) in bounds {
            for obligation in obligations {
                if let Err(error) =
                    self.check_call_type_bound(&Type::Var(*var), obligation, span, 0)
                {
                    self.unifier.restore_substitution(saved);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(super) fn check_call_type_bound(
        &mut self,
        actual: &Type,
        bound: &Type,
        span: Span,
        depth: usize,
    ) -> Result<()> {
        if depth >= 64 {
            return Err(TypeError::Other(
                "call-bound checking exceeds its depth budget".into(),
            ));
        }
        let actual = self.unifier.try_apply(actual, span)?;
        let bound = self.unifier.try_apply(bound, span)?;
        if actual == bound {
            return Ok(());
        }
        let actual = if let Type::Var(var) = &actual {
            self.unifier.rigid_fn_bound(*var).unwrap_or(actual)
        } else {
            actual
        };
        let actual = self.resolve_assoc_projections_deep(&actual);
        let actual = self.normalize_type(&actual);
        let bound = self.resolve_assoc_projections_deep(&bound);
        let bound = self.normalize_type(&bound);
        if let (
            Type::Function {
                property_requirement: actual_property_requirement,
                params: actual_params,
                return_type: actual_return,
                type_params: actual_generics,
                contexts: actual_contexts,
                properties: actual_properties,
            },
            Type::Function {
                property_requirement: bound_property_requirement,
                params: bound_params,
                return_type: bound_return,
                type_params: bound_generics,
                contexts: bound_contexts,
                properties: bound_properties,
            },
        ) = (&actual, &bound)
        {
            // Check the callable contract before variance-aware inference. Do
            // not let structural unification erase contexts or properties.
            let actual_contract = Type::Function {
                property_requirement: actual_property_requirement.clone(),
                params: List::new(),
                return_type: Box::new(Type::Unit),
                type_params: actual_generics.clone(),
                contexts: actual_contexts.clone(),
                properties: actual_properties.clone(),
            };
            let bound_contract = Type::Function {
                property_requirement: bound_property_requirement.clone(),
                params: List::new(),
                return_type: Box::new(Type::Unit),
                type_params: bound_generics.clone(),
                contexts: bound_contexts.clone(),
                properties: bound_properties.clone(),
            };
            if actual_params.len() != bound_params.len()
                || !self.subtyping.is_subtype(&actual_contract, &bound_contract)
            {
                return Err(TypeError::Mismatch {
                    expected: bound.to_text(),
                    actual: actual.to_text(),
                    span,
                });
            }
            for (actual_param, bound_param) in actual_params.iter().zip(bound_params) {
                self.check_call_type_bound(bound_param, actual_param, span, depth + 1)?;
            }
            // A callable constraint with no result consumes the invocation,
            // discarding its value; this does not change F's actual type.
            if !matches!(**bound_return, Type::Unit) {
                self.check_call_type_bound(actual_return, bound_return, span, depth + 1)?;
            }
            return Ok(());
        }
        if self.subtyping.is_subtype(&actual, &bound)
            || self.check_protocol_coercion(&actual, &bound)
        {
            return Ok(());
        }
        // A callable bound permits invoking F, not identifying universal F
        // with a particular function carrier. Only inference variables may
        // be solved while checking this directional obligation.
        let rigid = self.unifier.rigid_snapshot();
        self.unifier.enter_rigid_scope(
            rigid
                .iter()
                .map(|(var, info)| (*var, info.name.clone(), false, None)),
        );
        let inferred = self.unifier.unify(&actual, &bound, span);
        self.unifier.exit_rigid_scope(rigid);
        inferred?;
        // Unification solves inference variables; it is not independently a
        // proof of compatibility (e.g. nested callable property contracts).
        let actual = self.unifier.try_apply(&actual, span)?;
        let bound = self.unifier.try_apply(&bound, span)?;
        if self.subtyping.is_subtype(&actual, &bound)
            || self.check_protocol_coercion(&actual, &bound)
        {
            Ok(())
        } else {
            Err(TypeError::Mismatch {
                expected: bound.to_text(),
                actual: actual.to_text(),
                span,
            })
        }
    }
}
