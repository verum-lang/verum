//! Source-owned implementation admission shared by protocol and inherent methods.
use super::TypeChecker;
use crate::{Result, TypeError, ty::Type};
use crate::core_metadata::TypeDescriptor;
use verum_ast::source_implementation::AssociatedImplementationSource;
use verum_common::{List, Text};

#[derive(Debug)]
pub(super) struct ImportedStructuralMethod {
    provider: Text,
    span: verum_ast::Span,
    receiver: Type,
    scopes: verum_common::Set<Text>,
}

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
        self.with_implementation_owner(source.origin_module(), operation)
    }

    fn with_implementation_owner<T>(
        &mut self,
        owner: &str,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let previous_owner = std::mem::replace(&mut self.current_module_path, owner.into());
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

    /// The protocol owner can declare a blanket even when the goal's concrete
    /// receiver has no source module (for example a scalar). Loading that
    /// declaration supplies a candidate; its bounds are still proved normally.
    pub(super) fn load_protocol_owner_blankets(&mut self, protocol: &Text) -> Result<()> {
        use verum_ast::{ItemKind, decl::ImplKind};
        let Some((owner, _)) = protocol.rsplit_once('.') else { return Ok(()) };
        if self.blanket_impls_registered_modules.contains(owner) { return Ok(()); }
        let Some(module) = self.module_registry.read().get_by_path(owner) else { return Ok(()) };
        if !self.source_type_registration.loading_blankets.insert(protocol.clone()) { return Ok(()); }
        let result = self.with_implementation_owner(owner, |checker| {
            for item in &module.ast.items {
                let ItemKind::Impl(declaration) = &item.kind else { continue };
                let ImplKind::Protocol { protocol: path, for_type, .. } = &declaration.kind else { continue };
                if !checker.cfg_evaluator.should_include(&item.attributes)
                    || !checker.is_blanket_impl_for_type(for_type, &declaration.generics)
                {
                    continue;
                }
                let selected = checker.resolve_protocol_reference(path)?;
                if checker.path_to_string(&selected) == *protocol {
                    checker.register_blanket_impl_from_ast(declaration)?;
                }
            }
            Ok(())
        });
        self.source_type_registration.loading_blankets.remove(protocol);
        result
    }

    /// Unlike a nominal type, a structural receiver has no declaration to
    /// mount. A glob imports the signatures of its public inherent members.
    pub(super) fn import_structural_methods(
        &mut self,
        module: &verum_modules::ModuleInfo,
    ) -> Result<()> {
        use verum_ast::{
            ItemKind, TypeKind,
            decl::{ImplItemKind, ImplKind, Visibility},
        };
        let owner: Text = module.path.to_string().into();
        for item in &module.ast.items {
            let ItemKind::Impl(declaration) = &item.kind else {
                continue;
            };
            let ImplKind::Inherent(target) = &declaration.kind else {
                continue;
            };
            let family: Text = match target.kind {
                TypeKind::Array { .. } => "Array".into(),
                TypeKind::Slice(_) => "Slice".into(),
                _ => continue,
            };
            let members: Vec<_> = declaration.items.iter().filter(|member|
                matches!(&member.kind, ImplItemKind::Function(f) if f.visibility == Visibility::Public && f.params.iter().any(|p| p.is_self()))
            ).collect();
            if members.is_empty() {
                continue;
            }
            let header = verum_ast::source_signature::source_associated_implementation_items(
                declaration,
                &members,
            )
            .map_err(TypeError::Other)?;
            self.with_implementation_owner(owner.as_str(), |checker| {
                checker.register_impl_block(&header)
            })?;
            let methods = self.inherent_methods.read();
            for member in members {
                let ImplItemKind::Function(f) = &member.kind else {
                    unreachable!()
                };
                let scheme = methods
                    .get(&family)
                    .and_then(|methods| methods.get(&f.name.name));
                let Some(receiver) = scheme.and_then(|scheme| scheme.impl_self_type.as_ref())
                else {
                    return Err(TypeError::Other(
                        format!(
                            "missing structural implementation of {owner}.{}",
                            f.name.name
                        )
                        .into(),
                    ));
                };
                let declarations = self
                    .source_type_registration
                    .structural_methods
                    .entry(f.name.name.clone())
                    .or_default();
                if let Some(known) = declarations
                    .iter_mut()
                    .find(|entry| entry.provider == owner && entry.span == f.span)
                {
                    known.scopes.insert(self.current_module_path.clone());
                } else {
                    declarations.push(ImportedStructuralMethod {
                        provider: owner.clone(),
                        span: f.span,
                        receiver: receiver.clone(),
                        scopes: [self.current_module_path.clone()].into_iter().collect(),
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn check_structural_method_import(
        &self,
        receiver: &Type,
        member: &Text,
        span: verum_ast::Span,
    ) -> Result<()> {
        let Some(declarations) = self.source_type_registration.structural_methods.get(member)
        else {
            return Ok(());
        };
        let mut receiver = receiver;
        while let Type::Reference { inner, .. }
        | Type::CheckedReference { inner, .. }
        | Type::UnsafeReference { inner, .. } = receiver
        {
            receiver = inner;
        }
        if !matches!(receiver, Type::Slice { .. } | Type::Array { .. }) {
            return Ok(());
        }
        let mut visible: Vec<_> = declarations
            .iter()
            .filter(|entry| {
                entry.scopes.contains(&self.current_module_path)
                    || entry.provider == self.current_module_path
            })
            .filter(|entry| {
                matches!(
                    (&entry.receiver, receiver),
                    (Type::Slice { .. }, Type::Slice { .. } | Type::Array { .. })
                        | (Type::Array { .. }, Type::Array { .. })
                )
            })
            .collect();
        // Array members take precedence over the implicit borrowed slice view.
        if matches!(receiver, Type::Array { .. })
            && visible
                .iter()
                .any(|entry| matches!(entry.receiver, Type::Array { .. }))
        {
            visible.retain(|entry| matches!(entry.receiver, Type::Array { .. }));
        }
        if visible.len() > 1 {
            let mut candidates: List<Text> =
                visible.iter().map(|entry| entry.provider.clone()).collect();
            candidates.sort();
            return Err(TypeError::AmbiguousName {
                name: member.clone(),
                sources: candidates.join(", ").into(),
                span,
            });
        }
        if let Some(entry) = visible.first() {
            let pattern = &entry.receiver;
            let view;
            let actual =
                if let (Type::Slice { .. }, Type::Array { element, .. }) = (pattern, receiver) {
                    view = Type::Slice {
                        element: element.clone(),
                    };
                    &view
                } else {
                    receiver
                };
            if self
                .protocol_checker
                .read()
                .try_match_type(pattern, actual)
                .is_some()
            {
                return Ok(());
            }
        }
        Err(TypeError::MethodNotFound {
            method: member.clone(),
            ty: receiver.to_string().into(),
            span,
            did_you_mean: None,
            field_fn: None,
        })
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
