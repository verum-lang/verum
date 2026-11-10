//! Source-owned implementation admission shared by protocol and inherent methods.
use super::TypeChecker;
use crate::{Result, TypeError, ty::Type};
use crate::core_metadata::TypeDescriptor;
use verum_ast::source_implementation::AssociatedImplementationSource;
use verum_common::{List, Text};

impl TypeChecker {
    pub(super) fn metadata_implementation_target(descriptor: &TypeDescriptor) -> Type {
        // Only the global opaque catalogue represents a structural receiver.
        // Its element and extent are bound by the implementation header.
        if descriptor.module_path.is_empty() && descriptor.origin_module_path.is_none()
            && matches!(descriptor.kind, crate::core_metadata::TypeDescriptorKind::Opaque)
        {
            match descriptor.name.as_str() {
                "Slice" => return Type::Slice { element: Box::new(Type::Unknown) },
                "Array" => return Type::Array { element: Box::new(Type::Unknown), size: None },
                _ => {}
            }
        }
        let represented = super::helpers::parse_descriptor_type_string(&descriptor.name);
        if !matches!(represented, Type::Unknown)
            && represented.primitive_name().is_some_and(|name|
                Type::canonical_primitive(descriptor.name.as_str()) == name)
        { represented } else {
            Type::Named { path: Self::text_to_path(&Self::metadata_declaring_key(descriptor)), args: List::new() }
        }
    }

    pub(super) fn with_metadata_implementation_source<T>(
        &mut self,
        source: &AssociatedImplementationSource,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let previous_owner = std::mem::replace(&mut self.current_module_path, source.origin_module().into());
        let previous_signature_scope = std::mem::replace(&mut self.resolving_source_signature, true);
        let previous_parameters = std::mem::take(&mut self.decl_param_frames);
        // The importing function's generic variables are not declarations in
        // the implementation's lexical environment.
        let is_parameter = |ty: &Type| matches!(ty, Type::Var(_) | Type::TypeConstructor { .. } | Type::Meta { .. });
        let previous_bindings: List<_> = self.ctx.type_defs.iter()
            .filter(|(_, ty)| is_parameter(ty)).map(|(name, ty)| (name.clone(), ty.clone())).collect();
        for (name, _) in &previous_bindings { self.ctx.type_defs.remove(name); }
        self.ctx.enter_scope();
        let result = operation(self);
        self.ctx.exit_scope();
        self.ctx.type_defs.retain(|_, ty| !is_parameter(ty));
        for (name, ty) in previous_bindings { self.ctx.type_defs.insert(name, ty); }
        self.decl_param_frames = previous_parameters;
        self.resolving_source_signature = previous_signature_scope;
        self.current_module_path = previous_owner;
        result
    }

    pub(super) fn register_metadata_inherent_sources(
        &mut self,
        type_name: &Text,
        descriptor: &TypeDescriptor,
    ) -> Result<()> {
        let metadata = self.core_metadata.clone().ok_or_else(|| TypeError::Other("implementation admission requires an installed metadata snapshot".into()))?;
        let owner = Self::metadata_declaring_key(descriptor);
        let Some(sources) = metadata.inherent_implementations.get(&owner) else { return Ok(()); };
        let target = Self::metadata_implementation_target(descriptor);
        for (position, source) in sources.iter().enumerate() {
            let key = (owner.clone(), position);
            if self.metadata_inherent_receipts.contains(&key) || !self.metadata_inherent_loading.insert(key.clone()) { continue; }
            let result = (|| {
            let declaration = source.declaration().map_err(TypeError::Other)?;
            for item in &declaration.items {
                let verum_ast::decl::ImplItemKind::Function(function) = &item.kind else { continue; };
                let qualified: Text = format!("{}.{}", owner, function.name.name).into();
                let unqualified: Text = format!("{}.{}", descriptor.name, function.name.name).into();
                let owned = |method: &crate::core_metadata::FunctionDescriptor|
                    Self::metadata_function_owner(method, &metadata)
                        .is_some_and(|target| Self::metadata_declaring_key(target) == owner);
                let primary = metadata.functions.get(&qualified).or_else(|| metadata.functions.get(&unqualified))
                    .filter(|method| owned(method))
                    .ok_or_else(|| TypeError::Other(format!("inherent declaration has no owned method descriptor: {qualified}").into()))?;
                if let Some(signature) = &primary.source_signature && signature != function {
                    // The single lookup slot can represent a sibling. Replay
                    // the current header only with its own retained declaration.
                    let matches = metadata.functions.values().any(|method|
                        method.source_signature.as_ref() == Some(function) && owned(method)
                            && method.origin_module_path.as_ref().unwrap_or(&method.module_path).as_str() == source.origin_module());
                    if !matches {
                        return Err(TypeError::Other(format!("inherent method declaration disagrees with its signature: {qualified}").into()));
                    }
                }
            }
            self.with_metadata_implementation_source(source, |checker|
                checker.register_owned_inherent_impl_block(&declaration, &target))
            })();
            self.metadata_inherent_loading.remove(&key);
            result?;
            self.metadata_inherent_receipts.insert(key);
        }
        // A mount alias selects this exact declaration. Reuse its completed
        // schemes, preserving parameter identities and receiver constraints.
        if *type_name != owner {
            let mut methods = self.inherent_methods.write();
            if let Some(declared) = methods.get(&owner).cloned() {
                methods.entry(type_name.clone()).or_default().extend(declared);
            }
        }
        Ok(())
    }
}
