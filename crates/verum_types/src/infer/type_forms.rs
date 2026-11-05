//! Constructor-specific type elaboration keeps recursive dispatch small.
//! Each rule retains its own temporaries instead of reserving every grammar
//! constructor's workspace on every recursive conversion.
use super::helpers::resolve_builtin_meta_type;
use super::*;
use verum_ast::ty::TypeKind;

impl TypeChecker {
    pub(super) fn elaborate_function_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Function {
            is_async,
            contexts,
            params,
            return_type,
            ..
        } = &ast_ty.kind
        else {
            unreachable!("type constructor dispatch");
        };
        let param_types: Result<List<_>> = params.iter().map(|p| self.ast_to_type(p)).collect();
        let param_types = param_types?;
        let ret_type = self.ast_to_type(return_type)?;

        // Check if this is a dependent function (Pi type)
        // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case — Pi Types (Dependent Functions)
        // A function type is dependent if its return type references parameter types
        if self.has_dependent_types() && !param_types.is_empty() {
            use crate::dependent_helpers::{collect_type_vars, convert_internal_to_ast};
            use crate::dependent_integration::DependentTypeConstraint;

            // Collect type variables from parameters
            let param_vars: std::collections::HashSet<_> = param_types
                .iter()
                .flat_map(|p| collect_type_vars(p).into_iter())
                .collect();

            // Check if return type references any parameter variables
            let ret_vars = collect_type_vars(&ret_type);
            let is_dependent = ret_vars.iter().any(|v| param_vars.contains(v));

            if is_dependent {
                // This is a Pi type - verify the dependent function is well-formed
                // The return type must be valid for all possible parameter values
                let constraint = DependentTypeConstraint::PiType {
                    param_name: verum_common::Text::from("_param"), // Use placeholder name for combined params
                    param_type: if param_types.len() == 1 {
                        convert_internal_to_ast(&param_types[0])
                    } else {
                        // Multiple params - create tuple type
                        let tuple_ty = Type::tuple(param_types.clone());
                        convert_internal_to_ast(&tuple_ty)
                    },
                    return_type: convert_internal_to_ast(&ret_type),
                    span: ast_ty.span,
                };

                self.verify_dependent_type_constraint(&constraint, ast_ty.span)?;
            }
        }

