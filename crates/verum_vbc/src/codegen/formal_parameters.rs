//! Declaration-owned formal types, separate from runtime parameter carriers.
//! Both ordinary source compilation and bootstrap use this producer. Missing
//! facts stay unknown; no ABI descriptor, spelling or resource flag recovers them.
use super::VbcCodegen;
use crate::types::{CbgrTier, Mutability, TypeId, TypeParamId, TypeRef};
use verum_ast::{FunctionDecl, FunctionParamKind};
use verum_common::List;

/// Result facts come from source expressions while their bindings are live.
/// Missing or disagreeing exits invalidate the result; an emitted return with
/// no source fact (for example a residual edge) cannot acquire a unit default.
#[derive(Debug, Clone, Default)]
pub(super) struct SourceReturnTypes {
    exits: List<(usize, Option<TypeRef>)>,
}

impl SourceReturnTypes {
    pub(super) fn record(&mut self, instruction: usize, result: Option<TypeRef>) {
        self.exits.push((instruction, result));
    }

    pub(super) fn completed(&self, instructions: &[crate::Instruction]) -> Option<TypeRef> {
        let mut facts = self.exits.iter();
        let mut result = None;
        let mut seen = false;
        for (offset, instruction) in instructions.iter().enumerate() {
            if !matches!(instruction, crate::Instruction::Ret { .. } | crate::Instruction::RetV) {
                continue;
            }
            let (source_offset, source_type) = facts.next()?;
            if *source_offset != offset { return None; }
            result = if seen { super::place_copies::join_result_types(result, source_type.clone()) }
                else { source_type.clone() };
            seen = true;
        }
        (seen && facts.next().is_none()).then_some(result).flatten()
    }
}

/// A retained declaration is resolved in its own lexical and binder context,
/// independently of the function currently being emitted.
pub(super) struct DeclarationTypeScope<'a> {
    pub owner: &'a str,
    pub substitution: &'a super::source_substitution::SourceTypeSubstitution,
    pub receiver: Option<&'a super::ImplReceiverWitness>,
}

impl VbcCodegen {
    /// Resolve the target in the actual impl declaration, before any inherited
    /// method changes lexical scope. Method-local binders cannot define Self.
    pub(super) fn resolve_impl_receiver(
        &mut self,
        declaration: &verum_ast::decl::ImplDecl,
        dense: &std::collections::HashMap<String, u16>,
        impl_names: &[String],
    ) -> super::CodegenResult<super::ImplReceiverWitness> {
        let receiver_scope = dense.iter().filter(|(name, _)| impl_names.contains(name))
            .map(|(name, id)| (name.clone(), *id)).collect();
        let target = match &declaration.kind {
            verum_ast::decl::ImplKind::Inherent(target) => target,
            verum_ast::decl::ImplKind::Protocol { for_type, .. } => for_type,
        };
        let value_scope = self.implementation_value_scope(declaration, &receiver_scope)?;
        let previous = std::mem::replace(&mut self.current_source_substitution, value_scope);
        let result = self.resolve_source_type_witness(target, &receiver_scope,
            &Self::ordered_generic_param_ids(&receiver_scope, &Default::default()));
        self.current_source_substitution = previous;
        result
    }

