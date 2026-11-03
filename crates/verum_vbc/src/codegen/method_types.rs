//! One bottom-up analysis of a method receiver chain.
//!
//! Structural callable results, displayed names and declaration authority have
//! different precedence. Carry those facts together instead of recursively
//! asking each recovery path to traverse the same receiver again. Facts belong
//! only to this immutable query; no AST-address cache survives a scope change.
use super::*;
use crate::types::TypeRef;
use verum_common::{List, Maybe, Set, Text};

pub(super) struct ExpressionTypes {
    /// The declared structural result, including callable parameter witnesses.
    pub(super) callable: Maybe<TypeRef>,
    /// Value-expression recovery, with namespace calls retaining precedence.
    pub(super) name: Maybe<Text>,
    /// Method-chain recovery also admits bare zero-argument declaration names.
    pub(super) chain: Maybe<Text>,
    /// A nominal type operand takes precedence over value/variant recovery.
    pub(super) declared: Maybe<Text>,
}

impl VbcCodegen {
    /// Expand the selected alias declaration structurally, retaining type and
    /// const arguments. Unknown owners remain unknown; cyclic aliases refuse.
    pub(in crate::codegen) fn method_receiver_declaration_name(&self, name: &str) -> Maybe<Text> {
        let name = Self::method_receiver_type_name(name);
        let base = Self::strip_generic_args(name);
        let declaration = self.nominal_type_id(base).and_then(|id| self.type_by_id(id));
        let Some(alias) = declaration.filter(|ty| ty.kind == crate::types::TypeKind::Alias)
        else {
            // A mount rename is a source binding, not a TypeKind::Alias.
            // Its selected owner still governs method lookup; retain explicit
            // type/const arguments while replacing only the nominal head.
            if !self.local_concrete_types.contains(base)
                && let Some(owner) = self.mounted_type_path(base)
            {
                // The public mount may be a facade re-export. Its selected
                // declaration owns the layout and methods, not the export path.
                let owner = declaration.and_then(|ty| self.type_declaration_identity(ty))
                    .unwrap_or_else(|| owner.into());
                return Some(format!("{owner}{}", &name[base.len()..]).into());
            }
            return Some(name.into());
        };
        let ast = verum_fast_parser::Parser::new(name).parse_type().ok()?;
        // The scoped nominal lookup above selects the alias. Do not re-enter
        // the ambient bare-alias cache while recovering its instantiation.
        let witness = match &ast.kind {
            verum_ast::ty::TypeKind::Generic { .. } => {
                let TypeRef::Instantiated { args, .. } = self.explicit_type_witness(&ast, None)?
                else {
                    return None;
                };
                TypeRef::Instantiated {
                    base: alias.id,
                    args,
                }
            }
            _ => TypeRef::Concrete(alias.id),
        };
        self.expand_declared_alias(witness).and_then(|ty| self.callable_result_name(&ty))
    }

    /// Expand only the selected nominal alias declaration, preserving caller
    /// binders and complete arguments without reparsing a displayed type name.
    pub(in crate::codegen) fn expand_declared_alias(&self, mut witness: TypeRef) -> Maybe<TypeRef> {
        let mut seen = Set::new();
        let mut references = smallvec::SmallVec::<[(crate::types::CbgrTier, crate::types::Mutability); 2]>::new();
        loop {
            if let TypeRef::Reference { inner, tier, mutability } = witness {
                references.push((tier, mutability));
                witness = *inner;
                continue;
            }
            let (base, args) = match &witness {
                TypeRef::Concrete(id) => (*id, &[][..]),
                TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                _ => break,
            };
            let Some(descriptor) = self.type_by_id(base)
                .filter(|ty| ty.kind == crate::types::TypeKind::Alias)
            else {
                break;
            };
            if !seen.insert(base) || args.len() != descriptor.type_params.len() {
                return None;
            }
            witness = crate::mono::TypeSubstitution::new(&descriptor.type_params, args)
                .apply(descriptor.alias_target.as_ref()?).ok()?;
        }
        for (tier, mutability) in references.into_iter().rev() {
            witness = TypeRef::Reference { inner: Box::new(witness), tier, mutability };
        }
        Some(witness)
    }

