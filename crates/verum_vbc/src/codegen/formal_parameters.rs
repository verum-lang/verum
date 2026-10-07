//! Declaration-owned formal types, separate from runtime parameter carriers.
//! Both ordinary source compilation and bootstrap use this producer. Missing
//! facts stay unknown; no ABI descriptor, spelling or resource flag recovers them.
use super::VbcCodegen;
use crate::types::{CbgrTier, Mutability, TypeId, TypeParamId, TypeRef};
use verum_ast::{FunctionDecl, FunctionParamKind};
use verum_common::List;

impl VbcCodegen {
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
        let saved = self.current_impl_semantic_target.replace(target.clone());
        let result = self.compile_function(function, name);
        self.current_impl_semantic_target = saved;
        result
    }

    pub(super) fn source_semantic_parameters(
        &self,
        function: &FunctionDecl,
        scope: &std::collections::HashMap<String, u16>,
        receiver_scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> List<Option<TypeRef>> {
        let receiver = self
            .current_impl_semantic_target
            .as_ref()
            .filter(|_| self.ctx.current_impl_type_name.is_some())
            .and_then(|target| self.semantic_formal_type(target, receiver_scope, declared_ids))
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
                        return self.semantic_formal_type(ty, scope, declared_ids);
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

    /// The general ABI resolver retains bare-name recovery for bootstrap.
    /// A semantic signature may only use a generic slot, an exact owner key,
    /// an explicit mount, or a primitive. A sibling's convenient bare alias
    /// is not evidence that this declaration can name that sibling's type.
    fn semantic_formal_path_is_declared(
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
        if Self::ast_type_is_bare_self(ast) {
            // A regular `Self` occurrence may be nested in another source
            // type. Until the common signature resolver takes the exact impl
            // target, do not recover it from the owner template's generics.
            return false;
        }
        if !path.is_single() {
            return self.type_name_to_id.contains_key(&name);
        }
        let owner = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        if self
            .type_name_to_id
            .contains_key(&format!("{owner}.{name}"))
        {
            return true;
        }
        if self.ctx.mounted_types.contains_key(&name) {
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
        self.nominal_type_id(&name)
            .is_some_and(|id| id.is_builtin())
    }

    fn semantic_formal_type(
        &self,
        ast: &verum_ast::Type,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> Option<TypeRef> {
        use verum_ast::ty::{GenericArg, TypeKind};
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
                TypeKind::Reference { inner, .. }
                | TypeKind::CheckedReference { inner, .. }
                | TypeKind::UnsafeReference { inner, .. }
                | TypeKind::Slice(inner) => supported(inner, depth + 1, budget, cg, scope),
                TypeKind::Tuple(parts) => parts
                    .iter()
                    .all(|p| supported(p, depth + 1, budget, cg, scope)),
                TypeKind::Generic { base, args } => {
                    supported(base, depth + 1, budget, cg, scope)
                        && matches!(cg.resolve_signature_type_ref(base, scope), TypeRef::Concrete(id)
                        if id != TypeId::PTR && id != TypeId::RESERVED)
                        && args.iter().all(|arg| {
                            matches!(arg, GenericArg::Type(ty)
                        if supported(ty, depth + 1, budget, cg, scope))
                        })
                }
                TypeKind::Function {
                    params,
                    return_type,
                    contexts,
                    calling_convention,
                } => {
                    calling_convention.is_none()
                        && contexts.requirements.is_empty()
                        && params
                            .iter()
                            .all(|p| supported(p, depth + 1, budget, cg, scope))
                        && supported(return_type, depth + 1, budget, cg, scope)
                }
                // The existing resolver has recovery encodings for array
                // counts, rank-2 binders, raw pointers and unresolved forms.
                // These encodings cannot establish this declaration fact.
                _ => false,
            }
        }
        if !supported(ast, 0, &mut 4096, self, scope) {
            return None;
        }
        let owner = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        let ty =
            self.resolve_type_ref_scoped_with_count(ast, scope, true, Some(owner), &mut |_| None);
        resolved(&ty, declared_ids, 0).then_some(ty)
    }
}

/// A declaration can mention types from another archive entry. Absence of its
/// source-owned map is Unknown, never permission to retain a colliding raw ID.
pub(super) fn remapped_parameters(
    function: &crate::FunctionDescriptor,
    map: &std::collections::HashMap<u32, u32>,
) -> Option<List<Option<TypeRef>>> {
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
            TypeRef::Generic(_) | TypeRef::ConstValue(_) => true,
            TypeRef::Reference { inner, .. }
            | TypeRef::Slice(inner)
            | TypeRef::Array { element: inner, .. }
            | TypeRef::AssociatedProjection { base: inner, .. } => known(inner, map, depth + 1),
            TypeRef::Instantiated { base, args } => {
                id_known(*base) && args.iter().all(|ty| known(ty, map, depth + 1))
            }
            TypeRef::Tuple(parts) => parts.iter().all(|ty| known(ty, map, depth + 1)),
            TypeRef::Function {
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
    function.semantic_parameter_types().map(|params| {
        params
            .iter()
            .map(|ty| {
                ty.as_ref()
                    .filter(|ty| known(ty, map, 0))
                    .map(|ty| super::remap_type_ref_archive(ty, map))
            })
            .collect()
    })
}

fn resolved(ty: &TypeRef, declared_ids: &[TypeParamId], depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    let concrete = |id: TypeId| id != TypeId::PTR && id != TypeId::RESERVED;
    match ty {
        TypeRef::Concrete(id) => concrete(*id),
        TypeRef::Generic(id) => declared_ids.contains(id),
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => {
            resolved(inner, declared_ids, depth + 1)
        }
        TypeRef::Tuple(parts) => parts.iter().all(|p| resolved(p, declared_ids, depth + 1)),
        TypeRef::Instantiated { base, args } => {
            concrete(*base) && args.iter().all(|p| resolved(p, declared_ids, depth + 1))
        }
        TypeRef::Function {
            params,
            return_type,
            contexts,
        } => {
            contexts.is_empty()
                && params.iter().all(|p| resolved(p, declared_ids, depth + 1))
                && resolved(return_type, declared_ids, depth + 1)
        }
        _ => false,
    }
}
