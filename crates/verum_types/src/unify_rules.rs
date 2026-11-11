//! Individual unification rules. Dispatch order and guards live in `unify.rs`.
//! Each rule owns its intermediate values, so recursive unification retains
//! only the selected rule's frame, not temporaries for every type constructor.
use super::*;
use Type::*;

impl Unifier {
    pub(super) fn unify_integer_nominal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Int, Named { path, .. }) | (Named { path, .. }, Int)) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        let name = path
            .segments
            .last()
            .map(|s| match s {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                _ => "",
            })
            .unwrap_or("");
        if self.is_int_coercible_named(name) {
            Ok(Substitution::new())
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_unit_nominal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Unit, Named { path, args, .. }) | (Named { path, args, .. }, Unit)) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        let name = path
            .segments
            .last()
            .map(|s| match s {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                _ => "",
            })
            .unwrap_or("");
        let name_text = verum_common::Text::from(name);
        // Check type alias registry for unit newtypes
        if let Some(resolved) = self.type_aliases.get(&name_text) {
            if *resolved == Type::Unit || matches!(resolved, Type::Tuple(elems) if elems.is_empty())
            {
                return Ok(Substitution::new());
            }
        }
        // Check variant_type_names for registered unit types
        if self
            .variant_type_names
            .get(&verum_common::Text::from("()"))
            .is_some()
        {
            // If "()" is registered as a variant type name, check if it matches
        }
        Err(TypeError::Mismatch {
            expected: t2.to_text(),
            actual: t1.to_text(),
            span,
        })
    }
    pub(super) fn unify_unbound_nominal_parameter(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Named { path, args, .. }, other) | (other, Named { path, args, .. })) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // A bare single-letter `Named` is treated as an
        // unconstrained type parameter that unifies with
        // anything.  But "unifies" must not mean "learns
        // nothing": when the counterpart is a unification
        // VARIABLE, the empty substitution left that variable
        // unbound, and a receiver like `Wrap<P>` produced
        // `Guard<_>` — "the inferred type `_` is not fully
        // determined" — for every user type whose name happens
        // to be one uppercase letter.  Six other copies of the
        // "looks like a type parameter" rule were ruled out by
        // instrumentation before this one, which carries its
        // own inline copy, was found.
        //
        // Binding the variable keeps the arm's permissiveness
        // and stops it discarding the one fact it had.
        match other {
            Var(v) => {
                let named = Named {
                    path: path.clone(),
                    args: args.clone(),
                };
                self.bind_var(*v, &named, span)
            }
            _ => Ok(Substitution::new()),
        }
    }
    pub(super) fn unify_functions(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Function {
                property_requirement: requirement1,
                params: p1,
                return_type: r1,
                type_params: tp1,
                contexts: c1,
                properties: _,
            },
            Function {
                property_requirement: requirement2,
                params: p2,
                return_type: r2,
                type_params: tp2,
                contexts: c2,
                properties: _,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if p1.len() != p2.len() || requirement1 != requirement2 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        // Type parameters must match structurally (same count and names)
        // For unification, we require exact structural match of type parameters.
        // Subtyping of bounds is handled separately in the subtype checker.
        //

        // Function type unification: parameter types unify contravariantly, return types covariantly - Function type unification
        // requires identical type parameter structure. The subtype relation allows
        // contravariance in bounds, but unification is stricter.
        //

        // Example:
        //  fn f<T: Ord>(x: T) and fn f<T: Eq>(x: T) do NOT unify
        //  But fn f<T: Ord>(x: T) <: fn f<T: Eq>(x: T) if Ord extends Eq
        if tp1.len() != tp2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        // Context unification with support for context polymorphism
        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.5 - Context Polymorphism
        //

        // Unification rules for contexts:
        // - None/None: compatible (both pure)
        // - Concrete/Concrete: must be equal
        // - Variable/X: bind variable to X (captured in context_subst)
        // - Variable/Variable: unify them
        let context_subst = self.unify_contexts(c1, c2, span)?;
        let mut subst = context_subst;

        // Unify parameters
        for (pt1, pt2) in p1.iter().zip(p2.iter()) {
            let s = self.unify_inner(
                &pt1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &pt2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }

        // Unify return types
        let s = self.unify_inner(
            &r1.try_apply_subst(&subst)
                .map_err(|message| TypeError::UniverseBinding { message, span })?,
            &r2.try_apply_subst(&subst)
                .map_err(|message| TypeError::UniverseBinding { message, span })?,
            span,
        )?;
        Ok(subst
            .compose(&s)
            .map_err(|message| TypeError::UniverseBinding { message, span })?)
    }
    pub(super) fn unify_tuples(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Tuple(t1s), Tuple(t2s)) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        if t1s.len() != t2s.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for (ty1, ty2) in t1s.iter().zip(t2s.iter()) {
            let s = self.unify_inner(
                &ty1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &ty2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }
        Ok(subst)
    }
    pub(super) fn unify_arrays(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Array {
                element: e1,
                size: s1,
            },
            Array {
                element: e2,
                size: s2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Array size unification rules:
        // 1. If both have sizes, they must match exactly
        // 2. If one has size and the other doesn't, unify with the known size
        // 3. If neither has size, they unify (both unknown)
        match (s1, s2) {
            (Some(n1), Some(n2)) if n1 != n2 => Err(TypeError::ConstMismatch {
                expected: n2.to_string().into(),
                actual: n1.to_string().into(),
                span,
            }),
            _ => {
                // Either sizes match, or at least one is None
                self.unify_inner(e1, e2, span)
            }
        }
    }
    pub(super) fn unify_array_application(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((
            Array {
                element: e1,
                size: _,
            },
            TypeApp {
                constructor,
                args: app_args,
            },
        )
        | (
            TypeApp {
                constructor,
                args: app_args,
            },
            Array {
                element: e1,
                size: _,
            },
        )) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if app_args.len() != 1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        // If constructor is a type variable, bind it to the default array-coercible
        // collection type ("List"). Uses data-driven resolution; will be replaced
        // with FromArray protocol dispatch once protocols are available.
        // NOTE: "List" is the default because it is the first entry in
        // array_coercible_types. A proper protocol-based approach would not
        // need a default at all.
        if let Type::Var(var) = constructor.as_ref() {
            let list_ctor = Type::TypeConstructor {
                name: "List".into(),
                arity: 1,
                kind: crate::advanced_protocols::Kind::unary_constructor(),
            };
            let subst = self.bind_var(*var, &list_ctor, span)?;

            // Unify the element types
            let s = self.unify_inner(
                &e1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &app_args[0]
                    .try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            Ok(subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?)
        } else if let Type::TypeConstructor { name, arity: 1, .. } = constructor.as_ref() {
            if self.is_array_coercible(name.as_str()) {
                // Already an array-coercible collection constructor, just unify element types
                self.unify_inner(e1, &app_args[0], span)
            } else {
                Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                })
            }
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_records(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Record(f1), Record(f2)) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        if f1.len() != f2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for (name, ty1) in f1 {
            match f2.get(name) {
                Some(ty2) => {
                    let s = self.unify_inner(
                        &ty1.try_apply_subst(&subst)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?,
                        &ty2.try_apply_subst(&subst)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?,
                        span,
                    )?;
                    subst = subst
                        .compose(&s)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?;
                }
                None => {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
            }
        }
        Ok(subst)
    }
    pub(super) fn unify_open_records(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            ExtensibleRecord {
                fields: f1,
                row_var: r1,
            },
            ExtensibleRecord {
                fields: f2,
                row_var: r2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let mut subst = Substitution::new();

        // First unify the common fields
        for (name, ty1) in f1 {
            if let Some(ty2) = f2.get(name) {
                let s = self.unify_inner(
                    &ty1.try_apply_subst(&subst)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?,
                    &ty2.try_apply_subst(&subst)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?,
                    span,
                )?;
                subst = subst
                    .compose(&s)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?;
            } else if r2.is_none() {
                // f1 has a field that f2 doesn't, and f2 is closed
                return Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                });
            }
            // Otherwise, the field will be captured by r2's row variable
        }

        // Check that f2's fields are also in f1 (unless f1 is open)
        for (name, _) in f2 {
            if !f1.contains_key(name) && r1.is_none() {
                return Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                });
            }
        }

        // Unify row variables if both are present
        match (r1, r2) {
            (Some(rv1), Some(rv2)) => {
                // Both are open - unify the row variables
                let s = self.unify_inner(&Type::Var(*rv1), &Type::Var(*rv2), span)?;
                subst = subst
                    .compose(&s)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?;
            }
            (Some(rv), None) | (None, Some(rv)) => {
                // One open, one closed - bind row variable to empty record
                // Calculate the "remaining" fields
                let remaining_fields: IndexMap<_, _> = if r1.is_some() {
                    // f2 has fields not in f1, those become the row
                    f2.iter()
                        .filter(|(k, _)| !f1.contains_key(*k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                } else {
                    // f1 has fields not in f2, those become the row
                    f1.iter()
                        .filter(|(k, _)| !f2.contains_key(*k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                };

                // Bind the row variable to the remaining fields as a record
                let remaining_type = if remaining_fields.is_empty() {
                    Type::Record(IndexMap::new()) // Empty row
                } else {
                    Type::Record(remaining_fields)
                };
                let s = self.bind_var(*rv, &remaining_type, span)?;
                subst = subst
                    .compose(&s)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?;
            }
            (None, None) => {
                // Both closed - must have exactly same fields (already checked)
            }
        }

        Ok(subst)
    }
    pub(super) fn unify_open_closed_records(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((
            ExtensibleRecord {
                fields: ext_fields,
                row_var,
            },
            Record(rec_fields),
        )
        | (
            Record(rec_fields),
            ExtensibleRecord {
                fields: ext_fields,
                row_var,
            },
        )) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let mut subst = Substitution::new();

        // All extensible record fields must be present in closed record
        for (name, ty1) in ext_fields {
            match rec_fields.get(name) {
                Some(ty2) => {
                    let s = self.unify_inner(
                        &ty1.try_apply_subst(&subst)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?,
                        &ty2.try_apply_subst(&subst)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?,
                        span,
                    )?;
                    subst = subst
                        .compose(&s)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?;
                }
                None => {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
            }
        }

        // Bind row variable to remaining fields from closed record
        if let Some(rv) = row_var {
            let remaining_fields: IndexMap<_, _> = rec_fields
                .iter()
                .filter(|(k, _)| !ext_fields.contains_key(*k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            let remaining_type = Type::Record(remaining_fields);
            let s = self.bind_var(*rv, &remaining_type, span)?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        } else {
            // Extensible record is closed (no row var) - must have exact same fields
            if ext_fields.len() != rec_fields.len() {
                return Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                });
            }
        }

        Ok(subst)
    }
    pub(super) fn unify_variants(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Variant(v1), Variant(v2)) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        let v1_is_subset = v1.keys().all(|key| v2.contains_key(key));
        let v2_is_subset = v2.keys().all(|key| v1.contains_key(key));
        if !v1_is_subset && !v2_is_subset {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        let (smaller, larger) = if v1.len() <= v2.len() {
            (v1, v2)
        } else {
            (v2, v1)
        };
        let mut names: List<_> = smaller.keys().collect();
        names.sort();
        let mut substitution = Substitution::new();
        for name in names {
            let apply = |ty: &Type| {
                ty.try_apply_subst(&substitution)
                    .map_err(|message| TypeError::UniverseBinding { message, span })
            };
            let left = apply(&smaller[name])?;
            let right = apply(&larger[name])?;
            // A variable elsewhere in a payload cannot discharge a failed
            // obligation on its concrete type or value-index components.
            let next = self.unify_inner(&left, &right, span)?;
            substitution = substitution
                .compose(&next)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }
        Ok(substitution)
    }
    pub(super) fn unify_dynamic_protocols(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            DynProtocol {
                bounds: b1,
                bindings: bi1,
            },
            DynProtocol {
                bounds: b2,
                bindings: bi2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Bounds must match (same protocols)
        if b1.len() != b2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        // Check each bound matches (order-sensitive)
        for (bound1, bound2) in b1.iter().zip(b2.iter()) {
            if bound1 != bound2 {
                return Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                });
            }
        }

        // Unify associated type bindings
        if bi1.len() != bi2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for (name, ty1) in bi1.iter() {
            match bi2.get(name) {
                Some(ty2) => {
                    let s = self.unify_inner(
                        &ty1.try_apply_subst(&subst)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?,
                        &ty2.try_apply_subst(&subst)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?,
                        span,
                    )?;
                    subst = subst
                        .compose(&s)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?;
                }
                None => {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }
            }
        }

        Ok(subst)
    }
    pub(super) fn unify_references(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Reference {
                mutable: m1,
                inner: i1,
            },
            Reference {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Allow &mut T → &T (forgetting write capability is safe)
        // Error only if expected is mutable but actual is immutable
        if *m2 && !*m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_checked_references(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            CheckedReference {
                mutable: m1,
                inner: i1,
            },
            CheckedReference {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Allow &checked mut T → &checked T (forgetting write capability is safe)
        // Error only if expected is mutable but actual is immutable
        if *m2 && !*m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_unsafe_references(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            UnsafeReference {
                mutable: m1,
                inner: i1,
            },
            UnsafeReference {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Allow &unsafe mut T → &unsafe T (mutable to immutable is OK, like in C)
        // Error only if expected is mutable but actual is immutable
        if *m2 && !*m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_unsafe_checked_reference(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            UnsafeReference {
                mutable: m1,
                inner: i1,
            },
            CheckedReference {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if *m2 && !*m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        if !*m2 && *m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_checked_reference(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            CheckedReference {
                mutable: m1,
                inner: i1,
            },
            Reference {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if m1 != m2 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_reference_unsafe(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Reference {
                mutable: m1,
                inner: i1,
            },
            UnsafeReference {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Allow mutable ref to either mutable or immutable unsafe ref
        // (can always use a mutable pointer where const is expected)
        // But not immutable ref to mutable unsafe (need mut for writing)
        if *m2 && !*m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_pointer_reference(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((
            Pointer {
                mutable: m1,
                inner: i1,
            },
            Reference {
                mutable: m2,
                inner: i2,
            },
        )
        | (
            Reference {
                mutable: m1,
                inner: i1,
            },
            Pointer {
                mutable: m2,
                inner: i2,
            },
        )) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if *m2 && !*m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_pointer_checked_reference(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((
            Pointer {
                mutable: m1,
                inner: i1,
            },
            CheckedReference {
                mutable: m2,
                inner: i2,
            },
        )
        | (
            CheckedReference {
                mutable: m1,
                inner: i1,
            },
            Pointer {
                mutable: m2,
                inner: i2,
            },
        )) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if *m2 && !*m1 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_ownership(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Ownership {
                mutable: m1,
                inner: i1,
            },
            Ownership {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if m1 != m2 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_volatile_pointers(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            VolatilePointer {
                mutable: m1,
                inner: i1,
            },
            VolatilePointer {
                mutable: m2,
                inner: i2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if m1 != m2 {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_nominal_types(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Named { path: p1, args: a1 }, Named { path: p2, args: a2 }) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        // Sized integer cross-coercion: USize ↔ ISize, USize ↔ UInt64, etc.
        // Uses data-driven sized_numeric_types set instead of hardcoded name list.
        if p1.segments.len() == 1 && p2.segments.len() == 1 && a1.is_empty() && a2.is_empty() {
            fn extract_name(path: &verum_ast::ty::Path) -> &str {
                path.segments
                    .last()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "",
                    })
                    .unwrap_or("")
            }
            let n1 = extract_name(p1);
            let n2 = extract_name(p2);
            if n1 != n2 && self.is_sized_numeric(n1) && self.is_sized_numeric(n2) {
                return Ok(Substitution::new());
            }
        }

        let same_head = crate::ty::same_type_path(p1, p2);
        if !same_head && (p1.segments.len() != p2.segments.len() || a1.len() != a2.len()) {
            let tn1 = p1.last_segment_name();
            let tn2 = p2.last_segment_name();
            if self.is_tensor_family(tn1) && self.is_tensor_family(tn2) {
                return Ok(Substitution::new());
            }
            // Byte / UInt8 ↔ BytewiseFfi-marked struct (sockaddr
            // family + any future packed-C-struct byte mirror).
            // Replaces the previously hardcoded
            // `matches!(name, "Sockaddr" | "SocketAddr" |
            // "SockaddrIn")` match — registry-driven via
            // `is_bytewise_ffi`, populated from stdlib
            // `implement BytewiseFfi for X {}` blocks.
            if (tn1 == "Byte" || tn1 == "UInt8") && self.is_bytewise_ffi(tn2) {
                return Ok(Substitution::new());
            }
            if (tn2 == "Byte" || tn2 == "UInt8") && self.is_bytewise_ffi(tn1) {
                return Ok(Substitution::new());
            }
        }

        // Legacy descriptors can refer to an imported declaration by its
        // bare name. Only a whole unqualified path can use this bridge.
        let matches_bare = |bare: &verum_ast::ty::Path, qualified: &verum_ast::ty::Path| {
            let Some(bare) = bare.as_ident() else {
                return false;
            };
            if let Some(ident) = qualified.as_ident() {
                return ident
                    .name
                    .as_str()
                    .rsplit_once('.')
                    .is_some_and(|(_, leaf)| bare.name.as_str() == leaf);
            }
            !bare.name.as_str().contains('.')
                && bare.name.as_str() == qualified.last_segment_name()
                && crate::ty::nominal_path_name(qualified).is_some()
        };
        let bare_bridge = !same_head
            && (matches_bare(p1, p2) || matches_bare(p2, p1))
            && std::env::var_os("VERUM_NO_LEAF_BRIDGE").is_none();
        if a1.len() != a2.len() || !(same_head || bare_bridge) {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for (ty1, ty2) in a1.iter().zip(a2.iter()) {
            let s = self.unify_inner(
                &ty1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &ty2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }
        Ok(subst)
    }
    pub(super) fn unify_equal_projections(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Generic { name: n1, args: a1 }, Generic { name: n2, args: a2 }) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        // Same projection name (e.g., both are ::Item)
        // Unify the base types
        if a1.len() != a2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for (ty1, ty2) in a1.iter().zip(a2.iter()) {
            let s = self.unify_inner(
                &ty1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &ty2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }
        Ok(subst)
    }
    pub(super) fn unify_left_projection(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Generic { name, args }, other) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        let assoc = name.as_str().trim_start_matches("::").to_string();
        let resolver = self.projection_resolver.clone();
        if let Some(resolved) = resolver.and_then(|r| r(&args[0], &assoc)) {
            return self.unify_inner(&resolved, other, span);
        }
        Err(TypeError::Mismatch {
            expected: t2.to_text(),
            actual: t1.to_text(),
            span,
        })
    }
    pub(super) fn unify_right_projection(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (other, Generic { name, args }) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        let assoc = name.as_str().trim_start_matches("::").to_string();
        let resolver = self.projection_resolver.clone();
        if let Some(resolved) = resolver.and_then(|r| r(&args[0], &assoc)) {
            return self.unify_inner(other, &resolved, span);
        }
        Err(TypeError::Mismatch {
            expected: t2.to_text(),
            actual: t1.to_text(),
            span,
        })
    }
    pub(super) fn unify_generic_types(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Generic { name: n1, args: a1 }, Generic { name: n2, args: a2 }) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        if n1 != n2 || a1.len() != a2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for (ty1, ty2) in a1.iter().zip(a2.iter()) {
            let s = self.unify_inner(
                &ty1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &ty2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }
        Ok(subst)
    }
    pub(super) fn unify_generic_nominal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Generic { name, args: a1 }, Named { path, args: a2 })
        | (Named { path, args: a2 }, Generic { name, args: a1 })) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Source annotations can carry a qualified Generic head while
        // method metadata carries a Named application. Preserve the
        // entire path (including opaque dotted identifiers); a bare
        // spelling is equivalent only when declaration metadata says so.
        let path_name = crate::ty::nominal_path_name(path);
        let same_head = path_name.as_ref().is_some_and(|path_name| {
            name.as_str() == path_name.as_ref()
                || self
                    .nominal_identity_resolver
                    .as_ref()
                    .is_some_and(|resolve| resolve(name.as_str(), path_name.as_ref()))
        });

        if !same_head || a1.len() != a2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for (ty1, ty2) in a1.iter().zip(a2.iter()) {
            let s = self.unify_inner(
                &ty1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &ty2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }
        Ok(subst)
    }
    pub(super) fn unify_application_variant(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((
            TypeApp {
                constructor,
                args: app_args,
            },
            Variant(variants),
        )
        | (
            Variant(variants),
            TypeApp {
                constructor,
                args: app_args,
            },
        )) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Generate signature from variant structure
        let signature = Self::variant_type_signature(variants);

        // Check if constructor is a type variable (HKT inference case)
        if let Type::Var(var) = constructor.as_ref() {
            // Look up the type constructor name from registry
            if let Some(type_ctor_name) = self.variant_type_names.get(&signature) {
                // Calculate arity from non-Unit payloads
                let type_ctor_arity = variants
                    .values()
                    .filter(|payload| **payload != Type::Unit)
                    .count();

                // Check arity matches
                if app_args.len() != type_ctor_arity {
                    return Err(TypeError::Mismatch {
                        expected: t2.to_text(),
                        actual: t1.to_text(),
                        span,
                    });
                }

                // Bind the type variable to the type constructor
                let type_ctor = Type::TypeConstructor {
                    name: type_ctor_name.clone(),
                    arity: type_ctor_arity,
                    kind: Self::kind_for_arity(type_ctor_arity),
                };
                let mut subst = self.bind_var(*var, &type_ctor, span)?;

                // Collect type variables from the variant payloads
                let mut variant_type_vars: Vec<TypeVar> = Vec::new();
                for (_, payload_ty) in variants.iter() {
                    Self::collect_type_vars_from_type(payload_ty, &mut variant_type_vars);
                }
                variant_type_vars.sort();
                variant_type_vars.dedup();

                // Unify the type arguments with the variant's type parameters
                for (idx, app_arg) in app_args.iter().enumerate() {
                    if let Some(tv) = variant_type_vars.get(idx) {
                        let s = self.unify_inner(
                            &app_arg
                                .try_apply_subst(&subst)
                                .map_err(|message| TypeError::UniverseBinding { message, span })?,
                            &Type::Var(*tv)
                                .try_apply_subst(&subst)
                                .map_err(|message| TypeError::UniverseBinding { message, span })?,
                            span,
                        )?;
                        subst = subst
                            .compose(&s)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?;
                    }
                }

                return Ok(subst);
            }

            // Unknown variant type - can't infer HKT
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        } else {
            // Constructor is not a variable - check if it matches the variant type
            if let Type::TypeConstructor { name, .. } = constructor.as_ref() {
                // Look up from registry and check name match
                if let Some(registered_name) = self.variant_type_names.get(&signature) {
                    if registered_name.as_str() == name.as_str() {
                        // Collect and unify type arguments
                        let mut variant_type_vars: Vec<TypeVar> = Vec::new();
                        for (_, payload_ty) in variants.iter() {
                            Self::collect_type_vars_from_type(payload_ty, &mut variant_type_vars);
                        }
                        variant_type_vars.sort();
                        variant_type_vars.dedup();

                        let mut subst = Substitution::new();
                        for (idx, app_arg) in app_args.iter().enumerate() {
                            if let Some(tv) = variant_type_vars.get(idx) {
                                let s = self.unify_inner(
                                    &app_arg.try_apply_subst(&subst).map_err(|message| {
                                        TypeError::UniverseBinding { message, span }
                                    })?,
                                    &Type::Var(*tv).try_apply_subst(&subst).map_err(|message| {
                                        TypeError::UniverseBinding { message, span }
                                    })?,
                                    span,
                                )?;
                                subst = subst.compose(&s).map_err(|message| {
                                    TypeError::UniverseBinding { message, span }
                                })?;
                            }
                        }
                        return Ok(subst);
                    }
                }
            }
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_meta_values(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Meta {
                name: n1,
                ty: t1,
                refinement: r1,
                value: v1,
            },
            Meta {
                name: n2,
                ty: t2,
                refinement: r2,
                value: v2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        use crate::ty::ValueTerm;
        if v1 == v2 {
            return self.unify_inner(t1, t2, span);
        }
        let (variable, target) = match (v1, v2) {
            (ValueTerm::Inference(id), _) => (
                *id,
                Type::Meta {
                    name: n2.clone(),
                    ty: t2.clone(),
                    refinement: r2.clone(),
                    value: v2.clone(),
                },
            ),
            (_, ValueTerm::Inference(id)) => (
                *id,
                Type::Meta {
                    name: n1.clone(),
                    ty: t1.clone(),
                    refinement: r1.clone(),
                    value: v1.clone(),
                },
            ),
            _ => {
                return Err(TypeError::Mismatch {
                    expected: format!("exact value index `{n2}`").into(),
                    actual: format!("exact value index `{n1}`").into(),
                    span,
                });
            }
        };
        if target.value_ids().contains(&variable) {
            return Err(TypeError::OtherWithCodeSpanned {
                code: "E405".into(),
                msg: "cyclic compile-time value substitution".into(),
                span,
            });
        }
        let mut substitution = self.unify_inner(t1, t2, span)?;
        let target = target
            .try_apply_subst(&substitution)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;
        substitution.value_bindings.insert(variable, target);
        Ok(substitution)
    }
    pub(super) fn unify_dependent_functions(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Pi {
                param_name: n1,
                param_type: p1,
                return_type: r1,
            },
            Pi {
                param_name: n2,
                param_type: p2,
                return_type: r2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Unify parameter types
        let subst = self.unify_inner(p1, p2, span)?;

        // For dependent return types, we need alpha-equivalence
        // Substitute the parameter names to a common fresh variable
        let r1_applied = r1
            .try_apply_subst(&subst)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;
        let r2_applied = r2
            .try_apply_subst(&subst)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;

        // If parameter names differ, rename one to match the other
        let r2_renamed = if n1 != n2 {
            Self::rename_bound_var(&r2_applied, n2, n1)
        } else {
            r2_applied
        };

        let s2 = self.unify_inner(&r1_applied, &r2_renamed, span)?;
        Ok(subst
            .compose(&s2)
            .map_err(|message| TypeError::UniverseBinding { message, span })?)
    }
    pub(super) fn unify_dependent_pairs(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Sigma {
                fst_name: n1,
                fst_type: f1,
                snd_type: s1,
            },
            Sigma {
                fst_name: n2,
                fst_type: f2,
                snd_type: s2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Unify first component types
        let subst = self.unify_inner(f1, f2, span)?;

        // For dependent second component, handle alpha-equivalence
        let s1_applied = s1
            .try_apply_subst(&subst)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;
        let s2_applied = s2
            .try_apply_subst(&subst)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;

        let s2_renamed = if n1 != n2 {
            Self::rename_bound_var(&s2_applied, n2, n1)
        } else {
            s2_applied
        };

        let sub2 = self.unify_inner(&s1_applied, &s2_renamed, span)?;
        Ok(subst
            .compose(&sub2)
            .map_err(|message| TypeError::UniverseBinding { message, span })?)
    }
    pub(super) fn unify_equalities(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Eq {
                ty: t1,
                lhs: l1,
                rhs: r1,
            },
            Eq {
                ty: t2,
                lhs: l2,
                rhs: r2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Unify the carrier types
        let subst = self.unify_inner(t1, t2, span)?;

        // Check definitional equality of terms. Fast path is
        // syntactic equality on EqTerm; fallback routes through
        // the cubical normalizer so identities like
        // `transport Refl x ≡ x` and `sym(refl(x)) ≡ refl(x)`
        // are accepted.
        // Fast path: syntactic equality. Cubical-normalized
        // equality is only consulted when the feature is on
        // (cost: a whnf reduction per comparison).
        let l_eq = Self::eq_terms_equal(l1, l2)
            || (self.cubical_enabled
                && crate::cubical_bridge::definitionally_equal_cubical(l1, l2));
        if !l_eq {
            return Err(TypeError::Mismatch {
                expected: "Eq type with matching left-hand side".into(),
                actual: "Eq type with different left-hand side".into(),
                span,
            });
        }

        let r_eq = Self::eq_terms_equal(r1, r2)
            || (self.cubical_enabled
                && crate::cubical_bridge::definitionally_equal_cubical(r1, r2));
        if !r_eq {
            return Err(TypeError::Mismatch {
                expected: "Eq type with matching right-hand side".into(),
                actual: "Eq type with different right-hand side".into(),
                span,
            });
        }

        Ok(subst)
    }
    pub(super) fn unify_paths(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        let (
            Type::PathType {
                space: s1,
                left: l1,
                right: r1,
            },
            Type::PathType {
                space: s2,
                left: l2,
                right: r2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let subst = self.unify_inner(s1, s2, span)?;

        // When cubical is disabled, fall back to strict syntactic
        // equality on endpoints (skip normalization).
        let left_eq = if self.cubical_enabled {
            l1.definitionally_equal(l2)
        } else {
            l1 == l2
        };
        if !left_eq {
            return Err(TypeError::Mismatch {
                expected: "PathType with matching left endpoint".into(),
                actual: "PathType with different left endpoint".into(),
                span,
            });
        }
        let right_eq = if self.cubical_enabled {
            r1.definitionally_equal(r2)
        } else {
            r1 == r2
        };
        if !right_eq {
            return Err(TypeError::Mismatch {
                expected: "PathType with matching right endpoint".into(),
                actual: "PathType with different right endpoint".into(),
                span,
            });
        }
        Ok(subst)
    }
    pub(super) fn unify_partial_elements(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Type::Partial {
                element_type: e1,
                face: f1,
            },
            Type::Partial {
                element_type: e2,
                face: f2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let subst = self.unify_inner(e1, e2, span)?;
        let face_eq = if self.cubical_enabled {
            f1.definitionally_equal(f2)
        } else {
            f1 == f2
        };
        if !face_eq {
            return Err(TypeError::Mismatch {
                expected: "Partial type with matching face".into(),
                actual: "Partial type with divergent face".into(),
                span,
            });
        }
        Ok(subst)
    }
    pub(super) fn unify_inductive_types(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Inductive {
                name: n1,
                params: p1,
                indices: i1,
                universe: u1,
                constructors: c1,
            },
            Inductive {
                name: n2,
                params: p2,
                indices: i2,
                universe: u2,
                constructors: c2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Names must match
        if n1 != n2 {
            return Err(TypeError::Mismatch {
                expected: format!("Inductive type '{}'", n2).into(),
                actual: format!("Inductive type '{}'", n1).into(),
                span,
            });
        }

        // Universe levels must be compatible
        if !Self::universe_levels_unify(u1, u2) {
            return Err(TypeError::Mismatch {
                expected: format!("Universe level {}", u2).into(),
                actual: format!("Universe level {}", u1).into(),
                span,
            });
        }

        // Constructor count must match
        if c1.len() != c2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} constructors", c2.len()).into(),
                actual: format!("{} constructors", c1.len()).into(),
                span,
            });
        }

        // Unify parameters
        if p1.len() != p2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} type parameters", p2.len()).into(),
                actual: format!("{} type parameters", p1.len()).into(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for ((_, t1), (_, t2)) in p1.iter().zip(p2.iter()) {
            let s = self.unify_inner(
                &t1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &t2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }

        // Unify indices (for indexed families like List<T, n>)
        if i1.len() != i2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} indices", i2.len()).into(),
                actual: format!("{} indices", i1.len()).into(),
                span,
            });
        }

        for ((_, t1), (_, t2)) in i1.iter().zip(i2.iter()) {
            let s = self.unify_inner(
                &t1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &t2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }

        Ok(subst)
    }
    pub(super) fn unify_coinductive_types(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Coinductive {
                name: n1,
                params: p1,
                destructors: d1,
            },
            Coinductive {
                name: n2,
                params: p2,
                destructors: d2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Names must match
        if n1 != n2 {
            return Err(TypeError::Mismatch {
                expected: format!("Coinductive type '{}'", n2).into(),
                actual: format!("Coinductive type '{}'", n1).into(),
                span,
            });
        }

        // Destructor count must match
        if d1.len() != d2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} destructors", d2.len()).into(),
                actual: format!("{} destructors", d1.len()).into(),
                span,
            });
        }

        // Unify parameters
        if p1.len() != p2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} type parameters", p2.len()).into(),
                actual: format!("{} type parameters", p1.len()).into(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for ((_, t1), (_, t2)) in p1.iter().zip(p2.iter()) {
            let s = self.unify_inner(
                &t1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &t2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }

        Ok(subst)
    }
    pub(super) fn unify_higher_inductive_types(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            HigherInductive {
                name: n1,
                params: p1,
                point_constructors: pc1,
                path_constructors: pa1,
            },
            HigherInductive {
                name: n2,
                params: p2,
                point_constructors: pc2,
                path_constructors: pa2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Names must match
        if n1 != n2 {
            return Err(TypeError::Mismatch {
                expected: format!("HIT '{}'", n2).into(),
                actual: format!("HIT '{}'", n1).into(),
                span,
            });
        }

        // Constructor counts must match
        if pc1.len() != pc2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} point constructors", pc2.len()).into(),
                actual: format!("{} point constructors", pc1.len()).into(),
                span,
            });
        }

        if pa1.len() != pa2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} path constructors", pa2.len()).into(),
                actual: format!("{} path constructors", pa1.len()).into(),
                span,
            });
        }

        // Unify parameters
        if p1.len() != p2.len() {
            return Err(TypeError::Mismatch {
                expected: format!("{} type parameters", p2.len()).into(),
                actual: format!("{} type parameters", p1.len()).into(),
                span,
            });
        }

        let mut subst = Substitution::new();
        for ((_, t1), (_, t2)) in p1.iter().zip(p2.iter()) {
            let s = self.unify_inner(
                &t1.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &t2.try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }

        Ok(subst)
    }
    pub(super) fn unify_quantities(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Quantified {
                inner: i1,
                quantity: q1,
            },
            Quantified {
                inner: i2,
                quantity: q2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Quantities must be compatible
        if !Self::quantities_compatible(q1, q2) {
            return Err(TypeError::Mismatch {
                expected: "Compatible quantity annotation".into(),
                actual: "Incompatible quantity annotation".into(),
                span,
            });
        }

        // Unify inner types
        self.unify_inner(i1, i2, span)
    }
    pub(super) fn unify_quantified_carrier(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Quantified { inner, quantity }, ty) | (ty, Quantified { inner, quantity })) =
            (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if *quantity == Quantity::Omega {
            // Unrestricted quantity can be treated as regular type
            self.unify_inner(inner, ty, span)
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_existentials(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (Exists { var: v1, body: b1 }, Exists { var: v2, body: b2 }) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        // Rename v2 to v1 in b2 for alpha equivalence
        let mut rename_subst = Substitution::new();
        rename_subst.insert(*v2, Type::Var(*v1));
        let b2_renamed = b2
            .try_apply_subst(&rename_subst)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;

        // Unify the bodies
        self.unify_inner(b1, &b2_renamed, span)
    }
    pub(super) fn unify_existential_carrier(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Exists { var, body }, ty) | (ty, Exists { var, body })) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        // The concrete type becomes the witness for the existential
        let mut subst = Substitution::new();
        subst.insert(*var, ty.clone());
        let body_subst = body
            .try_apply_subst(&subst)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;

        // Unify the instantiated body with the concrete type
        self.unify_inner(&body_subst, ty, span)
    }
    pub(super) fn unify_universal_types(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Forall {
                vars: vs1,
                body: b1,
            },
            Forall {
                vars: vs2,
                body: b2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Must have same arity
        if vs1.len() != vs2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        // Rename vs2 to vs1 in b2 for alpha equivalence
        let mut rename_subst = Substitution::new();
        for (v1, v2) in vs1.iter().zip(vs2.iter()) {
            rename_subst.insert(*v2, Type::Var(*v1));
        }
        let b2_renamed = b2
            .try_apply_subst(&rename_subst)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;

        // Unify the bodies
        self.unify_inner(b1, &b2_renamed, span)
    }
    pub(super) fn unify_right_universal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (ty, Forall { vars, body }) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        let mut skolemize = Substitution::new();
        for v in vars.iter() {
            let skolem_name = crate::ty::skolem::name(TypeVar::fresh().id());
            skolemize.insert(
                *v,
                Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(skolem_name, span)),
                    args: verum_common::List::new(),
                },
            );
        }
        let body_sk = body
            .try_apply_subst(&skolemize)
            .map_err(|message| TypeError::UniverseBinding { message, span })?;
        self.unify_inner(ty, &body_sk, span)
    }
    pub(super) fn unify_applications(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Type::TypeApp {
                constructor: c1,
                args: a1,
            },
            Type::TypeApp {
                constructor: c2,
                args: a2,
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // First unify the constructors
        let mut subst = self.unify_inner(c1, c2, span)?;

        // Args must have the same length
        if a1.len() != a2.len() {
            return Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            });
        }

        // Unify each argument
        for (arg1, arg2) in a1.iter().zip(a2.iter()) {
            let s = self.unify_inner(
                &arg1
                    .try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                &arg2
                    .try_apply_subst(&subst)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?,
                span,
            )?;
            subst = subst
                .compose(&s)
                .map_err(|message| TypeError::UniverseBinding { message, span })?;
        }

        Ok(subst)
    }
    pub(super) fn unify_constructors(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let (
            Type::TypeConstructor {
                name: n1,
                arity: a1,
                ..
            },
            Type::TypeConstructor {
                name: n2,
                arity: a2,
                ..
            },
        ) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        if n1 == n2 && a1 == a2 {
            Ok(Substitution::new())
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_application_generic(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((
            Type::TypeApp {
                constructor,
                args: app_args,
            },
            Type::Generic {
                name,
                args: gen_args,
            },
        )
        | (
            Type::Generic {
                name,
                args: gen_args,
            },
            Type::TypeApp {
                constructor,
                args: app_args,
            },
        )) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Check if constructor is a type variable (HKT inference case)
        if let Type::Var(var) = constructor.as_ref() {
            if app_args.len() != gen_args.len() {
                return Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                });
            }

            // Bind the type variable to a TypeConstructor
            let arity = gen_args.len();
            let kind = Self::kind_for_arity(arity);
            let type_ctor = Type::TypeConstructor {
                name: name.clone(),
                arity,
                kind,
            };
            let mut subst = self.bind_var(*var, &type_ctor, span)?;

            // Unify the arguments
            for (app_arg, gen_arg) in app_args.iter().zip(gen_args.iter()) {
                let s = self.unify_inner(
                    &app_arg
                        .try_apply_subst(&subst)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?,
                    &gen_arg
                        .try_apply_subst(&subst)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?,
                    span,
                )?;
                subst = subst
                    .compose(&s)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?;
            }

            Ok(subst)
        } else {
            // Constructor is not a variable - check if it matches the Generic name
            if let Type::TypeConstructor {
                name: ctor_name, ..
            } = constructor.as_ref()
            {
                if ctor_name == name && app_args.len() == gen_args.len() {
                    let mut subst = Substitution::new();
                    for (app_arg, gen_arg) in app_args.iter().zip(gen_args.iter()) {
                        let s = self.unify_inner(
                            &app_arg
                                .try_apply_subst(&subst)
                                .map_err(|message| TypeError::UniverseBinding { message, span })?,
                            &gen_arg
                                .try_apply_subst(&subst)
                                .map_err(|message| TypeError::UniverseBinding { message, span })?,
                            span,
                        )?;
                        subst = subst
                            .compose(&s)
                            .map_err(|message| TypeError::UniverseBinding { message, span })?;
                    }
                    return Ok(subst);
                }
            }
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_application_nominal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((
            Type::TypeApp {
                constructor,
                args: app_args,
            },
            Type::Named {
                path,
                args: named_args,
            },
        )
        | (
            Type::Named {
                path,
                args: named_args,
            },
            Type::TypeApp {
                constructor,
                args: app_args,
            },
        )) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        // Check if constructor is a type variable (HKT inference case)
        if let Type::Var(var) = constructor.as_ref() {
            if app_args.len() != named_args.len() {
                return Err(TypeError::Mismatch {
                    expected: t2.to_text(),
                    actual: t1.to_text(),
                    span,
                });
            }

            // Extract name from path
            // Note: use verum_common::Text explicitly to avoid conflict with Type::Text
            let name = path
                .segments
                .last()
                .map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => {
                        let s: verum_common::Text = ident.name.clone();
                        s
                    }
                    _ => verum_common::Text::from(""),
                })
                .unwrap_or_else(|| verum_common::Text::from(""));

            // Bind the type variable to a TypeConstructor
            let arity = named_args.len();
            let kind = Self::kind_for_arity(arity);
            let type_ctor = Type::TypeConstructor { name, arity, kind };
            let mut subst = self.bind_var(*var, &type_ctor, span)?;

            // Unify the arguments
            for (app_arg, named_arg) in app_args.iter().zip(named_args.iter()) {
                let s = self.unify_inner(
                    &app_arg
                        .try_apply_subst(&subst)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?,
                    &named_arg
                        .try_apply_subst(&subst)
                        .map_err(|message| TypeError::UniverseBinding { message, span })?,
                    span,
                )?;
                subst = subst
                    .compose(&s)
                    .map_err(|message| TypeError::UniverseBinding { message, span })?;
            }

            Ok(subst)
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_placeholders(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Placeholder { name, .. }, other) | (other, Placeholder { name, .. })) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let compatible = match other {
            // The forward declaration and nominal carry the same
            // exact owner key; a sibling's identical leaf is not proof.
            Named { path, .. } => crate::ty::nominal_path_name(path)
                .is_some_and(|declared| declared.as_ref() == name.as_str()),
            Generic { name: n, .. } => n == name,
            // undetermined sides defer to later resolution
            Var(_) | Unknown => true,
            // two placeholders unify only for the same forward-declared name
            Placeholder { name: n2, .. } => n2 == name,
            // THE NAME'S OWN DEFINITION, ALREADY EXPANDED.
            //
            // A named type reaches this arm as a `Named`/`Generic`
            // and matches by name. The SAME type, expanded to its
            // variant body by any of the paths that do that, has no
            // name left to compare and was refused:
            //
            //     expected 'Simple(Unit) | Reserved(Unit) | …',
            //     found '<placeholder:ExprOp>'
            //
            // — a forward-declared type refusing to unify with
            // itself. The registry the unifier already carries
            // answers the question exactly: look the name up and
            // require the SAME set of constructors. That is a
            // name-checked identity, not the blanket
            // unify-with-everything that made leaked placeholders
            // unsound (tech-debt register A27, pool T0119).
            Variant(other_cases) => {
                // Two registries answer "which type is this
                // variant body?", and both are keyed by the
                // name, so neither is a blanket. The relaxed
                // signature is what the Generic↔Variant arms
                // already use, and it is populated for source
                // declarations as well as stdlib ones.
                let relaxed = Self::variant_type_signature_relaxed(other_cases);
                self.variant_type_names
                    .get(&relaxed)
                    .is_some_and(|registered| registered == name)
                    || self.original_variant_types.get(name).is_some_and(
                        |declared| match declared {
                            Variant(declared_cases) => {
                                declared_cases.len() == other_cases.len()
                                    && declared_cases.keys().all(|k| other_cases.contains_key(k))
                            }
                            _ => false,
                        },
                    )
            }
            _ => false,
        };
        if compatible {
            Ok(Substitution::new())
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_float_nominal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Float, Named { path, args }) | (Named { path, args }, Float)) = (t1, t2) else {
            unreachable!("unification rule selected for different constructors")
        };

        let pname = path
            .segments
            .last()
            .map(|s| match s {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                _ => "",
            })
            .unwrap_or("");
        // `Float` and `Float64` NAME THE SAME TYPE.  The
        // authority says so in as many words —
        // `canonical_identity_name` folds
        // `Float | Float64 | f64 | F64` to one identity and keeps
        // `Float32` separate, on the same rule that makes `Int` and
        // `Int64` one type while `Int8` stays distinct.  The INTEGER
        // arm of this match consults the coercion registry; this arm
        // consulted only `is_tensor_family`, so every mixed
        // `Float`/`Float64` spelling in the library was a hard
        // mismatch in BOTH directions (11 core/ files, measured
        // 2026-08-16) while `Int`/`Int64` unified silently.
        //
        // Identity, not coercion: asking the canonical name keeps
        // `Float32` a genuine mismatch, which `is_sized_numeric`
        // would have merged.
        let float_identity =
            verum_common::well_known_types::type_names::canonical_identity_name("Float");
        let named_identity =
            verum_common::well_known_types::type_names::canonical_identity_name(pname);
        if named_identity == float_identity || self.is_tensor_family(pname) {
            Ok(Substitution::new())
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_slice_nominal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Slice { element }, Named { path, args, .. })
        | (Named { path, args, .. }, Slice { element })) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let pname = path
            .segments
            .last()
            .map(|s| match s {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                _ => "",
            })
            .unwrap_or("");
        if matches!(pname, "UInt8" | "Byte" | "U8" | "u8") {
            // [Byte] vs UInt8 - allow coercion
            Ok(Substitution::new())
        } else {
            // Try to unify the element type with the named type
            self.unify_inner(
                element,
                &Named {
                    path: path.clone(),
                    args: args.clone(),
                },
                span,
            )
            .map_err(|_| TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_future_nominal(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Future { output }, Named { path, args }) | (Named { path, args }, Future { output })) =
            (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let path_name = path
            .segments
            .last()
            .map(|s| match s {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                _ => "",
            })
            .unwrap_or("");
        if path_name == "Future" {
            self.unify_inner(output, &args[0], span)
        } else {
            Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            })
        }
    }
    pub(super) fn unify_variant_constructor(
        &mut self,
        t1: &Type,
        t2: &Type,
        span: Span,
    ) -> Result<Substitution> {
        let ((Variant(variants), TypeConstructor { name, arity, .. })
        | (TypeConstructor { name, arity, .. }, Variant(variants))) = (t1, t2)
        else {
            unreachable!("unification rule selected for different constructors")
        };

        let signature = Self::variant_type_signature(variants);
        if let Some(registered_name) = self.variant_type_names.get(&signature) {
            // Concrete-name match — the variant nominally is
            // this constructor, so the kinded form unifies
            // trivially. (Argument-level unification has
            // already happened upstream when the wrapping
            // TypeApps unified pairwise.)
            if registered_name.as_str() == name.as_str() {
                return Ok(Substitution::new());
            }
        }
        // HKT-parameter shape: single-letter or short
        // uppercase identifier with arity ≥ 1 (e.g. `F`, `M`,
        // `Self`). Accept structurally — the outer dispatch
        // has already pinned which concrete constructor this
        // parameter resolves to.
        if Self::looks_like_hkt_param(name.as_str()) {
            return Ok(Substitution::new());
        }
        Err(TypeError::Mismatch {
            expected: t2.to_text(),
            actual: t1.to_text(),
            span,
        })
    }
}
