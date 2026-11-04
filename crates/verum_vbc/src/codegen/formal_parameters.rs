//! Declaration-owned formal types, separate from runtime parameter carriers.
//! Both ordinary source compilation and bootstrap use this producer. Missing
//! facts stay unknown; no ABI descriptor, spelling or resource flag recovers them.
use super::VbcCodegen;
use crate::types::{CbgrTier, Mutability, TypeId, TypeParamId, TypeRef};
use verum_ast::{FunctionDecl, FunctionParamKind};
use verum_common::List;

impl VbcCodegen {
    /// Resolve the target in the actual impl declaration, before any inherited
    /// method changes lexical scope. Method-local binders cannot define Self.
    pub(super) fn resolve_impl_receiver(
        &mut self,
        target: &verum_ast::Type,
        dense: &std::collections::HashMap<String, u16>,
        impl_names: &[String],
    ) -> super::CodegenResult<super::ImplReceiverWitness> {
        let receiver_scope = dense.iter().filter(|(name, _)| impl_names.contains(name))
            .map(|(name, id)| (name.clone(), *id)).collect();
        let value_scope = self.implementation_value_scope(target, &receiver_scope)?;
        let previous = std::mem::replace(&mut self.current_source_substitution, value_scope);
        let result = (|| {
            let ty = self.resolve_signature_type_ref(target, &receiver_scope)?;
            let semantic = self.semantic_formal_type(target, &receiver_scope,
                &Self::ordered_generic_param_ids(&receiver_scope, &Default::default())).as_ref() == Some(&ty);
            Ok(super::ImplReceiverWitness { ty, semantic })
        })();
        self.current_source_substitution = previous;
        result
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
        implementation: &verum_ast::decl::ImplKind,
        name: Option<&String>,
    ) -> super::CodegenResult<()> {
        let target = match implementation {
            verum_ast::decl::ImplKind::Inherent(target) => target,
            verum_ast::decl::ImplKind::Protocol { for_type, .. } => for_type,
        };
        let names = self.ctx.generic_type_params_ordered.clone();
        let (dense, _) = self.declared_impl_generic_param_maps(function, implementation, &names);
        let receiver = self.resolve_impl_receiver(target, &dense, &names)?;
        let saved = self.current_impl_receiver.replace(receiver);
        let result = self.compile_function(function, name);
        self.current_impl_receiver = saved;
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
        self.semantic_formal_type_with_values(function.return_type.as_ref()?, scope, declared_ids,
            &Self::source_value_binders(function))
    }

