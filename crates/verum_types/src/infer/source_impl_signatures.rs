//! Method conformance is checked in the implementation's declaration frame,
//! before that implementation can provide protocol evidence.
use super::{TypeChecker, protocol_methods::AppliedProtocolMethod};
use crate::{
    Result, Type, TypeError,
    protocol::{ProtocolBound, ProtocolConstraints, ProtocolImpl, ProtocolMethod, WhereClause},
    ty::{Substitution, TypeVar},
};
use verum_ast::{
    Span,
    decl::{ImplDecl, ImplItemKind},
};
use verum_common::{List, Map, Text};

fn invalid(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E427".into(),
        msg: message.into(),
        span,
    }
}

impl TypeChecker {
    /// Lazy and local implementations introduce their own binders. Evidence
    /// about a suspended caller cannot discharge their receiver constraints.
    pub(super) fn in_impl_proof_scope<R>(
        &mut self,
        check: impl FnOnce(&mut Self) -> Result<R>,
    ) -> Result<R> {
        let protocols = std::mem::take(&mut self.source_type_registration.projection_assumptions);
        let callables = std::mem::take(&mut self.source_type_registration.callable_assumptions);
        let result = check(self);
        self.source_type_registration.projection_assumptions = protocols;
        self.source_type_registration.callable_assumptions = callables;
        result
    }

    pub(super) fn enter_source_impl_conditions(
        &mut self,
        source: &ImplDecl,
        owner: &indexmap::IndexMap<Text, Type>,
    ) -> Result<(List<WhereClause>, Map<TypeVar, Type>)> {
        let (clauses, callables) = self.elaborate_source_impl_conditions(source, owner)?;
        self.source_type_registration
            .projection_assumptions
            .extend(clauses.iter().cloned());
        self.source_type_registration.callable_assumptions.extend(
            callables
                .iter()
                .map(|(variable, contract)| (Type::Var(*variable), contract.clone())),
        );
        Ok((clauses, callables))
    }