    /// Retain the semantic authority produced by this exact source annotation.
    /// A resolved runtime carrier alone does not authorize protocol evidence.
    pub(super) fn resolve_source_type_witness(
        &self,
        ty: &verum_ast::Type,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> super::CodegenResult<super::ImplReceiverWitness> {
        let resolved = self.resolve_signature_type_ref(ty, scope)?;
        let semantic = self.semantic_formal_type(ty, scope, declared_ids).as_ref() == Some(&resolved);
        Ok(super::ImplReceiverWitness { ty: resolved, semantic })
    }

    /// Registration must reject unresolved annotations even for declarations
    /// with no body, before any executable or interface metadata is emitted.
    pub(super) fn validate_source_parameter_types(
        &self,
        function: &FunctionDecl,
        scope: &std::collections::HashMap<String, u16>,
    ) -> super::CodegenResult<()> {
        for parameter in &function.params {
            if let FunctionParamKind::Regular { ty, .. } = &parameter.kind {
                self.resolve_signature_type_ref(ty, scope)?;
            }
        }
        Ok(())
    }

    /// Keep the exact target only while compiling that source method. The
    /// normal ABI resolver cannot reconstruct `Container<Int>` from its name.
    pub(super) fn compile_declared_impl_function(
        &mut self,
        function: &FunctionDecl,
        implementation: &verum_ast::decl::ImplDecl,
        name: Option<&String>,
    ) -> super::CodegenResult<()> {
        let names = self.ctx.generic_type_params_ordered.clone();
        let dense = self.declared_impl_generic_scope(&implementation.kind, &names);
        let receiver = self.resolve_impl_receiver(implementation, &dense, &names)?;
        let equations = self.implementation_associated_equalities(implementation, &receiver, &dense)?;
        let saved = self.current_impl_receiver.replace(receiver);
        let saved_equations = std::mem::replace(&mut self.current_impl_associated_equalities, equations);
        let result = self.compile_function(function, name);
        self.current_impl_receiver = saved;
        self.current_impl_associated_equalities = saved_equations;
        result
    }

    /// The existing callable signature stores source meaning independently of
    /// ABI layouts. Only complete, supported declarations establish it.
    pub(super) fn source_callable_signature(
        &self,
        function: &FunctionDecl,
        scope: &std::collections::HashMap<String, u16>,
        receiver_scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> Option<TypeRef> {
        let return_type = self.source_semantic_result(function, scope, declared_ids)?;
        let params = self.source_semantic_parameters(function, scope, receiver_scope, declared_ids)
            .into_iter().collect::<Option<List<_>>>()?;
        Some(TypeRef::Function {
            is_async: function.is_async,
            params: params.into_iter().collect(),
            return_type: Box::new(return_type),
            contexts: Default::default(),
        })
    }

    pub(super) fn source_semantic_result(
        &self,
        function: &FunctionDecl,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> Option<TypeRef> {
        if function.is_generator || !function.contexts.is_empty() {
            return None;
        }
        if let Some(result) = &function.return_type {
            return self.semantic_formal_type_with_values(
                result, scope, declared_ids, &Self::source_value_binders(function),
            );
        }
        // A written empty block has one possible result. An omitted type on
        // a forward declaration or a body with a value still needs evidence.
        let verum_ast::FunctionBody::Block(block) = function.body.as_ref()? else { return None };
        (block.stmts.is_empty() && block.expr.is_none()
            && self.can_infer_source_result(function))
            .then_some(TypeRef::Concrete(TypeId::UNIT))
    }

    /// An emitted eager body's exits establish its result, including async
    /// bodies. A forward, external, intrinsic or generator callable needs its
    /// own producer's evidence.
    pub(super) fn can_infer_source_result(&self, function: &FunctionDecl) -> bool {
        function.body.is_some() && function.return_type.is_none()
            && !function.is_generator && function.contexts.is_empty()
            && function.throws_clause.is_none() && function.extern_abi.is_none()
            && self.extract_intrinsic_name(function).is_none()
    }

    pub(super) fn register_declared_impl_function(
        &mut self,
        function: &FunctionDecl,
        receiver: &super::ImplReceiverWitness,
        name: &str,
        generics: &[String],
    ) -> super::CodegenResult<crate::module::FunctionId> {
        let saved = self.current_impl_receiver.replace(receiver.clone());
        let result = self.register_impl_function(function, name, generics);
        self.current_impl_receiver = saved;
        result
    }

    pub(super) fn record_source_callable_bounds(
        &mut self,
        id: crate::module::FunctionId,
        function: &FunctionDecl,
        scope: &std::collections::HashMap<String, u16>,
        parameters: &[Option<TypeRef>],
    ) -> super::CodegenResult<()> {
        for (slot, (parameter, semantic)) in function.params.iter().zip(parameters).enumerate() {
            let (FunctionParamKind::Regular { ty, .. }, Some(generic @ TypeRef::Generic(_))) =
                (&parameter.kind, semantic)
            else {
                continue;
            };
            let bound = self.substitute_fn_bound_for_generic(generic.clone(), ty, function, scope)?;
            if matches!(bound, TypeRef::Function { .. }) {
                self.ctx.source_callable_bounds.insert((id, slot), bound);
            }
        }
        Ok(())
    }

    pub(super) fn source_semantic_parameters(
        &self,
        function: &FunctionDecl,
        scope: &std::collections::HashMap<String, u16>,
        _receiver_scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> List<Option<TypeRef>> {
        let values = Self::source_value_binders(function);
        let receiver = self.current_impl_receiver.as_ref()
            .filter(|receiver| receiver.semantic)
            .map(|receiver| receiver.ty.clone())
            .filter(|ty| {
                !matches!(ty, TypeRef::Concrete(id)
                if self.type_by_id(*id).is_some_and(|desc| !desc.type_params.is_empty()))
            });
        function
            .params
            .iter()
            .map(|param| {
                let (mutable, tier) = match &param.kind {
                    FunctionParamKind::Regular { ty, .. } => {
                        return self.semantic_formal_type_with_values(ty, scope, declared_ids, &values);
                    }
                    FunctionParamKind::SelfValue
                    | FunctionParamKind::SelfValueMut
                    | FunctionParamKind::SelfOwn
                    | FunctionParamKind::SelfOwnMut => return receiver.clone(),
                    FunctionParamKind::SelfRef => (false, CbgrTier::Tier0),
                    FunctionParamKind::SelfRefMut => (true, CbgrTier::Tier0),
                    FunctionParamKind::SelfRefChecked => (false, CbgrTier::Tier1),
                    FunctionParamKind::SelfRefCheckedMut => (true, CbgrTier::Tier1),
                    FunctionParamKind::SelfRefUnsafe => (false, CbgrTier::Tier2),
                    FunctionParamKind::SelfRefUnsafeMut => (true, CbgrTier::Tier2),
                };
                receiver.clone().map(|inner| TypeRef::Reference {
                    inner: Box::new(inner),
                    tier,
                    mutability: if mutable {
                        Mutability::Mutable
                    } else {
                        Mutability::Immutable
                    },
                })
            })
            .collect()
    }

    /// Top-level predicates constrain a value without introducing a storage layer.
    pub(super) fn parameter_storage_base(mut ty: &verum_ast::Type) -> super::CodegenResult<&verum_ast::Type> {
        let mut remaining = crate::types::MAX_TYPE_REF_DEPTH;
        while let verum_ast::TypeKind::Refined { base, .. } = &ty.kind {
            remaining = remaining.checked_sub(1).ok_or_else(|| super::CodegenError::unsupported_expr(
                "refined parameter exceeds the type nesting limit").at_source(ty.span))?;
            ty = base;
        }
        Ok(ty)
    }

    /// A refinement restricts admissible values without changing how its
    /// declared base is transferred or destroyed. This witness is used only
    /// by the ownership boundary, never as the parameter's semantic type.
    pub(super) fn refined_parameter_carrier(
        &self,
        function: &super::FunctionInfo,
        index: usize,
    ) -> Option<TypeRef> {
        use verum_ast::TypeKind;
        let descriptor = self.declared_callable_descriptors.get(&function.id)?;
        let signature = descriptor.source_signature.as_ref()?;
        let FunctionParamKind::Regular { ty, .. } = &signature.params.get(index)?.kind
            else { return None; };
        if !matches!(ty.kind, TypeKind::Refined { .. }) { return None; }
        let carrier = Self::parameter_storage_base(ty).ok()?;
        self.retained_formal_type(function, descriptor, signature, carrier)
    }

    pub(super) fn retained_formal_type(
        &self, function: &super::FunctionInfo, descriptor: &crate::module::FunctionDescriptor,
        signature: &FunctionDecl, ty: &verum_ast::Type,
    ) -> Option<TypeRef> {
        let roster = self.callable_generic_roster(function)?;
        // Locals follow inherited parameters in the roster and shadow them.
        let scope = roster.iter().filter_map(|parameter|
            Some((parameter.name.clone(), parameter.witness_slot?.0))).collect();
        let owner = match descriptor.origin_module {
            Some(id) => self.ctx.strings.get(id.0 as usize)?.as_str(),
            None => self.config.module_name.as_str(),
        };
        let receiver = descriptor.source_receiver.as_ref().map(|ty|
            super::ImplReceiverWitness { ty: ty.clone(), semantic: true });
        self.semantic_formal_type_in_declaration(ty, &scope,
            &function.type_param_ids, &Self::source_value_binders(signature),
            &DeclarationTypeScope {
                owner, substitution: &Default::default(), receiver: receiver.as_ref(),
            })
    }

    /// Source type lowering may only use a generic slot, an exact owner key,
    /// an explicit mount, a primitive, or a canonical prelude variant carrier.
    /// A sibling's convenient bare alias
    /// is not evidence that this declaration can name that sibling's type.
    pub(super) fn semantic_formal_path_is_declared(
        &self,
        ast: &verum_ast::Type,
        scope: &std::collections::HashMap<String, u16>,
    ) -> bool {
        let verum_ast::ty::TypeKind::Path(path) = &ast.kind else {
            return false;
        };
        let name = path.to_string().replace("::", ".");
        if path.is_single() && scope.contains_key(&name) {
            return true;
        }
        if path.is_single() && self.current_source_substitution.get(&name)
            .is_some_and(|witness| witness.semantic) {
            return true;
        }
        if Self::ast_type_is_bare_self(ast) {
            // The source mapper consumes exactly this retained witness, then
            // lifts it under local quantifiers. Flat owner names and template
            // positions cannot establish a semantic `Self` type.
            return self.current_impl_receiver.as_ref()
                .is_some_and(|receiver| receiver.semantic);
        }
        self.nominal_path_is_declared_in_scope(ast, self.lexical_type_scope())
    }

    pub(super) fn nominal_path_is_declared_in_scope(&self, ast: &verum_ast::Type, owner: &str) -> bool {
        let verum_ast::ty::TypeKind::Path(path) = &ast.kind else { return false; };
        let name = path.to_string().replace("::", ".");
        if !path.is_single() {
            return self.nominal_type_id_in_scope(&name, owner).is_some();
        }
        if self.selected_language_alias_in_scope(&name, owner).is_some() {
            return true;
        }
        if let Some(selected) = self.source_nominal_type_id_in_scope(&name, owner) { return selected.is_some(); }
        if self.mounted_type_path_in_scope(&name, owner).is_some() {
            return self.nominal_type_id_in_scope(&name, owner).is_some();
        }
        if self
            .type_name_claim_owner
            .get(&name)
            .is_some_and(|declaring_owner| {
                declaring_owner == owner
                    || (declaring_owner.starts_with("file:")
                        && self.ctx.current_source_module.is_none())
            })
        {
            return true;
        }
        if let Some(selected) = self.source_glob_type_id(&name, owner) {
            return selected.is_some();
        }
        self.nominal_type_id_in_scope(&name, owner).is_some_and(|id| id.is_builtin()
            || (id.well_known_name() == Some(name.as_str())
                && (self.is_builtin_ctor_collection(&name)
                    || verum_common::well_known_types::BUILTIN_VARIANT_CARRIERS
                        .iter().any(|(parent, _)| *parent == name))))
    }

    fn source_value_binders(function: &FunctionDecl) -> std::collections::HashSet<String> {
        use verum_ast::{visitor::{Visitor, walk_pattern}, Pattern, PatternKind, VariantPatternData};
        #[derive(Default)]
        struct Bindings(std::collections::HashSet<String>);
        impl Visitor for Bindings {
            fn visit_pattern(&mut self, pattern: &Pattern) {
                match &pattern.kind {
                    PatternKind::Ident { name, .. } => { self.0.insert(name.name.to_string()); }
                    PatternKind::Record { fields, .. }
                    | PatternKind::Variant { data: Some(VariantPatternData::Record { fields, .. }), .. } => {
                        for field in fields {
                            if field.pattern.is_none() { self.0.insert(field.name.name.to_string()); }
                        }
                    }
                    _ => {}
                }
                walk_pattern(self, pattern);
            }
        }
        let mut names = Bindings::default();
        for parameter in &function.params {
            if let FunctionParamKind::Regular { pattern, .. } = &parameter.kind {
                names.visit_pattern(pattern);
            }
        }
        for parameter in &function.generics {
            use verum_ast::ty::GenericParamKind;
            if let GenericParamKind::Const { name, .. } | GenericParamKind::Meta { name, .. } = &parameter.kind {
                names.0.insert(name.name.to_string());
            }
        }
        names.0
    }

    pub(super) fn semantic_formal_type(
        &self,
        ast: &verum_ast::Type,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> Option<TypeRef> {
        self.semantic_formal_type_with_values(ast, scope, declared_ids, &Default::default())
    }

    fn semantic_formal_type_with_values(
        &self,
        ast: &verum_ast::Type,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
        values: &std::collections::HashSet<String>,
    ) -> Option<TypeRef> {
        self.semantic_formal_type_in_declaration(ast, scope, declared_ids, values, &DeclarationTypeScope {
            owner: self.lexical_type_scope(),
            substitution: &self.current_source_substitution,
            receiver: self.current_impl_receiver.as_ref(),
        })
    }

    fn declaration_signature_type(
        &self, ast: &verum_ast::Type, scope: &std::collections::HashMap<String, u16>,
        declaration: &DeclarationTypeScope<'_>,
        count: &mut dyn FnMut(&verum_ast::Expr) -> Option<u64>,
    ) -> super::CodegenResult<TypeRef> {
        declaration.substitution.elaborate(scope, |scope, values| {
            self.resolve_type_ref_in_scope(ast, scope, true, Some(declaration.owner), count,
                declaration.receiver.map(|receiver| &receiver.ty), values, &[])
        })
    }

    pub(super) fn semantic_formal_type_in_declaration(
        &self,
        ast: &verum_ast::Type,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
        values: &std::collections::HashSet<String>,
        declaration: &DeclarationTypeScope<'_>,
    ) -> Option<TypeRef> {
        use verum_ast::ty::{GenericArg, TypeKind};
        fn parameter_path<'a>(
            ty: &verum_ast::Type,
            scope: &std::collections::HashMap<String, u16>,
            cg: &'a VbcCodegen,
            declaration: &DeclarationTypeScope<'a>,
            declared_ids: &[TypeParamId],
            binders: u32,
            retained: &mut List<&'a TypeRef>,
        ) -> Option<bool> {
            let TypeKind::Path(path) = &ty.kind else { return None; };
            if !path.is_single() { return None; }
            let name = path.to_string();
            if let Some(id) = scope.get(&name) {
                let index = u32::from(*id);
                return Some(index < binders
                    || declared_ids.contains(&TypeParamId((index - binders) as u16)));
            }
            let witness = if VbcCodegen::ast_type_is_bare_self(ty) {
                declaration.receiver
            } else {
                declaration.substitution.get(&name)
            }?;
            if witness.semantic {
                retained.push(&witness.ty);
            }
            Some(witness.semantic)
        }
        fn declared_projection_base<'a>(
            ty: &verum_ast::Type,
            scope: &std::collections::HashMap<String, u16>,
            depth: usize,
            cg: &'a VbcCodegen,
            declaration: &DeclarationTypeScope<'a>,
            declared_ids: &[TypeParamId],
            binders: u32,
            retained: &mut List<&'a TypeRef>,
        ) -> bool {
            if depth > 64 { return false; }
            match &ty.kind {
                TypeKind::Path(_) => parameter_path(ty, scope, cg, declaration, declared_ids, binders, retained)
                    .unwrap_or_else(|| cg.nominal_path_is_declared_in_scope(ty, declaration.owner)),
                TypeKind::AssociatedType { base, .. } => declared_projection_base(base, scope, depth + 1, cg, declaration, declared_ids, binders, retained),
                TypeKind::Qualified { self_ty, trait_ref, .. } => trait_ref.segments.is_empty()
                    && declared_projection_base(self_ty, scope, depth + 1, cg, declaration, declared_ids, binders, retained),
                _ => false,
            }
        }
        fn supported<'a>(
            ty: &verum_ast::Type,
            depth: usize,
            budget: &mut usize,
            cg: &'a VbcCodegen,
            declaration: &DeclarationTypeScope<'a>,
            scope: &std::collections::HashMap<String, u16>,
            declared_ids: &[TypeParamId],
            binders: u32,
            retained: &mut List<&'a TypeRef>,
        ) -> bool {
            if depth > 64 || *budget == 0 {
                return false;
            }
            *budget -= 1;
            match &ty.kind {
                TypeKind::Int
                | TypeKind::Float
                | TypeKind::Bool
                | TypeKind::Text
                | TypeKind::Char
                | TypeKind::Unit
                | TypeKind::Never => true,
                TypeKind::Path(_) => parameter_path(ty, scope, cg, declaration, declared_ids, binders, retained)
                    .unwrap_or_else(|| cg.nominal_path_is_declared_in_scope(ty, declaration.owner)),
                TypeKind::AssociatedType { base, .. } => declared_projection_base(base, scope, depth + 1, cg, declaration, declared_ids, binders, retained),
                TypeKind::Qualified { self_ty, trait_ref, .. } => trait_ref.segments.is_empty()
                    && declared_projection_base(self_ty, scope, depth + 1, cg, declaration, declared_ids, binders, retained),
                TypeKind::Reference { inner, .. }
                | TypeKind::CheckedReference { inner, .. }
                | TypeKind::UnsafeReference { inner, .. }
                | TypeKind::Pointer { inner, .. }
                | TypeKind::Slice(inner) => supported(inner, depth + 1, budget, cg, declaration, scope, declared_ids, binders, retained),
                TypeKind::Array { element, size } => {
                    size.is_some() && supported(element, depth + 1, budget, cg, declaration, scope, declared_ids, binders, retained)
                }
                TypeKind::Tuple(parts) => parts
                    .iter()
                    .all(|p| supported(p, depth + 1, budget, cg, declaration, scope, declared_ids, binders, retained)),
                TypeKind::Generic { base, args } => {
                    supported(base, depth + 1, budget, cg, declaration, scope, declared_ids, binders, retained)
                        && matches!(cg.declaration_signature_type(base, scope, declaration, &mut |_| None), Ok(TypeRef::Concrete(id))
                        if id != TypeId::PTR && id != TypeId::RESERVED)
                        && args.iter().all(|arg| {
                            match arg {
                                GenericArg::Type(ty) => supported(ty, depth + 1, budget, cg, declaration, scope, declared_ids, binders, retained),
                                // The scoped mapper below checks the exact value or
                                // declaration binder, including source substitutions.
                                GenericArg::Const(_) => true,
                                GenericArg::Lifetime(_) => true,
                                _ => false,
                            }
                        })
                }
                TypeKind::Function {
                    params,
                    return_type,
                    contexts,
                    calling_convention,
                    ..
                } => {
                    calling_convention.is_none()
                        && contexts.requirements.is_empty()
                        && params
                            .iter()
                            .all(|p| supported(p, depth + 1, budget, cg, declaration, scope, declared_ids, binders, retained))
                        && supported(return_type, depth + 1, budget, cg, declaration, scope, declared_ids, binders, retained)
                }
                TypeKind::Rank2Function {
                    type_params, params, return_type, contexts, calling_convention, where_clause,
                    ..
                } => {
                    let Ok((count, inner)) = VbcCodegen::rank2_type_scope(type_params, scope, ty.span) else {
                        return false;
                    };
                    let Some(binders) = binders.checked_add(u32::from(count)) else { return false; };
                    calling_convention.is_none()
                        && contexts.requirements.is_empty()
                        && where_clause.as_ref().is_none_or(|clause| clause.predicates.is_empty())
                        && params.iter().all(|p| supported(p, depth + 1, budget, cg, declaration, &inner, declared_ids, binders, retained))
                        && supported(return_type, depth + 1, budget, cg, declaration, &inner, declared_ids, binders, retained)
                }
                TypeKind::DynProtocol { .. } => cg.declaration_signature_type(ty, scope, declaration, &mut |_| None)
                    .is_ok_and(|resolved| resolved_in_scope(&resolved,
                        &mut |id| declared_ids.contains(&id), depth + 1, binders, false)),
                // Other unsupported forms do not establish an exact
                // declaration-owned semantic type.
                _ => false,
            }
        }
        let mut retained = List::new();
        if !supported(ast, 0, &mut 4096, self, declaration, scope, declared_ids, 0, &mut retained) {
            return None;
        }
        // Substitution transports an already checked type, including its free
        // binders. The lexical scope above validates source variables before
        // these witnesses are transported; coincident numeric IDs alone cannot
        // authorize an undeclared source variable.
        let mut admitted_ids = List::from_iter(declared_ids.iter().copied());
        for witness in &retained {
            if !resolved_in_scope(witness, &mut |id| {
                if !admitted_ids.contains(&id) { admitted_ids.push(id); }
                true
            }, 0, 0, false) { return None; }
        }
        let owner = declaration.owner;
        // A fixed extent requires its exact declaring scope. The wildcard
        // family bypasses this evaluator and never claims a numeric extent.
        let mut complete = true;
        let mut active = verum_common::Set::new();
        let mut cached = verum_common::Map::new();
        let mut array_count = |count: &verum_ast::Expr| {
            let shadowed = Self::constant_integer_dependencies(count).iter()
                .any(|name| {
                    let root = name.as_str().split('.').next().unwrap_or(name.as_str());
                    scope.contains_key(root) || values.contains(root)
                });
            let extent = (!shadowed).then(|| self.const_eval_i64_in_scope(
                count, Some(owner), None, true, &mut active, &mut cached, 0,
            ).ok().flatten()).flatten().and_then(|value| u64::try_from(value).ok());
            complete &= extent.is_some();
            extent
        };
        let ty = self.declaration_signature_type(ast, scope, declaration, &mut array_count).ok()?;
        (complete && resolved(&ty, &admitted_ids, 0)).then_some(ty)
    }
}

/// A declaration can mention types from another archive entry. Absence of its
/// source-owned map is Unknown, never permission to retain a colliding raw ID.
pub(super) fn remapped_parameters(
    function: &crate::FunctionDescriptor,
    map: &std::collections::HashMap<u32, u32>,
) -> Option<List<Option<TypeRef>>> {
    function.semantic_parameter_types().map(|params| {
        params.iter().map(|ty| ty.as_ref().and_then(|ty| remapped_declaration_type(ty, map))).collect()
    })
}

pub(super) fn remapped_receiver(
    function: &crate::FunctionDescriptor,
    map: &std::collections::HashMap<u32, u32>,
) -> Option<TypeRef> {
    remapped_declaration_type(function.source_receiver.as_ref()?, map)
}

pub(super) fn remapped_result(
    function: &crate::FunctionDescriptor,
    map: &std::collections::HashMap<u32, u32>,
) -> Option<TypeRef> {
    remapped_declaration_type(function.semantic_return.as_ref()?, map)
}

pub(super) fn remapped_callable(
    function: &crate::FunctionDescriptor,
    map: &std::collections::HashMap<u32, u32>,
) -> Option<TypeRef> {
    remapped_declaration_type(&function.semantic_callable_signature()?, map)
}

fn remapped_declaration_type(
    ty: &TypeRef,
    map: &std::collections::HashMap<u32, u32>,
) -> Option<TypeRef> {
    fn known(ty: &TypeRef, map: &std::collections::HashMap<u32, u32>, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        let id_known = |id: TypeId| {
            map.contains_key(&id.0)
                || (id.is_builtin() && id != TypeId::PTR && id != TypeId::RESERVED)
        };
        match ty {
            TypeRef::Dynamic(object) => object.bounds.iter().chain(object.bindings.iter().map(|binding| &binding.owner))
                .all(|application| id_known(TypeId(application.protocol.0)))
                && object.types().all(|ty| known(ty, map, depth + 1)),
            TypeRef::Concrete(id) => id_known(*id),
            TypeRef::RecordFieldProjection { owner, .. } => map.contains_key(&owner.0),
            TypeRef::Generic(_) | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) => true,
            TypeRef::Reference { inner, .. }
            | TypeRef::Slice(inner)
            | TypeRef::Array { element: inner, .. } => known(inner, map, depth + 1),
            TypeRef::AssociatedProjection { base, qualifier, args, .. } => known(base, map, depth + 1)
                && args.iter().all(|argument| known(argument, map, depth + 1))
                && qualifier.as_ref().is_none_or(|application| id_known(TypeId(application.protocol.0))
                    && application.args.iter().all(|argument| known(argument, map, depth + 1))),
            TypeRef::Instantiated { base, args } => {
                id_known(*base) && args.iter().all(|ty| known(ty, map, depth + 1))
            }
            TypeRef::Tuple(parts) => parts.iter().all(|ty| known(ty, map, depth + 1)),
            TypeRef::Function {
                is_async: _,
                params,
                return_type,
                contexts,
            }
            | TypeRef::Rank2Function {
                params,
                return_type,
                contexts,
                ..
            } => {
                contexts.is_empty()
                    && params.iter().all(|ty| known(ty, map, depth + 1))
                    && known(return_type, map, depth + 1)
            }
        }
    }
    known(ty, map, 0).then(|| super::remap_type_ref_archive(ty, map))
}

/// A source field has already resolved its own telescope. Retain that exact
/// declaration only when no component was erased to an opaque ABI carrier.
/// Callable scopes deliberately cannot admit the record's field projections.
pub(super) fn field_declaration_type(
    ty: TypeRef,
    parameters: &[crate::types::TypeParamDescriptor],
) -> Option<TypeRef> {
    resolved_in_scope(&ty, &mut |id| parameters.iter().any(|parameter| parameter.id == id), 0, 0, true)
        .then_some(ty)
}

pub(super) fn resolved(ty: &TypeRef, declared_ids: &[TypeParamId], depth: usize) -> bool {
    resolved_in_scope(ty, &mut |id| declared_ids.contains(&id), depth, 0, false)
}

fn resolved_in_scope(
    ty: &TypeRef,
    parameter: &mut impl FnMut(TypeParamId) -> bool,
    depth: usize,
    binders: u32,
    source_field: bool,
) -> bool {
    if depth > 64 {
        return false;
    }
    let concrete = |id: TypeId| id != TypeId::PTR && id != TypeId::RESERVED;
    match ty {
        TypeRef::Concrete(id) => concrete(*id),
        TypeRef::Generic(id) => {
            let index = u32::from(id.0);
            index < binders || parameter(TypeParamId((index - binders) as u16))
        }
        TypeRef::Dynamic(object) => binders.checked_add(1).is_some_and(|binders|
            object.types().all(|ty| resolved_in_scope(ty, parameter, depth + 1, binders, source_field))),
        TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) => true,
        // The record telescope supplies this binder, not the callable's scope.
        TypeRef::RecordFieldProjection { owner, .. } => source_field && concrete(*owner),
        TypeRef::AssociatedProjection { base, qualifier, assoc, args } => !assoc.is_empty()
            && resolved_in_scope(base, parameter, depth + 1, binders, source_field)
            && args.iter().all(|argument| resolved_in_scope(argument, parameter, depth + 1, binders, source_field))
            && qualifier.as_ref().is_none_or(|application|
                concrete(TypeId(application.protocol.0)) && application.args.iter().all(|argument|
                    resolved_in_scope(argument, parameter, depth + 1, binders, source_field))),
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner)
        | TypeRef::Array { element: inner, .. } => resolved_in_scope(inner, parameter, depth + 1, binders, source_field),
        TypeRef::Tuple(parts) => parts.iter().all(|p| resolved_in_scope(p, parameter, depth + 1, binders, source_field)),
        TypeRef::Instantiated { base, args } => {
            concrete(*base) && args.iter().all(|p| resolved_in_scope(p, parameter, depth + 1, binders, source_field))
        }
        TypeRef::Function {
            is_async: _,
            params,
            return_type,
            contexts,
        } => {
            (source_field || contexts.is_empty())
                && params.iter().all(|p| resolved_in_scope(p, parameter, depth + 1, binders, source_field))
                && resolved_in_scope(return_type, parameter, depth + 1, binders, source_field)
        }
        TypeRef::Rank2Function { is_async: _, type_param_count, params, return_type, contexts } => {
            let Some(binders) = binders.checked_add(u32::from(*type_param_count)) else { return false; };
            (source_field || contexts.is_empty())
                && params.iter().all(|p| resolved_in_scope(p, parameter, depth + 1, binders, source_field))
                && resolved_in_scope(return_type, parameter, depth + 1, binders, source_field)
        }
    }
}

#[cfg(test)]
mod record_projection_tests {
    use super::*;

    #[test]
    fn record_projection_transport_requires_its_nominal_owner() {
        let projection = TypeRef::RecordFieldProjection { owner: TypeId(2048), ordinal: 7 };
        assert!(remapped_declaration_type(&projection, &Default::default()).is_none());
        let map = std::collections::HashMap::from([(2048, 4096)]);
        assert_eq!(remapped_declaration_type(&projection, &map), Some(
            TypeRef::RecordFieldProjection { owner: TypeId(4096), ordinal: 7 }
        ));
        assert!(!resolved(&projection, &[TypeParamId(7)], 0));
        assert!(!resolved(&TypeRef::Function {
            is_async: false,
            params: vec![], return_type: Box::new(projection), contexts: Default::default(),
        }, &[TypeParamId(7)], 0));
    }
}