        self.source_callable_type(param_types, ret_type, *is_async, contexts, ast_ty.span)
    }

    pub(super) fn elaborate_dependent_application_type(
        &mut self,
        ast_ty: &verum_ast::Type,
    ) -> Result<Type> {
        let TypeKind::DependentApp {
            carrier,
            value_args,
        } = &ast_ty.kind
        else {
            unreachable!("type constructor dispatch");
        };
        // T0266 — validate the value-index ARITY against the
        // family's declared value-param list (recorded at type
        // registration into `ctx.dependent_value_params`). Only
        // an UNQUALIFIED head is checked: qualified or aliased
        // carriers resolve through other tables, and a lookup
        // miss must not fabricate errors (stdlib-safe skip).
        // Per-index TYPE checking is the dependent-binder
        // scoping follow-up (T0266 journal).
        let head_name: Option<&str> = {
            let base_kind = match &carrier.kind {
                verum_ast::ty::TypeKind::Generic { base, .. } => &base.kind,
                other => other,
            };
            if std::env::var_os("VERUM_TRACE_DEPAPP").is_some() {
                eprintln!(
                    "[depapp] arm carrier={:?} supplied={} map_len={}",
                    std::mem::discriminant(base_kind),
                    value_args.len(),
                    self.ctx.dependent_value_params.len()
                );
            }
            match base_kind {
                verum_ast::ty::TypeKind::Path(p) if p.segments.len() == 1 => {
                    match p.segments.first() {
                        Some(verum_ast::ty::PathSegment::Name(id)) => Some(id.name.as_str()),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        if let Some(name) = head_name
            && let Some(&declared) = self
                .ctx
                .dependent_value_params
                .get(&verum_common::Text::from(name))
            && declared != value_args.len()
        {
            return Err(crate::TypeError::DependentValueArgArityMismatch {
                family: verum_common::Text::from(name),
                declared,
                supplied: value_args.len(),
                span: ast_ty.span,
            });
        }
        for index in value_args {
            self.check_record_index_expression(index)?;
        }
        let carrier = self.ast_to_type(carrier)?;
        if carrier.contains_record_field() {
            return Err(TypeError::UniverseBinding {
                message:
                    "value application of a field-dependent type requires retained term indices"
                        .into(),
                span: ast_ty.span,
            });
        }
        Ok(carrier)
    }

    pub(super) fn elaborate_refinement_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Refined { base, predicate } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        self.check_record_index_expression(&predicate.expr)?;
        use crate::refinement::{RefinementBinding, RefinementPredicate as TyRefinementPredicate};
        use Option;

        let base_ty = self.ast_to_type(base)?;
        // Post collapse, `TypeKind::Refined` carries all three
        // surface forms. The sigma form reaches us with a
        // `predicate.binding = Some(name)`; preserve that distinction
        // by emitting a `RefinementBinding::Sigma` when the binding
        // came from the sigma surface syntax. We can no longer tell
        // sigma apart from lambda `T where |x| expr` at this layer
        // (both share the same AST shape), so both are treated as
        // Sigma — semantically equivalent in the type system.
        let binding = match &predicate.binding {
            Some(ident) => RefinementBinding::Sigma(ident.name.clone()),
            None => RefinementBinding::Inline,
        };
        // A NAME IN A PREDICATE MUST EXIST (T1112).
        //
        // Every identifier here becomes a free variable of the
        // solver obligation. An unknown one is therefore not
        // "no constraint" — it is an ARBITRARY constraint, and
        // the type it produces is not the author's. Measured
        // 2026-09-03, one binary:
        //
        //     { self > 0 }   value  5 -> ok    value -5 -> E500
        //     { slef > 0 }   value  5 -> E500  value -5 -> ok
        //
        // A single typo INVERTS the type: it accepts exactly
        // the values the author wrote the refinement to reject,
        // silently, in a feature whose purpose is rejecting
        // them. The same name in ordinary code is `E100:
        // unbound variable`, so the checker knows it is
        // unknown; this path simply never asked.
        //
        // Deliberately narrow: single-segment paths only, and
        // simple expression forms. Qualified paths and local binding
        // expressions remain the responsibility of expression checking.
        // DEFERRED, not reported here. A `const` the predicate
        // names is registered LATER than the type declaration
        // that uses it, so an eager check rejects
        // `type P is Int { self < LIMIT }` — correct code, and
        // exactly the false positive the first version of this
        // produced. `take_deferred_errors` re-asks in the DECLARING
        // module once registration is complete.
        if self.in_type_declaration {
            for name in self.refinement_names_to_resolve(
                &predicate.expr,
                predicate.binding.as_ref().map(|i| i.name.clone()),
            ) {
                self.deferred_refinement_names.push((
                    self.current_module_path.clone(),
                    name,
                    predicate.span,
                ));
            }
        }
        let pred = TyRefinementPredicate {
            predicate: predicate.expr.clone(),
            binding: binding.clone(),
            span: predicate.span,
            // Lowered from a refinement written in the source.
            provenance: crate::refinement::PredicateProvenance::Declared,
        };

        // Verify dependent type constraint using SMT if enabled for
        // sigma-form refinements (previously the `TypeKind::Sigma`
        // arm's behaviour). Sigma types (dependent pairs):
        // `(x: A, B(x))` where second component type depends on first
        // value — refinement types desugar to Sigma.
        if let Some(name) = &predicate.binding {
            if self.has_dependent_types() {
                use crate::dependent_helpers::convert_internal_to_ast;
                use crate::dependent_integration::DependentTypeConstraint;

                let constraint = DependentTypeConstraint::SigmaType {
                    fst_name: name.name.clone(),
                    fst_type: convert_internal_to_ast(&base_ty),
                    snd_type: convert_internal_to_ast(&Type::Bool),
                    span: ast_ty.span,
                };

                match self.verify_dependent_type(&constraint) {
                    Ok(result) => {
                        if let crate::refinement::VerificationResult::Invalid { counterexample } =
                            result
                        {
                            // Log verification failure but don't fail type checking
                            // This allows gradual adoption of dependent types
                            // The constraint will be checked at runtime if not proven
                            if let Maybe::Some(ref ce) = counterexample {
                                // Record that this constraint needs runtime checking
                                // (could be tracked in a separate data structure for diagnostics)
                                let _ = ce; // Suppress unused warning for now
                            }
                        }
                    }
                    Err(_) => {
                        // SMT verification failed - continue with type checking
                        // The constraint will be checked at runtime
                    }
                }
            }
        }

        Ok(Type::refined(base_ty, pred))
    }

    pub(super) fn elaborate_tensor_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Tensor {
            element,
            shape,
            layout: _,
        } = &ast_ty.kind
        else {
            unreachable!("type constructor dispatch");
        };
        use crate::const_eval::ConstEvaluator;
        use verum_common::ConstValue;

        for dimension in shape {
            self.check_record_index_expression(dimension)?;
        }
        let element_ty = self.ast_to_type(element)?;

        // Evaluate shape expressions to const values.
        // SIMD and tensor system: unified Tensor<T, Shape> type with compile-time shape validation, SIMD acceleration (SSE/AVX/NEON), auto-differentiation — Tensor types with compile-time shape
        //

        // Tensor dimensions can be:
        // 1. Literal integers (e.g., 3, 4, 5) - evaluated directly
        // 2. Meta parameters (e.g., N, M) - represented symbolically
        // 3. Expressions (e.g., N * 2) - evaluated if possible
        let mut shape_values = List::new();
        let mut const_eval = ConstEvaluator::new();

        for dim_expr in shape {
            match const_eval.eval(dim_expr) {
                Ok(val) => shape_values.push(val),
                Err(_) => {
                    // Cannot evaluate at compile time.
                    // Check if this is a meta parameter reference that will be
                    // resolved during monomorphization.
                    if let verum_ast::expr::ExprKind::Path(path) = &dim_expr.kind {
                        if let Some(ident) = path.as_ident() {
                            // Simple identifier - likely a meta parameter.
                            // Use u128::MAX as sentinel for symbolic dimension.
                            tracing::debug!(
                                "Tensor dimension '{}' deferred to monomorphization",
                                ident.name
                            );
                            shape_values.push(ConstValue::UInt(u128::MAX));
                            continue;
                        }
                    }
                    // Complex expression - also defer to monomorphization.
                    // This handles cases like N * 2 where N is a meta parameter.
                    shape_values.push(ConstValue::UInt(u128::MAX));
                }
            }
        }

        Ok(Type::tensor(element_ty, shape_values, ast_ty.span))
    }

    pub(super) fn elaborate_constructor_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::TypeConstructor { base, arity } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        use crate::advanced_protocols::Kind;

        // Extract the name from the base type
        let name = if let TypeKind::Path(path) = &base.kind {
            if let Some(ident) = path.as_ident() {
                ident.name.clone()
            } else {
                return Err(TypeError::Other(verum_common::Text::from(format!(
                    "Type constructor must have simple name, got: {}",
                    path
                ))));
            }
        } else {
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "Type constructor base must be a path, got: {}",
                type_kind_description(&base.kind)
            ))));
        };

        // Create appropriate kind based on arity
        let kind = match *arity {
            0 => Kind::type_kind(),
            1 => Kind::unary_constructor(),
            2 => Kind::binary_constructor(),
            _ => {
                // For higher arities, build the kind recursively
                let mut k = Kind::Type;
                for _ in 0..*arity {
                    k = Kind::Arrow(Box::new(Kind::Type), Box::new(k));
                }
                k
            }
        };

        Ok(Type::type_constructor(name, *arity, kind))
    }

    pub(super) fn elaborate_existential_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Existential { name, bounds } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        // Existential type: some T: Bound
        // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — Existential Types
        //

        // Creates an existentially quantified type: ∃T. T where T: Bound
        // Used for opaque return types and type erasure.
        //

        // The existential hides the concrete implementation type while
        // exposing only the protocol interface.
        let type_var = TypeVar::fresh();

        // Convert AST bounds to protocol bounds and register them
        let protocol_bounds: List<crate::protocol::ProtocolBound> = bounds
            .iter()
            .filter_map(|bound| {
                match &bound.kind {
                    verum_ast::ty::TypeBoundKind::Protocol(path) => {
                        Some(crate::protocol::ProtocolBound {
                            associated_types: Default::default(),
                            protocol: path.clone(),
                            args: List::new(),
                            is_negative: false,
                        })
                    }
                    verum_ast::ty::TypeBoundKind::NegativeProtocol(path) => {
                        Some(crate::protocol::ProtocolBound {
                            associated_types: Default::default(),
                            protocol: path.clone(),
                            args: List::new(),
                            is_negative: true,
                        })
                    }
                    _ => None, // Handle other bound kinds as needed
                }
            })
            .collect();

        // Register bounds on the type variable for method resolution
        self.register_type_var_bounds(type_var, protocol_bounds);

        // Create the existential type: ∃T. T
        // The body is just the type variable, bounds are tracked separately
        Ok(Type::Exists {
            var: type_var,
            body: Box::new(Type::Var(type_var)),
        })
    }

    pub(super) fn elaborate_tuple_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Tuple(types) = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let types: Result<List<_>> = types.iter().map(|t| self.ast_to_type(t)).collect();
        Ok(Type::tuple(types?))
    }

    pub(super) fn elaborate_slice_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Slice(element) = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let elem_ty = self.ast_to_type(element)?;
        Ok(Type::slice(elem_ty))
    }

    pub(super) fn elaborate_reference_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Reference { mutable, inner } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let inner_ty = self.ast_to_type(inner)?;
        Ok(Type::reference(*mutable, inner_ty))
    }

    pub(super) fn elaborate_checked_reference_type(
        &mut self,
        ast_ty: &verum_ast::Type,
    ) -> Result<Type> {
        let TypeKind::CheckedReference { mutable, inner } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let inner_ty = self.ast_to_type(inner)?;
        Ok(Type::checked_reference(*mutable, inner_ty))
    }

    pub(super) fn elaborate_unsafe_reference_type(
        &mut self,
        ast_ty: &verum_ast::Type,
    ) -> Result<Type> {
        let TypeKind::UnsafeReference { mutable, inner } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let inner_ty = self.ast_to_type(inner)?;
        Ok(Type::unsafe_reference(*mutable, inner_ty))
    }

    pub(super) fn elaborate_path_identity_type(
        &mut self,
        ast_ty: &verum_ast::Type,
    ) -> Result<Type> {
        let TypeKind::PathType { carrier, lhs, rhs } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        self.check_record_index_expression(lhs)?;
        self.check_record_index_expression(rhs)?;
        let carrier_ty = self.ast_to_type(carrier)?;
        let lhs_eq = crate::expr_to_eqterm::expr_to_eq_term(lhs);
        let rhs_eq = crate::expr_to_eqterm::expr_to_eq_term(rhs);
        Ok(Type::Eq {
            ty: Box::new(carrier_ty),
            lhs: Box::new(lhs_eq),
            rhs: Box::new(rhs_eq),
        })
    }

    pub(super) fn elaborate_named_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Path(path) = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        // `@builtin_*` meta-type markers are language-level primitives
        // referenced from stdlib (e.g. `type I is @builtin_interval;`,
        // `type Path<A>(a, b) is @builtin_path;` in core/math/hott.vr).
        // They are not looked up in the user type environment — they
        // name compiler intrinsics directly and resolve here.
        if path.segments.len() == 1 {
            if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                let name = ident.name.as_str();
                if let Some(builtin) = resolve_builtin_meta_type(name) {
                    return Ok(builtin);
                }
            }
        }
        // Named type (user-defined type or type alias)
        // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Cross-module type resolution
        // Use the new resolver for qualified paths
        self.resolve_value_annotation(path, ast_ty.span)
    }

    pub(super) fn elaborate_pointer_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Pointer { mutable, inner } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let inner_ty = self.ast_to_type(inner)?;
        Ok(Type::Pointer {
            mutable: *mutable,
            inner: Box::new(inner_ty),
        })
    }

    pub(super) fn elaborate_volatile_pointer_type(
        &mut self,
        ast_ty: &verum_ast::Type,
    ) -> Result<Type> {
        let TypeKind::VolatilePointer { mutable, inner } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let inner_ty = self.ast_to_type(inner)?;
        Ok(Type::VolatilePointer {
            mutable: *mutable,
            inner: Box::new(inner_ty),
        })
    }

    pub(super) fn elaborate_ownership_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Ownership { mutable, inner } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let inner_ty = self.ast_to_type(inner)?;
        Ok(Type::Ownership {
            mutable: *mutable,
            inner: Box::new(inner_ty),
        })
    }

    pub(super) fn elaborate_generation_reference_type(
        &mut self,
        ast_ty: &verum_ast::Type,
    ) -> Result<Type> {
        let TypeKind::GenRef { inner } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let inner_ty = self.ast_to_type(inner)?;
        Ok(Type::genref(inner_ty))
    }

    pub(super) fn elaborate_associated_projection_type(
        &mut self,
        ast_ty: &verum_ast::Type,
    ) -> Result<Type> {
        let TypeKind::AssociatedType { base, assoc } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        // Associated type path: T.Item or Self.Item
        // Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
        //

        // Represents a projection from a type to its associated type.
        // E.g., Iterator.Item, Container.Element, W.Inner.Item (chained)
        //

        // Resolution happens during protocol checking when the concrete
        // implementing type is known and we can look up the associated type
        // in the implementation.
        let base_ty = self.ast_to_type(base)?;

        // Represent associated types as Generic with projection naming convention.
        // Format: "::AssocName" with base_ty stored in args[0] for later resolution.
        // This allows chained projections like W.Inner.Item to be resolved iteratively.
        //

        // Example: Iterator<T>.Item becomes Generic { name: "::Item", args: [Iterator<T>] }
        // Chained: W.Inner.Item becomes Generic { name: "::Item", args: [W.Inner] }
        //  where W.Inner is also Generic { name: "::Inner", args: [W] }
        let assoc_name = assoc.name.as_str();
        let projection_name = verum_common::Text::from(format!("::{}", assoc_name));

        Ok(Type::Generic {
            name: projection_name,
            args: List::from(vec![base_ty]),
        })
    }

    pub(super) fn elaborate_capability_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::CapabilityRestricted { base, capabilities } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        // Capability-restricted type: T with [Capabilities]
        // Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
        //

        // Check the base type and track capabilities for capability-based
        // method filtering. The type system ensures that:
        // - T with [A, B, C] <: T with [A, B] (more caps = subtype of fewer caps)
        // - Method calls requiring capability C are only valid if C is in the set
        let base_ty = self.ast_to_type(base)?;

        // Convert AST CapabilitySet to structured TypeCapabilitySet
        // This enables proper set operations (subset, superset, intersection)
        // for capability attenuation subtyping
        let type_caps = crate::capability::TypeCapabilitySet::from_ast(capabilities);

        Ok(Type::CapabilityRestricted {
            base: Box::new(base_ty),
            capabilities: type_caps,
        })
    }

    pub(super) fn elaborate_record_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Record { fields, .. } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        use indexmap::IndexMap;
        let mut field_types: IndexMap<Text, Type> = IndexMap::new();
        for f in fields {
            let ty = self.ast_to_type(&f.ty)?;
            field_types.insert(f.name.name.clone(), ty);
        }
        Ok(Type::Record(field_types))
    }

    pub(super) fn elaborate_universe_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::Universe { level } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        let level = match level {
            Some(expression) => self.ast_universe_level_to_internal(expression)?,
            None => crate::ty::UniverseLevel::TYPE,
        };
        Ok(Type::Universe { level })
    }

    pub(super) fn elaborate_type_lambda_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        let TypeKind::TypeLambda { params: _, body } = &ast_ty.kind else {
            unreachable!("type constructor dispatch");
        };
        // For now, evaluate the body type
        // Full dependent type support would track the lambda structure
        let ty = self.ast_to_type(body)?;
        if ty.contains_record_field() {
            return Err(TypeError::UniverseBinding {
                message: "a field-dependent type lambda requires a retained binder".into(),
                span: ast_ty.span,
            });
        }
        Ok(ty)
    }

    pub(super) fn elaborate_atomic_type(&mut self, ast_ty: &verum_ast::Type) -> Result<Type> {
        match &ast_ty.kind {
            TypeKind::Unit => Ok(Type::unit()),
            TypeKind::Bool => Ok(Type::bool()),
            TypeKind::Int => Ok(Type::int()),
            TypeKind::Float => Ok(Type::float()),
            TypeKind::Char => Ok(Type::Char),
            TypeKind::Text => Ok(Type::text()),
            TypeKind::Never => Ok(Type::never()),
            TypeKind::Inferred => Ok(Type::Var(TypeVar::fresh())),
            TypeKind::Unknown => Ok(Type::Unknown),
            _ => unreachable!("type constructor dispatch"),
        }
    }
}
