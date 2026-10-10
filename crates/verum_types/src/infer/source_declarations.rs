//! Declaration-owned source signatures and inferred receiver method closure.

use super::TypeChecker;
use crate::{Result, Type, TypeError};
use verum_ast::{Span, decl::ImplItemKind};
use verum_common::{Set, Shared, Text};
use verum_modules::{ModuleInfo, ModulePath, visibility::VisibilityChecker};

impl TypeChecker {
    /// Local declaration authority is independent of whether an importer may
    /// name the type. A public function may return a privately declared type.
    pub(super) fn local_source_type_declaration(
        ast: &verum_ast::Module,
        name: &str,
    ) -> Option<verum_ast::TypeDecl> {
        ast.items.iter().find_map(|item| match &item.kind {
            verum_ast::ItemKind::Type(declaration) if declaration.name.name.as_str() == name => {
                Some(declaration.clone())
            }
            _ => None,
        })
    }

    /// An imported signature belongs to the declaring file, including its
    /// private mounts. Re-export targets still need their own public authority.
    pub(super) fn resolve_declaring_source_type(&mut self, name: &str, span: Span) -> Result<Type> {
        let declaration = {
            let registry = self.module_registry.read();
            self.get_module_with_path_aliases(self.current_module_path.as_str(), &registry)
                .and_then(|module| {
                    self.find_type_declaration_with_source_module_inner(
                        &module.ast,
                        name,
                        &Text::from(module.path.to_string()),
                        &registry,
                        &mut Set::new(),
                        true,
                        true,
                    )
                })
        };
        if let Some((declaration, owner)) = declaration {
            let key: Text = format!("{owner}.{}", declaration.name.name).into();
            if let Some(ty) = self.ctx.lookup_type(key.as_str()) {
                return Ok(ty.clone());
            }
            self.register_type_declaration_in_module(&declaration, owner.as_str())?;
            return self
                .ctx
                .lookup_type(key.as_str())
                .cloned()
                .ok_or_else(|| TypeError::TypeNotFound { name: key, span });
        }

        // Keep language primitives and a signature's explicit binders. A
        // consumer's nominal type or mount alias cannot provide a missing name.
        if let Some(ty) = self.ctx.lookup_type(name)
            && (ty.primitive_name().is_some()
                || matches!(ty, Type::Var(_) | Type::TypeConstructor { .. })
                || verum_common::well_known_types::type_names::is_primitive_value_type(name))
        {
            return Ok(ty.clone());
        }
        // The existing ambient archive interface remains available, but use
        // its descriptor authority rather than a consumer's source binding.
        let metadata_owner = self.core_metadata.as_ref().and_then(|metadata| {
            metadata.types.get(&Text::from(name)).map(|descriptor| {
                descriptor
                    .origin_module_path
                    .as_ref()
                    .filter(|owner| !owner.is_empty())
                    .unwrap_or(&descriptor.module_path)
                    .clone()
            })
        });
        if let Some(owner) = metadata_owner
            && let Some(ty) = self.ensure_mounted_type_loaded_qualified(name, owner.as_str())
        {
            return Ok(ty);
        }
        Err(TypeError::TypeNotFound {
            name: name.into(),
            span,
        })
    }

    /// Resolve only a nominal key whose exact source module declares the type.
    /// An umbrella spelling, unknown owner, or same-leaf sibling is not proof.
    pub(super) fn source_receiver_declaration(
        &self,
        receiver: &Type,
    ) -> Option<(Shared<ModuleInfo>, verum_ast::TypeDecl)> {
        let key = self.get_type_name(receiver)?;
        let (owner, name) = key.rsplit_once('.')?;
        let registry = self.module_registry.read();
        let module = self.get_module_with_path_aliases(owner, &registry)?;
        if module.path.to_string() != owner {
            return None;
        }
        let declaration = Self::local_source_type_declaration(&module.ast, name)?;
        Some((module, declaration))
    }

    pub(super) fn load_source_receiver_methods(&mut self, receiver: &Type) -> Result<()> {
        let Some((module, declaration)) = self.source_receiver_declaration(receiver) else {
            return Ok(());
        };
        let name = declaration.name.name;
        let owner = module.path.to_string();
        let key: Text = format!("{owner}.{name}").into();
        if !self.loaded_source_receiver_methods.insert(key.clone()) {
            return Ok(());
        }
        let result = self.import_impl_blocks_for_type_in_module(
            &module.ast,
            name.as_str(),
            Some(owner.as_str()),
        );
        if result.is_err() {
            self.loaded_source_receiver_methods.remove(&key);
        }
        result
    }

    /// Retain private methods for checking their owner's implementation, but
    /// consult declaration visibility before any caller-side dispatch can win.
    pub(super) fn check_source_method_visibility(
        &self,
        receiver: &Type,
        method: &str,
        span: Span,
    ) -> Result<()> {
        let Some((module, declaration)) = self.source_receiver_declaration(receiver) else {
            return Ok(());
        };
        let name = declaration.name.name;
        let caller = ModulePath::from_str(self.current_module_path.as_str());
        let visibility = VisibilityChecker::new();
        for implementation in self.find_impl_blocks_for_type(&module.ast, name.as_str()) {
            if !matches!(implementation.kind, verum_ast::decl::ImplKind::Inherent(_)) {
                continue;
            }
            for item in &implementation.items {
                if let ImplItemKind::Function(function) = &item.kind
                    && function.name.name.as_str() == method
                    && !visibility.is_visible(item.visibility.clone(), &module.path, &caller)
                {
                    return Err(TypeError::MethodNotFound {
                        ty: self.get_type_name(receiver).unwrap_or(name),
                        method: method.into(),
                        span,
                        did_you_mean: None,
                        field_fn: None,
                    });
                }
            }
        }
        Ok(())
    }
}