    fn callable_result_name(&self, ty: &TypeRef) -> Maybe<Text> {
        self.scoped_callable_type_name(ty, Some(&self.ctx.current_generic_param_ids))
            .or_else(|| self.type_ref_to_field_name(ty).map(Into::into))
    }

    pub(super) fn method_expression_types(&self, expr: &Expr) -> ExpressionTypes {
        let mut receivers = List::new();
        let mut root = expr;
        loop {
            let next = match &root.kind {
                ExprKind::MethodCall { receiver, .. } => receiver.as_ref(),
                ExprKind::Paren(inner) => inner.as_ref(),
                _ => break,
            };
            receivers.push(root);
            root = next;
        }
        let name: Maybe<Text> = self.extract_expr_type_name(root).map(Into::into);
        let chain = match &root.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => match &path.segments[0] {
                PathSegment::Name(ident) => {
                    let name = ident.name.as_str();
                    self.ctx
                        .variable_type_names
                        .get(name)
                        .cloned()
                        .or_else(|| {
                            self.ctx
                                .lookup_function_in_scope(name)
                                .filter(|info| info.param_count == 0)
                                .and_then(|info| info.return_type_name.clone())
                        })
                        .or_else(|| {
                            self.type_name_to_id
                                .contains_key(name)
                                .then(|| name.to_owned())
                        })
                        .map(Into::into)
                }
                _ => name.clone(),
            },
            _ => name.clone(),
        };
        // A projected place carries the owner's substituted field declaration.
        // A displayed nominal head cannot recover its nested type arguments.
        let projection = matches!(root.kind, ExprKind::Field { .. })
            .then(|| self.declared_expression_result(root, None, 0, 0))
            .flatten();
        let declared = self
            .declared_nominal_receiver_name(root)
            .or_else(|| projection.as_ref().and_then(|ty| self.callable_result_name(ty)))
            .or_else(|| name.clone());
        let mut types = ExpressionTypes {
            callable: projection,
            name,
            chain,
            declared,
        };
        for expr in receivers.into_iter().rev() {
            if matches!(expr.kind, ExprKind::Paren(_)) {
                // Parentheses preserve the value, while the chain query's
                // zero-argument path fallback applies only to a bare path.
                types.chain = match &types.callable {
                    Some(ty) => self.callable_result_name(ty),
                    None => types.name.clone(),
                };
                continue;
            }
            let ExprKind::MethodCall {
                receiver,
                method,
                type_args,
                args,
            } = &expr.kind
            else {
                unreachable!("collected receiver edge");
            };
            // Namespace resolution selects one declaration for every query
            // view. A missing explicit target cannot borrow method evidence
            // from an unrelated value with the same spelling.
            if let Some(result) = self.scoped_call_result_type_name(expr) {
                let name: Maybe<Text> = result.map(Into::into);
                types = ExpressionTypes {
                    callable: None,
                    name: name.clone(),
                    chain: name.clone(),
                    declared: name,
                };
                continue;
            }
            if let Some(result) = types.declared.as_deref().and_then(|owner|
                self.dynamic_call_result(owner, method.name.as_str(), args.len(), type_args))
            {
                let name: Option<Text> = result.as_ref()
                    .and_then(|ty| self.render_field_type_ref(ty, true)).map(Into::into);
                types = ExpressionTypes {
                    callable: result, name: name.clone(), chain: name.clone(), declared: name,
                };
                continue;
            }
            let callable = self.callable_method_result_from_receiver(expr, &types);
            let callable_name = callable
                .as_ref()
                .and_then(|ty| self.callable_result_name(ty));
            let chain = if callable.is_some() {
                callable_name.clone()
            } else {
                let type_namespace = matches!(&receiver.kind, ExprKind::Path(path)
                    if path.as_ident().is_some_and(|ident|
                        self.type_name_to_id.contains_key(ident.name.as_str())
                        && !self.ctx.variable_type_names.contains_key(ident.name.as_str())));
                if type_namespace {
                    None
                } else {
                    types
                        .name
                        .as_ref()
                        .or(types.chain.as_ref())
                        .and_then(|name| {
                            self.resolved_method_return_type_name(
                                name.as_str(),
                                method.name.as_str(),
                                type_args,
                            )
                        })
                        .map(Into::into)
                }
            };
            let name = chain.clone().or_else(|| {
                self.method_result_name_fallback(expr, types.name.as_ref().map(Text::as_str))
                    .map(Into::into)
            });
            let declared = callable_name.or_else(|| name.clone());
            types = ExpressionTypes {
                callable,
                name,
                chain,
                declared,
            };
        }
        types
    }

    fn declared_nominal_receiver_name(&self, receiver: &Expr) -> Maybe<Text> {
        if let Some(ty) = self.declaration_property_operand(receiver) {
            let name = Self::extract_type_name_from_ast(&ty);
            let base = Self::strip_generic_args(&name);
            if !self.ctx.generic_type_params.contains(base) && self.nominal_type_id(base).is_some()
            {
                return Some(name.into());
            }
        }
        None
    }

    fn callable_method_result_from_receiver(
        &self,
        expr: &Expr,
        receiver_types: &ExpressionTypes,
    ) -> Maybe<TypeRef> {
        let ExprKind::MethodCall {
            receiver,
            method,
            type_args,
            args,
        } = &expr.kind
        else {
            unreachable!("method expression");
        };
        let carried_receiver = receiver_types.callable.as_ref();
        // An associated function has no self argument. Its call operands can
        // specialize the owner, for example through a local factory's return.
        if carried_receiver.is_none()
            && self.declared_nominal_receiver_name(receiver).is_some()
            && !args.is_empty()
            && let Some(result) = self.source_call_declaration(expr, None, 0)
        {
            return Some(result);
        }
        if carried_receiver.is_none()
            && !args.iter().any(|arg| {
                let mut arg = arg;
                while let ExprKind::Paren(inner) = &arg.kind {
                    arg = inner;
                }
                matches!(&arg.kind, ExprKind::Closure { .. })
            })
        {
            // Associated constructors also produce temporary receivers. Their
            // source signatures are sufficient evidence even when the import
            // has no textual return hint or that hint names an exporting scope.
            let owner = receiver_types.declared.as_ref()?.to_string();
            return self.declared_method_result_type(&owner, method.name.as_str(), type_args);
        }
        let receiver_name = match carried_receiver {
            Some(ty) => self.callable_result_name(ty)?.to_string(),
            None => receiver_types.declared.as_ref()?.to_string(),
        };
        let mut owner = self
            .method_receiver_declaration_name(&receiver_name)?
            .to_string();
        for _ in 0..self.method_receiver_deref_count(&owner, method.name.as_str()) {
            owner = self.user_deref_target_type_name(&owner)?.to_string();
        }
        let key = self.registered_receiver_method(&owner, method.name.as_str())?;
        let info = self.ctx.lookup_qualified_function(&key)?;
        // Partial explicit lists are valid inference inputs. Surplus arguments
        // must not be truncated into a seemingly valid result instantiation.
        if type_args.len() > info.explicit_type_param_ids.len() {
            return None;
        }

        let local = self
            .functions
            .iter()
            .find(|function| function.descriptor.id == info.id)
            .map(|function| &function.descriptor);
        let result = local
            .map(|descriptor| &descriptor.return_type)
            .or(info.return_type.as_ref())?;
        let owner_id = self.nominal_type_id(Self::strip_generic_args(&owner))?;
        let owner_descriptor = self.type_by_id(owner_id)?;
        let mut substitution = match carried_receiver {
            Some(TypeRef::Instantiated { base, args })
                if *base == owner_id && args.len() == owner_descriptor.type_params.len() =>
            {
                crate::mono::TypeSubstitution::new(&owner_descriptor.type_params, args)
            }
            _ => self.callable_owner_substitution(&owner)?,
        };
        let mut bindings: std::collections::BTreeMap<u32, TypeRef> = info
            .type_param_ids
            .iter()
            .filter_map(|id| {
                substitution
                    .get(*id)
                    .map(|ty| (u32::from(id.0), ty.clone()))
            })
            .collect();
        let receiver_slots =
            usize::from(info.param_names.first().is_some_and(|name| name == "self"));
        for (index, arg) in args.iter().enumerate() {
            let index = index + receiver_slots;
            let parameter = local
                .and_then(|descriptor| descriptor.params.get(index).map(|p| &p.type_ref))
                .or_else(|| {
                    self.ctx
                        .archive_fn_param_types
                        .get(&info.id.0)
                        .and_then(|params| params.get(index))
                });
            if let Some(parameter) = parameter {
                self.bind_compiled_callable_argument(
                    info.id.0,
                    index,
                    parameter,
                    arg,
                    &mut bindings,
                );
            }
        }
        for (id, ty) in bindings {
            substitution.bind(crate::types::TypeParamId(id as u16), ty);
        }
        for (arg, id) in type_args.iter().zip(&info.explicit_type_param_ids) {
            if let (Some(ty), Some(id)) = (arg.type_for_declared_slot(), id)
                && let Some(ty) = self.explicit_type_witness(&ty, None)
            {
                substitution.bind(*id, ty);
            }
        }
        // The callee and caller have distinct binder scopes, even when their
        // numerical slot IDs coincide. Only a bound result can enter the caller.
        crate::codegen::constructor_types::fully_bound(result, &substitution)
            .then(|| substitution.apply(result).ok()).flatten()
    }

    fn method_result_name_fallback(
        &self,
        expr: &Expr,
        receiver_type: Maybe<&str>,
    ) -> Option<String> {
        let ExprKind::MethodCall {
            receiver,
            method,
            args,
            type_args,
        } = &expr.kind
        else {
            unreachable!("method expression");
        };
        // CONST-GENERIC-VALUE-CARRY-1 (task #19): static call on an
        // explicit TypeExpr receiver (`B<7>.new()`,
        // `StackAllocator<1024>.new()`).  Pre-fix this arm ignored
        // TypeExpr receivers entirely, so resolution fell through to
        // the short-name registry fallback below, which returns the
        // BARE declared return name ("B") — losing the callsite's
        // instantiation.  The callsite TypeExpr is the ONLY holder
        // of the const/type args, so a constructor-shaped return
        // (bare base / Self) re-attaches the FULL rendered
        // instantiation — `let b = B<7>.new()` records "B<7>", which
        // the receiver-driven witness derivation later parses back
        // into `ConstValue(7)`.
        if let ExprKind::TypeExpr(recv_ty) = &receiver.kind {
            let full = VbcCodegen::extract_type_name_from_ast(recv_ty);
            let full = self.method_receiver_declaration_name(&full)?.to_string();
            if !full.is_empty() {
                let base = VbcCodegen::strip_generic_args(&full).to_string();
                let qualified = format!("{}.{}", base, method.name);
                if let Some(func_info) = self.ctx.lookup_function(&qualified)
                    && let Some(ref ret_type_name) = func_info.return_type_name
                {
                    let resolved = VbcCodegen::substitute_self_in_type_name(ret_type_name, &full);
                    // CONST-GENERIC-RETNAME-SUBST-1 (#40): an explicit
                    // constructor return type that mentions the type's
                    // OWN symbolic params (`fn new() -> B<CAP>` instead
                    // of `Self`) surfaces `ret_type_name = "B<CAP>"`.
                    // `substitute_self_in_type_name` leaves it untouched
                    // (no `Self` token), so the binding would be typed by
                    // the UNBOUND param name and the receiver-driven const
                    // witness derivation parses no literal — the callsite
                    // degrades to a plain `Call` with no `ConstValue`
                    // type_arg (nil / wrong body result).  The receiver
                    // TypeExpr `full` ("B<16>") is the authoritative
                    // instantiation: when the return type's base is the
                    // receiver's own base, map the type's declared params
                    // onto `full`'s concrete args so `B<CAP>` → `B<16>`.
                    let resolved = {
                        let ret_base = VbcCodegen::strip_generic_args(&resolved).to_string();
                        if ret_base == base {
                            let params = self
                                .ctx
                                .type_generic_params
                                .get(&ret_base)
                                .cloned()
                                .unwrap_or_default();
                            let recv_args = VbcCodegen::split_generic_args(&full);
                            if !params.is_empty() && !recv_args.is_empty() {
                                VbcCodegen::substitute_generic_params_in_type_name(
                                    &resolved, &params, &recv_args,
                                )
                            } else {
                                resolved
                            }
                        } else {
                            resolved
                        }
                    };
                    if resolved == base && full.contains('<') {
                        return Some(full);
                    }
                    if let Some(ref inner) = func_info.return_type_inner
                        && !inner.is_empty()
                        && inner.iter().all(|s| !s.trim().is_empty())
                        && !resolved.contains('<')
                    {
                        return Some(format!("{}<{}>", resolved, inner.join(", ")));
                    }
                    return Some(resolved);
                }
            }
        }
        // First, check if this is a static method call on a type
        if let ExprKind::Path(path) = &receiver.kind
            && path.segments.len() == 1
            && let PathSegment::Name(ident) = &path.segments[0]
        {
            let name = &ident.name;
            // **Static-mut receiver classification** (4th site of the
            // static-mut-vs-type-namespace defect class; siblings live
            // in `try_resolve_static_method`'s value-receiver gates).
            // `GLOBAL_HAZARD_DOMAIN.scan_hazards()` is an INSTANCE
            // call on the static's declared type (`HazardDomain`),
            // but SCREAMING_CASE trips the is_uppercase type-name
            // heuristic below, whose miss-fallback returned the
            // static's NAME as the "type".  Downstream that junk
            // type-name qualified sibling method calls
            // ("GLOBAL_HAZARD_DOMAIN.binary_search") and defeated
            // the for-in iterator classification (`drain` result
            // lowered to native IterNew against a Drain record —
            // the hazard/reclaim SIGSEGV).  Resolve via the
            // static's declared type and never fall through.
            // FN-LOCAL-STATIC-ONCE-1: scope-first probe so a
            // hoisted fn-local static receiver (`SEED.load(...)`
            // inside its declaring fn) classifies by its
            // declared type too.
            if let Some(static_ty) = self.scoped_static_type_name(name) {
                let base_type = VbcCodegen::strip_generic_args(&static_ty);
                let qualified_method = format!("{}.{}", base_type, method.name);
                if let Some(func_info) = self.ctx.lookup_function(&qualified_method)
                    && let Some(ref ret_type_name) = func_info.return_type_name
                {
                    let resolved =
                        VbcCodegen::substitute_self_in_type_name(ret_type_name, &static_ty);
                    if let Some(ref inner) = func_info.return_type_inner
                        && !inner.is_empty()
                        && inner.iter().all(|s| !s.trim().is_empty())
                        && !resolved.contains('<')
                    {
                        return Some(format!("{}<{}>", resolved, inner.join(", ")));
                    }
                    return Some(resolved);
                }
                // The receiver IS a static binding; its method's
                // return type is simply unknown.  Returning the
                // static's name here would poison every consumer —
                // report "unknown" instead.
                return None;
            }
            if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                // Heap.new(x) / Shared.new(x): return the WRAPPER type
                // `Heap<T>` / `Shared<T>` (NOT the bare inner `T`).
                //
                // Returning the wrapper is required so that wrapper
                // METHODS dispatch correctly: `let b = Heap.new(rec);
                // b.is_valid()` must resolve to `Heap.is_valid`, and
                // `let s = Shared.new(v); s.strong_count()` to
                // `Shared.strong_count`.  Pre-fix this returned the
                // inner type, so the binding was typed `T` and the
                // method call dispatched to `T.is_valid` /
                // `T.strong_count` — not found, runtime kind Object
                // (the failing `core-tests/mem/allocator/integration_*`
                // Heap/Shared wrapper-method tests).  `Heap.new` /
                // `Shared.new` DO return a real wrapper record at
                // runtime (`Heap { ptr, generation, epoch }`), so the
                // wrapper-typed dispatch is sound.
                //
                // Field access through the wrapper (`b.field`) still
                // works because `resolve_field_index` auto-derefs a
                // `Heap<T>` / `Shared<T>` receiver to the inner `T`
                // when the field is not one of the wrapper's own
                // (ptr / generation / epoch) — see the auto-deref arm
                // in `resolve_field_index_impl`.
                if self.is_allocating_wrapper(name.as_str())
                    && method.name.as_str() == "new"
                    && self
                        .registered_receiver_method(name.as_str(), "new")
                        .is_some()
                    && let Some(first_arg) = args.first()
                    && let Some(inner_type) = self.extract_expr_type_name(first_arg)
                {
                    return Some(format!("{}<{}>", name, inner_type));
                }
                // Associated calls use the same declaration/alias resolver as
                // value receivers, retaining the selected owner's arguments.
                return self.resolved_method_return_type_name(
                    name,
                    method.name.as_str(),
                    type_args,
                );
            }
        }

        // Try to look up the method's return type for instance method calls.
        // If the receiver is a variable with a known type, construct the qualified
        // method name (e.g., "ValidFd.as_fd") and check for return_type_name.
        {
            // Receiver-type resolution: variable lookup for simple
            // Path receivers, RECURSIVE extraction for everything
            // else (field access `self.retired.drain(...)`, chained
            // calls, index, deref).  Pre-fix only single-segment
            // Path receivers resolved, so a method call on a
            // field/chain receiver reported None — the for-in
            // classifier then lowered stdlib `Drain` iterators
            // through native IterNew against the record layout
            // (hazard/reclaim SIGSEGV class).
            let receiver_type = match &receiver.kind {
                ExprKind::Path(path) if path.segments.len() == 1 => {
                    match &path.segments[0] {
                        PathSegment::Name(var_ident) => self
                            .ctx
                            .variable_type_names
                            .get(&*var_ident.name)
                            .cloned()
                            // A PRIMITIVE RECEIVER HAS A TYPE NAME TOO —
                            // the same fallback the infer sibling's Path
                            // arm carries (`a.cmp(b)` / Ordering-Display
                            // note). The name map only holds NOMINAL
                            // names, so `let used = 0;
                            // used.checked_add(mask)` resolved None here,
                            // the match SCRUTINEE went untyped, the
                            // pattern binder got no payload type, and the
                            // AOT static resolver lost the follow-up call
                            // (runtime type-switch → scalar receiver →
                            // fabricated 0: the checked_add tier-
                            // divergence micro-repro). The register-kind
                            // discriminator the codegen already keeps IS
                            // this fact — spell it as a name.
                            .or_else(|| {
                                Self::primitive_type_name(
                                    self.ctx.get_variable_type(&var_ident.name),
                                )
                                .map(|s| s.to_string())
                            }),
                        PathSegment::SelfValue => self.ctx.variable_type_names.get("self").cloned(),
                        _ => None,
                    }
                }
                _ => receiver_type.map(str::to_owned),
            };
            if let Some(ref receiver_type) = receiver_type {
                let base_type = VbcCodegen::strip_generic_args(receiver_type);
                let qualified_method = format!("{}.{}", base_type, method.name);
                if let Some(func_info) = self.ctx.lookup_function(&qualified_method)
                    && let Some(ref ret_type_name) = func_info.return_type_name
                {
                    // Resolve generic return types (e.g., "V" → "Node" from Map<Int, Node>)
                    if ret_type_name.len() <= 2
                        && ret_type_name.chars().all(|c| c.is_ascii_uppercase())
                        && let Some(resolved) = self.resolve_generic_return_type(
                            receiver_type,
                            base_type,
                            ret_type_name,
                        )
                    {
                        return Some(resolved);
                    }
                    // **Self → concrete substitution at READ-site**
                    // (task #11): defensive substitution so
                    // protocol default-method monomorphisations
                    // whose `return_type_name` is the literal
                    // `Self` still resolve to the concrete
                    // receiver type here.  Mirrors the same fix
                    // in `infer_expr_type_name`'s MethodCall arm.
                    let after_self =
                        VbcCodegen::substitute_self_in_type_name(ret_type_name, receiver_type);
                    // **Generic-param → concrete substitution**
                    // (task #12 §B): when the callee's
                    // `return_type_name` contains a generic
                    // param mention nested inside a wrapper
                    // (e.g. `Shared<T>` on `Shared.clone`'s
                    // signature), substitute the param with
                    // the corresponding concrete arg from
                    // `receiver_type`.  Without this, the
                    // literal `T` leaks into downstream
                    // tracking and method-name resolution emits
                    // `T.method` instead of `<concrete>.method`.
                    let params = self
                        .ctx
                        .functions
                        .get(&qualified_method)
                        .map(|_| ())
                        .and_then(|_| self.collection_type_params.get(base_type).cloned())
                        .unwrap_or_default();
                    let args = VbcCodegen::split_generic_args(receiver_type);
                    if !params.is_empty() && !args.is_empty() {
                        return Some(VbcCodegen::substitute_generic_params_in_type_name(
                            &after_self,
                            &params,
                            &args,
                        ));
                    }
                    // Archive-loaded FunctionInfo splits the declared
                    // return type into base ("Maybe") +
                    // `return_type_inner` (["Int"]) — the URL-8
                    // read-site composition the STATIC arm already
                    // performs. Instance methods load through the
                    // same archive path, so without this the
                    // scrutinee of `match recv.checked_add(n)` was
                    // typed bare "Maybe" and the pattern binder had
                    // no payload type to give the binding.
                    if let Some(ref inner) = func_info.return_type_inner
                        && !inner.is_empty()
                        && inner.iter().all(|s| !s.trim().is_empty())
                        && !after_self.contains('<')
                    {
                        return Some(format!("{}<{}>", after_self, inner.join(", ")));
                    }
                    return Some(after_self);
                }
            }
        }

        // General receiver type inference for non-Path receivers
        // (e.g., self.entries.offset(idx), field.method(), chained calls)
        {
            let receiver_type = receiver_type.map(str::to_owned);
            if let Some(ref rt) = receiver_type {
                let base_type = VbcCodegen::strip_generic_args(rt);
                let qualified_method = format!("{}.{}", base_type, method.name);
                if let Some(func_info) = self.ctx.lookup_function(&qualified_method)
                    && let Some(ref ret_type_name) = func_info.return_type_name
                {
                    // Resolve generic return types for non-Path receivers too
                    if ret_type_name.len() <= 2
                        && ret_type_name.chars().all(|c| c.is_ascii_uppercase())
                        && let Some(resolved) =
                            self.resolve_generic_return_type(rt, base_type, ret_type_name)
                    {
                        return Some(resolved);
                    }
                    // Same defensive `Self` substitution as
                    // above — applies to non-Path receivers
                    // (chained method calls, field-receivers).
                    let after_self = VbcCodegen::substitute_self_in_type_name(ret_type_name, rt);
                    // Mirror the Path-receiver arm's generic-
                    // param substitution (task #12 §B): when
                    // the callee's ret_type_name contains a
                    // generic param nested inside a wrapper
                    // (`Shared<T>`, `Maybe<T>`, …), substitute
                    // each param with the corresponding
                    // concrete arg from `rt`.
                    let params = self
                        .collection_type_params
                        .get(base_type)
                        .cloned()
                        .unwrap_or_default();
                    let args = VbcCodegen::split_generic_args(rt);
                    if !params.is_empty() && !args.is_empty() {
                        return Some(VbcCodegen::substitute_generic_params_in_type_name(
                            &after_self,
                            &params,
                            &args,
                        ));
                    }
                    // URL-8 read-site composition for field/chained
                    // receivers too — `self.used.checked_add(n)` is
                    // the arena's own shape.
                    if let Some(ref inner) = func_info.return_type_inner
                        && !inner.is_empty()
                        && inner.iter().all(|s| !s.trim().is_empty())
                        && !after_self.contains('<')
                    {
                        return Some(format!("{}<{}>", after_self, inner.join(", ")));
                    }
                    return Some(after_self);
                }
            }
        }

        // Without a selected receiver declaration there is no result evidence.
        // Agreement among unrelated same-named methods cannot provide it.
        None
    }
}
