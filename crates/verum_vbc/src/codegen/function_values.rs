//! Function values keep the same source declaration before and after emission.
use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, VbcCodegen};
use crate::{module::{FunctionDescriptor, FunctionId}, types::{StringId, TypeRef}};
use verum_ast::{Expr, ExprKind};
use verum_common::{List, Set, Text};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum FunctionValueScope {
    Module(Option<StringId>),
    Callable(FunctionId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FunctionValueFamily {
    scope: FunctionValueScope,
    name: Text,
}

impl VbcCodegen {
    /// A local value shadows functions. Nested callables are selected by their
    /// retained lexical parent IDs; module functions use the source index.
    /// Neither search consults an ambient bare-name function registration.
    pub(super) fn declared_function_value(&self, expr: &Expr) -> CodegenResult<Option<&FunctionInfo>> {
        let ExprKind::Path(path) = &expr.kind else { return Ok(None); };
        let Some(name) = path.as_ident() else { return Ok(None); };
        let name = name.as_str();
        if self.ctx.lookup_var(name).is_some() { return Ok(None); }
        let mut parent = self.current_source_callable_parent;
        let mut visited = Set::new();
        while let Some(id) = parent {
            if !visited.insert(id) || visited.len() > crate::types::MAX_TYPE_REF_DEPTH {
                return Err(CodegenError::internal("invalid source callable parent chain"));
            }
            let candidates = self.function_value_candidates(&FunctionValueFamily {
                scope: FunctionValueScope::Callable(id), name: name.into(),
            });
            if !candidates.is_empty() { return self.select_function_value(name, candidates); }
            parent = self.declared_callable_descriptors.get(&id)
                .and_then(|declaration| declaration.declaration_identity.as_ref())
                .and_then(|identity| identity.lexical_parent);
        }
        if let Some(primary) = self.ctx.lookup_declared_function(name)
            && self.is_local_function_value(name, primary)
        {
            return self.function_value_family(primary);
        }
        if let Some(selected) = self.source_mounted_value(name) {
            let selected = selected?;
            return if selected.kind == super::source_mounts::SourceValueKind::Callable {
                self.function_value_family(selected.function)
            } else { Ok(None) };
        }
        Ok(None)
    }

    /// The legacy scoped registry also contains mount aliases and synthetic
    /// constructors. Its row alone is not a source function declaration.
    fn is_local_function_value(&self, name: &str, primary: &FunctionInfo) -> bool {
        let Some(declaration) = self.declared_callable_descriptors.get(&primary.id) else { return false; };
        let Some(family) = self.function_value_family_key(declaration) else { return false; };
        let FunctionValueScope::Module(owner) = family.scope else { return false; };
        let owner = owner.and_then(|id| self.ctx.strings.get(id.0 as usize)).map(String::as_str);
        family.name.as_str() == name && owner == self.ctx.lexical_source_scope()
    }

    fn function_value_family(&self, primary: &FunctionInfo) -> CodegenResult<Option<&FunctionInfo>> {
        let family = self.declared_callable_descriptors.get(&primary.id)
            .and_then(|declaration| self.function_value_family_key(declaration))
            .ok_or_else(|| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                "selected function value has no declaration-owned forward contract".into())))?;
        self.select_function_value(family.name.as_str(), self.function_value_candidates(&family))
    }

    pub(super) fn function_value_family_key(&self, declaration: &FunctionDescriptor) -> Option<FunctionValueFamily> {
        if declaration.parent_type.is_some() || declaration.source_receiver.is_some() || declaration.is_const { return None; }
        let identity = declaration.declaration_identity.as_ref()?;
        let name = self.ctx.strings.get(identity.source_name.0 as usize)?;
        Some(FunctionValueFamily {
            scope: identity.lexical_parent.map(FunctionValueScope::Callable)
                .unwrap_or(FunctionValueScope::Module(declaration.origin_module)),
            name: name.as_str().into(),
        })
    }

    fn function_value_candidates(&self, family: &FunctionValueFamily) -> List<&FunctionInfo> {
        self.declared_function_value_families.get(family).into_iter().flatten()
            .filter_map(|id| self.declared_callable_descriptors.get(id))
            .filter_map(|declaration| self.ctx.lookup_function_by_id(declaration.id))
            .filter(|function| self.is_function_value(function)).collect()
    }

    fn is_function_value(&self, function: &FunctionInfo) -> bool {
        function.variant_tag.is_none() && function.parent_type_name.is_none() && !function.is_const
    }

    fn select_function_value<'a>(&self, name: &str, candidates: List<&'a FunctionInfo>) -> CodegenResult<Option<&'a FunctionInfo>> {
        if candidates.len() <= 1 { return Ok(candidates.first().copied()); }
        let expected = self.ctx.expected_value_type.as_ref();
        let mut matching = candidates.iter().copied().filter(|function|
            expected.is_some_and(|expected| self.function_value_signature(function, Some(expected)).as_ref() == Some(expected)));
        if let Some(selected) = matching.next() && matching.next().is_none() { return Ok(Some(selected)); }
        Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
            format!("ambiguous declared function value `{name}` requires a callable context"))))
    }

    /// Generic function parameters belong to the selected declaration, never
    /// to coincidentally numbered caller slots. Context must bind every slot.
    pub(super) fn function_value_signature(&self, function: &FunctionInfo, expected: Option<&TypeRef>) -> Option<TypeRef> {
        let signature = function.callable_signature.as_ref()?;
        if function.type_param_ids.is_empty() {
            return super::formal_parameters::resolved(signature, &[], 0).then(|| signature.clone());
        }
        let expected = expected.filter(|ty| matches!(ty, TypeRef::Function { .. } | TypeRef::Rank2Function { .. }))?;
        let mut bindings = crate::mono::TypeSubstitution::empty();
        if !self.bind_source_declaration(signature, expected, &mut bindings)
            || function.type_param_ids.iter().any(|id| bindings.get(*id).is_none()) { return None; }
        bindings.apply(signature).ok()
    }
}