    pub(super) fn register_declared_impl_function(
        &mut self,
        function: &FunctionDecl,
        receiver: &super::ImplReceiverWitness,
        name: &str,
        generics: &[String],
    ) -> super::CodegenResult<()> {
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
        if !path.is_single() {
            return self.nominal_type_id(&name).is_some();
        }
        let owner = self.lexical_type_scope();
        if let Some(selected) = self.source_nominal_type_id(&name) { return selected.is_some(); }
        if self.mounted_type_path(&name).is_some() {
            return self.nominal_type_id(&name).is_some();
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
        self.nominal_type_id(&name).is_some_and(|id| id.is_builtin()
            || (id.well_known_name() == Some(name.as_str())
                && (self.is_builtin_ctor_collection(&name)
                    || verum_common::well_known_types::BUILTIN_VARIANT_CARRIERS
                        .iter().any(|(parent, _)| *parent == name))))
    }

    /// The grammar's explicit existential array extent, distinct from an
    /// unresolved named constant or a fixed zero-length array.
    pub(super) fn inferred_array_extent(expr: &verum_ast::Expr) -> bool {
        matches!(&Self::normalized_place(expr).kind, verum_ast::ExprKind::Path(path)
            if path.as_ident().is_some_and(|name| name.name.as_str() == "_"))
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
        use verum_ast::ty::{GenericArg, TypeKind};
        fn generic_projection_base(
            ty: &verum_ast::Type,
            scope: &std::collections::HashMap<String, u16>,
            depth: usize,
            cg: &VbcCodegen,
        ) -> bool {
            if depth > 64 { return false; }
            match &ty.kind {
                TypeKind::Path(path) => path.is_single() && (scope.contains_key(&path.to_string())
                    || cg.current_source_substitution.get(&path.to_string()).is_some_and(|witness| witness.semantic)),
                TypeKind::AssociatedType { base, .. } => generic_projection_base(base, scope, depth + 1, cg),
                TypeKind::Qualified { self_ty, trait_ref, .. } => trait_ref.segments.is_empty()
                    && generic_projection_base(self_ty, scope, depth + 1, cg),
                _ => false,
            }
        }
        fn supported(
            ty: &verum_ast::Type,
            depth: usize,
            budget: &mut usize,
            cg: &VbcCodegen,
            scope: &std::collections::HashMap<String, u16>,
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
                TypeKind::Path(_) => cg.semantic_formal_path_is_declared(ty, scope),
                TypeKind::AssociatedType { base, .. } => generic_projection_base(base, scope, depth + 1, cg),
                TypeKind::Qualified { self_ty, trait_ref, .. } => trait_ref.segments.is_empty()
                    && generic_projection_base(self_ty, scope, depth + 1, cg),
                TypeKind::Reference { inner, .. }
                | TypeKind::CheckedReference { inner, .. }
                | TypeKind::UnsafeReference { inner, .. }
                | TypeKind::Slice(inner) => supported(inner, depth + 1, budget, cg, scope),
                TypeKind::Array { element, size } => {
                    size.is_some() && supported(element, depth + 1, budget, cg, scope)
                }
                TypeKind::Tuple(parts) => parts
                    .iter()
                    .all(|p| supported(p, depth + 1, budget, cg, scope)),
                TypeKind::Generic { base, args } => {
                    supported(base, depth + 1, budget, cg, scope)
                        && matches!(cg.resolve_signature_type_ref(base, scope), Ok(TypeRef::Concrete(id))
                        if id != TypeId::PTR && id != TypeId::RESERVED)
                        && args.iter().all(|arg| {
                            match arg {
                                GenericArg::Type(ty) => supported(ty, depth + 1, budget, cg, scope),
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
                            .all(|p| supported(p, depth + 1, budget, cg, scope))
                        && supported(return_type, depth + 1, budget, cg, scope)
                }
                TypeKind::Rank2Function {
                    type_params, params, return_type, contexts, calling_convention, where_clause,
                    ..
                } => {
                    let Ok((_, inner)) = VbcCodegen::rank2_type_scope(type_params, scope, ty.span) else {
                        return false;
                    };
                    calling_convention.is_none()
                        && contexts.requirements.is_empty()
                        && where_clause.as_ref().is_none_or(|clause| clause.predicates.is_empty())
                        && params.iter().all(|p| supported(p, depth + 1, budget, cg, &inner))
                        && supported(return_type, depth + 1, budget, cg, &inner)
                }
                // Raw pointers and unsupported forms do not establish an
                // exact declaration-owned semantic type.
                _ => false,
            }
        }
        if !supported(ast, 0, &mut 4096, self, scope) {
            return None;
        }
        let owner = self.lexical_type_scope();
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
        let ty = self.resolve_type_ref_scoped_with_count(ast, scope, true, Some(owner), &mut array_count).ok()?;
        (complete && resolved(&ty, declared_ids, 0)).then_some(ty)
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
            TypeRef::Concrete(id) => id_known(*id),
            TypeRef::RecordFieldProjection { owner, .. } => map.contains_key(&owner.0),
            TypeRef::Generic(_) | TypeRef::ConstValue(_) => true,
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

pub(super) fn resolved(ty: &TypeRef, declared_ids: &[TypeParamId], depth: usize) -> bool {
    resolved_in_scope(ty, declared_ids, depth, 0)
}

fn resolved_in_scope(ty: &TypeRef, declared_ids: &[TypeParamId], depth: usize, binders: u32) -> bool {
    if depth > 64 {
        return false;
    }
    let concrete = |id: TypeId| id != TypeId::PTR && id != TypeId::RESERVED;
    match ty {
        TypeRef::Concrete(id) => concrete(*id),
        TypeRef::Generic(id) => {
            let index = u32::from(id.0);
            index < binders || declared_ids.contains(&TypeParamId((index - binders) as u16))
        }
        TypeRef::ConstValue(_) => true,
        // The record telescope supplies this binder, not the callable's scope.
        TypeRef::RecordFieldProjection { .. } => false,
        TypeRef::AssociatedProjection { base, qualifier, assoc, args } => !assoc.is_empty()
            && resolved_in_scope(base, declared_ids, depth + 1, binders)
            && args.iter().all(|argument| resolved_in_scope(argument, declared_ids, depth + 1, binders))
            && qualifier.as_ref().is_none_or(|application|
                concrete(TypeId(application.protocol.0)) && application.args.iter().all(|argument|
                    resolved_in_scope(argument, declared_ids, depth + 1, binders))),
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner)
        | TypeRef::Array { element: inner, .. } => resolved_in_scope(inner, declared_ids, depth + 1, binders),
        TypeRef::Tuple(parts) => parts.iter().all(|p| resolved_in_scope(p, declared_ids, depth + 1, binders)),
        TypeRef::Instantiated { base, args } => {
            concrete(*base) && args.iter().all(|p| resolved_in_scope(p, declared_ids, depth + 1, binders))
        }
        TypeRef::Function {
            is_async: _,
            params,
            return_type,
            contexts,
        } => {
            contexts.is_empty()
                && params.iter().all(|p| resolved_in_scope(p, declared_ids, depth + 1, binders))
                && resolved_in_scope(return_type, declared_ids, depth + 1, binders)
        }
        TypeRef::Rank2Function { is_async: _, type_param_count, params, return_type, contexts } => {
            let Some(binders) = binders.checked_add(u32::from(*type_param_count)) else { return false; };
            contexts.is_empty()
                && params.iter().all(|p| resolved_in_scope(p, declared_ids, depth + 1, binders))
                && resolved_in_scope(return_type, declared_ids, depth + 1, binders)
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