    /// One declaration-owned carrier for conditions at every source impl
    /// publication boundary. A compound subject is an obligation too; it is
    /// never narrowed to just the subset that spells a bare parameter name.
    pub(super) fn elaborate_source_impl_conditions(
        &mut self,
        source: &ImplDecl,
        owner: &indexmap::IndexMap<Text, Type>,
    ) -> Result<(List<WhereClause>, Map<TypeVar, Type>)> {
        use verum_ast::ty::{GenericParamKind, WherePredicateKind};
        let previous_self = self.current_self_type.clone();
        let assumption_depth = self.source_type_registration.projection_assumptions.len();
        let previous_source = std::mem::replace(&mut self.resolving_source_signature, true);
        self.push_decl_param_frame(owner.clone());
        let result = (|| {
            let receiver = match &source.kind {
                verum_ast::decl::ImplKind::Protocol { for_type, .. }
                | verum_ast::decl::ImplKind::Inherent(for_type) => for_type,
            };
            let receiver = self.ast_to_type_for_protocol_impl(receiver)?;
            self.set_current_self_type(Some(receiver));
            let mut clauses = List::new();
            let mut callable_bounds = Map::new();
            for parameter in &source.generics {
                if let GenericParamKind::Type { name, bounds, .. } = &parameter.kind {
                    let subject = owner.get(&name.name).ok_or_else(|| invalid(
                        parameter.span, "implementation condition lacks its declaration binder",
                    ))?;
                    self.append_source_impl_conditions(subject, bounds, owner, &mut clauses, &mut callable_bounds)?;
                }
            }
            for clause in [source.generic_where_clause.as_ref(), source.meta_where_clause.as_ref()]
                .into_iter().flatten()
            {
                for predicate in &clause.predicates {
                    let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                        return Err(invalid(predicate.span,
                            "implementation predicate has no checked condition carrier"));
                    };
                    let subject = self.ast_to_type(ty)?;
                    self.append_source_impl_conditions(&subject, bounds, owner, &mut clauses, &mut callable_bounds)?;
                }
            }
            Ok((clauses, callable_bounds))
        })();
        self.pop_decl_param_frame();
        self.resolving_source_signature = previous_source;
        self.set_current_self_type(previous_self);
        self.source_type_registration.projection_assumptions.truncate(assumption_depth);
        result
    }

    fn append_source_impl_conditions(
        &mut self,
        subject: &Type,
        bounds: &[verum_ast::ty::TypeBound],
        owner: &indexmap::IndexMap<Text, Type>,
        clauses: &mut List<WhereClause>,
        callable_bounds: &mut Map<TypeVar, Type>,
    ) -> Result<()> {
        use verum_ast::ty::{TypeBoundKind, TypeKind};
        let mut protocols = List::new();
        for bound in bounds {
            match &bound.kind {
                TypeBoundKind::Equality(ty) | TypeBoundKind::GenericProtocol(ty)
                    if matches!(ty.kind, TypeKind::Function { .. }) =>
                {
                    let Type::Var(parameter) = subject else {
                        return Err(invalid(bound.span,
                            "callable implementation condition requires an owned type parameter"));
                    };
                    if !owner.values().any(|ty| ty == subject) {
                        return Err(invalid(bound.span,
                            "callable implementation condition captures an undeclared parameter"));
                    }
                    let required = self.ast_to_type(ty)?;
                    if let Some(previous) = callable_bounds.get(parameter)
                        && !self.protocol_checker.read().types_structurally_equal(previous, &required)
                    {
                        return Err(invalid(bound.span,
                            "multiple callable conditions need a represented intersection"));
                    }
                    callable_bounds.insert(*parameter, required);
                }
                // This shared elaborator refuses unrepresented equalities and
                // dependent predicates instead of silently weakening the impl.
                _ => protocols.extend(self.elaborate_family_bounds(std::slice::from_ref(bound))?),
            }
        }
        if !protocols.is_empty() {
            let clause = WhereClause { ty: subject.clone(), bounds: protocols };
            self.source_type_registration.projection_assumptions.push(clause.clone());
            clauses.push(clause);
        }
        Ok(())
    }

    pub(super) fn check_source_impl_signatures(
        &mut self,
        source: &ImplDecl,
        implementation: &ProtocolImpl,
        owner: &indexmap::IndexMap<Text, Type>,
    ) -> Result<()> {
        let name = self.path_to_string(&implementation.protocol);
        self.push_decl_param_frame(owner.clone());
        let checked = (|| {
            for item in &source.items {
                if !self.cfg_evaluator.should_include(&item.attributes) {
                    continue;
                }
                let ImplItemKind::Function(function) = &item.kind else {
                    continue;
                };
                // Preserve the origin of Self projections until associated
                // assignments are selected. A protocol argument may later equal
                // the receiver while belonging to a different protocol.
                let self_markers = [TypeVar::fresh(), TypeVar::fresh()];
                let self_type = Type::Var(self_markers[0]);
                let obligations = self.applied_protocol_methods_in_hierarchy(
                    &name,
                    &implementation.protocol_args,
                    &self_type,
                    &implementation.for_type,
                    &function.name.name,
                    function.span,
                )?;
                if obligations.is_empty() {
                    continue;
                }
                let actual = self.elaborate_protocol_method(
                    &[],
                    &ProtocolConstraints::Unknown,
                    function,
                    true,
                )?;
                let actual =
                    self.applied_protocol_method(&actual, &[], &Type::Var(self_markers[1]))?;
                for owner in &obligations {
                    self.check_source_method(
                        &owner.method,
                        &actual,
                        implementation,
                        owner,
                        self_markers,
                        function.span,
                    )?;
                }
            }
            Ok(())
        })();
        self.pop_decl_param_frame();
        checked
    }

    fn check_source_method(
        &self,
        expected: &ProtocolMethod,
        actual: &ProtocolMethod,
        implementation: &ProtocolImpl,
        owner: &AppliedProtocolMethod,
        self_markers: [TypeVar; 2],
        span: Span,
    ) -> Result<()> {
        if expected.receiver_kind() != actual.receiver_kind()
            || expected.is_async != actual.is_async
        {
            return Err(invalid(
                span,
                format!(
                    "method `{}` changes its declared receiver or async mode",
                    expected.name
                ),
            ));
        }
        let expected_scope = expected
            .scope
            .as_ref()
            .ok_or_else(|| invalid(span, "protocol method has no owned local telescope"))?;
        let actual_scope = actual
            .scope
            .as_ref()
            .ok_or_else(|| invalid(span, "implementation method has no owned local telescope"))?;
        if expected_scope.binders.local.len() != actual_scope.binders.local.len() {
            return Err(invalid(
                span,
                "implementation method changes its declared local telescope",
            ));
        }
        let mut alpha = Substitution::new();
        for (actual, expected) in actual_scope
            .binders
            .local
            .iter()
            .zip(&expected_scope.binders.local)
        {
            alpha.insert(*actual, Type::Var(*expected));
        }
        let actual_type = actual
            .ty
            .try_apply_subst(&alpha)
            .map_err(|e| invalid(span, e))?;
        let mut assumptions = implementation.where_clauses.clone();
        for (variable, bounds) in &expected_scope.protocol_bounds {
            assumptions.push(WhereClause {
                ty: Type::Var(*variable),
                bounds: bounds.clone(),
            });
        }
        assumptions.extend(expected_scope.where_clauses.iter().cloned());
        let mut receiver_scope = Substitution::new();
        for variable in self_markers {
            receiver_scope.insert(variable, implementation.for_type.clone());
        }
        for assumption in &mut assumptions {
            assumption.ty = assumption
                .ty
                .try_apply_subst(&receiver_scope)
                .map_err(|e| invalid(span, e))?;
            assumption.bounds = assumption
                .bounds
                .iter()
                .map(|bound| bound.try_apply_subst(&receiver_scope))
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| invalid(span, e))?;
        }
        // Implementation-local requirements must already be available to a
        // caller of the protocol method. Declaration spans and binder spelling
        // do not identify a bound, and dropping a requirement is permissible.
        let mut declared = implementation.where_clauses.clone();
        declared.extend(expected_scope.where_clauses.iter().cloned());
        for (variable, bounds) in &expected_scope.protocol_bounds {
            declared.push(WhereClause {
                ty: Type::Var(*variable),
                bounds: bounds.clone(),
            });
        }
        let mut required = actual_scope.where_clauses.clone();
        for (variable, bounds) in &actual_scope.protocol_bounds {
            required.push(WhereClause {
                ty: Type::Var(*variable),
                bounds: bounds.clone(),
            });
        }
        for clause in required {
            let clause = clause
                .try_apply_subst(&alpha)
                .map_err(|e| invalid(span, e))?;
            for bound in &clause.bounds {
                let mut available = false;
                for declaration in &declared {
                    if !self
                        .source_method_carriers_equal(
                            &declaration.ty,
                            &clause.ty,
                            implementation,
                            owner,
                            self_markers,
                            &assumptions,
                            0,
                        )
                        .map_err(|e| invalid(span, e))?
                    {
                        continue;
                    }
                    for candidate in &declaration.bounds {
                        if self
                            .source_method_bound_covers(
                                candidate,
                                bound,
                                implementation,
                                owner,
                                self_markers,
                                &assumptions,
                            )
                            .map_err(|e| invalid(span, e))?
                        {
                            available = true;
                            break;
                        }
                    }
                    if available {
                        break;
                    }
                }
                if !available {
                    return Err(invalid(
                        span,
                        "implementation method strengthens its declared protocol bounds",
                    ));
                }
            }
        }
        for (expected_name, actual_name) in expected
            .type_param_names
            .iter()
            .zip(&actual.type_param_names)
        {
            if let Some(actual_bounds) = actual.type_param_bounds.get(actual_name) {
                for actual in actual_bounds {
                    let actual = actual
                        .try_apply_subst(&alpha)
                        .map_err(|e| invalid(span, e))?;
                    let mut available = false;
                    if let Some(expected_bounds) = expected.type_param_bounds.get(expected_name) {
                        for expected in expected_bounds {
                            if self
                                .source_method_carriers_equal(
                                    expected,
                                    &actual,
                                    implementation,
                                    owner,
                                    self_markers,
                                    &assumptions,
                                    0,
                                )
                                .map_err(|e| invalid(span, e))?
                            {
                                available = true;
                                break;
                            }
                        }
                    }
                    if !available {
                        return Err(invalid(
                            span,
                            "implementation method strengthens its declared callable bounds",
                        ));
                    }
                }
            }
        }
        if !self
            .source_method_carriers_equal(
                &expected.ty,
                &actual_type,
                implementation,
                owner,
                self_markers,
                &assumptions,
                0,
            )
            .map_err(|e| invalid(span, e))?
        {
            return Err(invalid(
                span,
                format!(
                    "method `{}` has carrier `{actual_type}`, but its instantiated protocol declares `{}`",
                    expected.name, expected.ty
                ),
            ));
        }
        Ok(())
    }

    fn source_method_bound_covers(
        &self,
        declared: &ProtocolBound,
        required: &ProtocolBound,
        implementation: &ProtocolImpl,
        owner: &AppliedProtocolMethod,
        self_markers: [TypeVar; 2],
        assumptions: &[WhereClause],
    ) -> std::result::Result<bool, Text> {
        if declared.protocol.to_string() != required.protocol.to_string()
            || declared.is_negative != required.is_negative
            || declared.args.len() != required.args.len()
        {
            return Ok(false);
        }
        for (declared, required) in declared.args.iter().zip(&required.args) {
            if !self.source_method_carriers_equal(
                declared,
                required,
                implementation,
                owner,
                self_markers,
                assumptions,
                0,
            )? {
                return Ok(false);
            }
        }
        for (name, required) in &required.associated_types {
            let Some(declared) = declared.associated_types.get(name) else {
                return Ok(false);
            };
            if !self.source_method_carriers_equal(
                declared,
                required,
                implementation,
                owner,
                self_markers,
                assumptions,
                0,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn source_impl_projection(
        &self,
        ty: &Type,
        implementation: &ProtocolImpl,
        owner: &AppliedProtocolMethod,
        self_markers: [TypeVar; 2],
        assumptions: &[WhereClause],
    ) -> std::result::Result<Option<Type>, Text> {
        let ordinary = match ty {
            Type::Generic { name, args } if name.starts_with("::") && args.len() == 1 => {
                Some((&args[0], name[2..].into(), &[][..]))
            }
            _ => None,
        };
        let Some((receiver, name, arguments)) =
            ordinary.or_else(|| super::associated_family_applications::projection_parts(ty))
        else {
            return Ok(None);
        };
        let (method_owner, method_arguments) = match receiver {
            Type::Var(variable) if *variable == self_markers[0] => {
                (&owner.owner, owner.arguments.as_slice())
            }
            Type::Var(variable) if *variable == self_markers[1] => (
                &self.path_to_string(&implementation.protocol),
                implementation.protocol_args.as_slice(),
            ),
            _ => return Ok(None),
        };
        let mut receiver_scope = Substitution::new();
        for variable in self_markers {
            receiver_scope.insert(variable, implementation.for_type.clone());
        }
        let arguments = arguments
            .iter()
            .map(|argument| argument.try_apply_subst(&receiver_scope))
            .collect::<std::result::Result<List<_>, _>>()?;
        self.protocol_checker
            .read()
            .source_impl_projection(
                implementation,
                method_owner,
                method_arguments,
                &name,
                &arguments,
                assumptions,
            )
            .map(Some)
    }

    /// Equality of instantiated carriers never uses assignment coercion or
    /// wildcard unification. In particular neither Never nor a local Var can
    /// identify two different declared carriers.
    fn source_method_carriers_equal(
        &self,
        expected: &Type,
        actual: &Type,
        implementation: &ProtocolImpl,
        owner: &AppliedProtocolMethod,
        self_markers: [TypeVar; 2],
        assumptions: &[WhereClause],
        depth: usize,
    ) -> std::result::Result<bool, Text> {
        if depth >= 64 {
            return Err("method carrier comparison exceeded its structural depth budget".into());
        }
        let receiver_leaf = |ty: &Type| match ty {
            Type::Var(variable) if self_markers.contains(variable) => {
                self.unifier.normalize_aliases(&implementation.for_type)
            }
            _ => self.unifier.normalize_aliases(ty),
        };
        let expected = receiver_leaf(expected);
        let actual = receiver_leaf(actual);
        if let Some(resolved) = self.source_impl_projection(
            &expected,
            implementation,
            owner,
            self_markers,
            assumptions,
        )? {
            return self.source_method_carriers_equal(
                &resolved,
                &actual,
                implementation,
                owner,
                self_markers,
                assumptions,
                depth + 1,
            );
        }
        if let Some(resolved) =
            self.source_impl_projection(&actual, implementation, owner, self_markers, assumptions)?
        {
            return self.source_method_carriers_equal(
                &expected,
                &resolved,
                implementation,
                owner,
                self_markers,
                assumptions,
                depth + 1,
            );
        }
        let same = |a: &Type, b: &Type| {
            self.source_method_carriers_equal(
                a,
                b,
                implementation,
                owner,
                self_markers,
                assumptions,
                depth + 1,
            )
        };
        let sequence = |a: &[Type], b: &[Type]| -> std::result::Result<bool, Text> {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.iter().zip(b) {
                if !same(a, b)? {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        Ok(match (&expected, &actual) {
            (
                Type::Function {
                    params: ep,
                    return_type: er,
                    contexts: ec,
                    type_params: et,
                    property_requirement: eq,
                    ..
                },
                Type::Function {
                    params: ap,
                    return_type: ar,
                    contexts: ac,
                    type_params: at,
                    property_requirement: aq,
                    ..
                },
            ) => ec == ac && et == at && eq == aq && sequence(ep, ap)? && same(er, ar)?,
            (Type::Tuple(e), Type::Tuple(a)) => sequence(e, a)?,
            (Type::Generic { name: en, args: ea }, Type::Generic { name: an, args: aa }) => {
                en == an && sequence(ea, aa)?
            }
            (Type::Named { path: en, args: ea }, Type::Named { path: an, args: aa }) => {
                // Applied Self and source annotations may retain different
                // path representations of the same declared owner.
                let same_owner = self.protocol_checker.read().paths_equal(en, an);
                same_owner && sequence(ea, aa)?
            }
            (
                Type::TypeApp {
                    constructor: ec,
                    args: ea,
                },
                Type::TypeApp {
                    constructor: ac,
                    args: aa,
                },
            ) => same(ec, ac)? && sequence(ea, aa)?,
            (
                Type::Reference {
                    mutable: em,
                    inner: e,
                },
                Type::Reference {
                    mutable: am,
                    inner: a,
                },
            )
            | (
                Type::CheckedReference {
                    mutable: em,
                    inner: e,
                },
                Type::CheckedReference {
                    mutable: am,
                    inner: a,
                },
            )
            | (
                Type::UnsafeReference {
                    mutable: em,
                    inner: e,
                },
                Type::UnsafeReference {
                    mutable: am,
                    inner: a,
                },
            )
            | (
                Type::Ownership {
                    mutable: em,
                    inner: e,
                },
                Type::Ownership {
                    mutable: am,
                    inner: a,
                },
            )
            | (
                Type::Pointer {
                    mutable: em,
                    inner: e,
                },
                Type::Pointer {
                    mutable: am,
                    inner: a,
                },
            )
            | (
                Type::VolatilePointer {
                    mutable: em,
                    inner: e,
                },
                Type::VolatilePointer {
                    mutable: am,
                    inner: a,
                },
            ) => em == am && same(e, a)?,
            (
                Type::Array {
                    element: e,
                    size: es,
                },
                Type::Array {
                    element: a,
                    size: as_,
                },
            ) => es == as_ && same(e, a)?,
            (Type::Slice { element: e }, Type::Slice { element: a })
            | (Type::GenRef { inner: e }, Type::GenRef { inner: a })
            | (Type::Future { output: e }, Type::Future { output: a }) => same(e, a)?,
            (
                Type::Generator {
                    yield_ty: ey,
                    return_ty: er,
                },
                Type::Generator {
                    yield_ty: ay,
                    return_ty: ar,
                },
            ) => same(ey, ay)? && same(er, ar)?,
            (Type::Record(e), Type::Record(a)) | (Type::Variant(e), Type::Variant(a)) => {
                if e.len() != a.len() {
                    false
                } else {
                    let mut equal = true;
                    for (name, expected) in e {
                        match a.get(name) {
                            Some(actual) if same(expected, actual)? => {}
                            _ => {
                                equal = false;
                                break;
                            }
                        }
                    }
                    equal
                }
            }
            (Type::Forall { vars: ev, body: eb }, Type::Forall { vars: av, body: ab })
                if ev.len() == av.len() =>
            {
                let mut expected_alpha = Substitution::new();
                let mut actual_alpha = Substitution::new();
                for (e, a) in ev.iter().zip(av) {
                    let rigid = Type::Var(TypeVar::fresh());
                    expected_alpha.insert(*e, rigid.clone());
                    actual_alpha.insert(*a, rigid);
                }
                same(
                    &eb.try_apply_subst(&expected_alpha)?,
                    &ab.try_apply_subst(&actual_alpha)?,
                )?
            }
            (Type::Exists { var: ev, body: eb }, Type::Exists { var: av, body: ab }) => {
                let rigid = Type::Var(TypeVar::fresh());
                let mut expected_alpha = Substitution::new();
                expected_alpha.insert(*ev, rigid.clone());
                let mut actual_alpha = Substitution::new();
                actual_alpha.insert(*av, rigid);
                same(
                    &eb.try_apply_subst(&expected_alpha)?,
                    &ab.try_apply_subst(&actual_alpha)?,
                )?
            }
            (
                Type::Refined {
                    base: eb,
                    predicate: ep,
                },
                Type::Refined {
                    base: ab,
                    predicate: ap,
                },
            ) => {
                // Declaration locations do not constrain a refinement domain.
                // Require both checked implications: a subtype alone cannot
                // replace the protocol's domain. The shared proof projection
                // owns binder and literal identity and refuses unknown atoms.
                same(eb, ab)?
                    && (ep == ap
                        || (ep.provenance == ap.provenance
                            && crate::refinement_entailment::entails(eb, ep, ab, ap)
                            && crate::refinement_entailment::entails(ab, ap, eb, ep)))
            },
            _ => self
                .protocol_checker
                .read()
                .types_structurally_equal(&expected, &actual),
        })
    }
}
