//! Declaration-owned source signatures and inferred receiver method closure.

use super::TypeChecker;
use crate::{Result, Type, TypeError};
use verum_ast::{Span, decl::ImplItemKind};
use verum_common::{Set, Shared, Text};
use verum_modules::{ModuleInfo, ModulePath, visibility::VisibilityChecker};

/// A named source mount owns its local binding even when its target cannot be
/// resolved. Renaming replaces the original spelling; a glob does not itself
/// declare every possible name.
fn source_mount_binds_name(tree: &verum_ast::decl::MountTree, name: &str) -> bool {
    use verum_ast::decl::MountTreeKind;
    use verum_ast::ty::PathSegment;

    if let Some(alias) = &tree.alias {
        return alias.name.as_str() == name;
    }
    match &tree.kind {
        MountTreeKind::Path(path) => matches!(
            path.segments.last(),
            Some(PathSegment::Name(identifier)) if identifier.name.as_str() == name
        ),
        MountTreeKind::Nested { trees, .. } => {
            trees.iter().any(|tree| source_mount_binds_name(tree, name))
        }
        MountTreeKind::Glob(_) | MountTreeKind::File { .. } => false,
    }
}

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
        self.resolve_declaring_source_type_with_parameter_key(name, span)
            .map(|(ty, _)| ty)
    }

    pub(super) fn resolve_declaring_source_type_with_parameter_key(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<(Type, Option<Text>)> {
        if let Some((declaration, owner)) = self.declaring_source_type_declaration(name) {
            return self
                .load_source_type_declaration_with_parameter_key(&declaration, &owner, span)
                .map(|(ty, key)| (ty, Some(key)));
        }

        // Keep language primitives and a signature's explicit binders. A
        // consumer's nominal type or mount alias cannot provide a missing name.
        if let Some(ty) = self.ctx.lookup_type(name)
            && (ty.primitive_name().is_some()
                || matches!(ty, Type::Var(_) | Type::TypeConstructor { .. })
                || verum_common::well_known_types::type_names::is_primitive_value_type(name))
        {
            return Ok((ty.clone(), None));
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
            && let Some((ty, key)) =
                self.ensure_mounted_type_loaded_with_parameter_key(name, owner.as_str())
        {
            return Ok((ty, Some(key)));
        }
        Err(TypeError::TypeNotFound {
            name: name.into(),
            span,
        })
    }

    /// Resolve a lexical source name through its declaration and explicit
    /// mounts, retaining renamed declarations without inspecting the type body.
    pub(super) fn declaring_source_type_declaration(
        &self,
        name: &str,
    ) -> Option<(verum_ast::TypeDecl, Text)> {
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
    }

    /// A qualified source name selects its declaration before any template
    /// cache or leaf fallback. Loading an unmounted declaration must retain
    /// the same parameter/default/refinement authority as an explicit mount.
    pub(super) fn resolve_qualified_source_type_with_parameter_key(
        &mut self,
        qualified: &Text,
        span: Span,
    ) -> Result<Option<(Type, Text)>> {
        let Some((module_name, type_name)) = qualified.rsplit_once('.') else {
            return Ok(None);
        };
        let (module_owner, source_declaration) = {
            let registry = self.module_registry.read();
            let Some(module) = self.get_module_with_path_aliases(module_name, &registry) else {
                return Ok(None);
            };
            // Archive-backed modules can be empty, or contain source consumers
            // of declarations supplied only by metadata. Source declarations
            // and re-exports remain authoritative when they exist.
            if module.ast.items.is_empty() {
                return Ok(None);
            }
            let owner: Text = module.path.to_string().into();
            let declaration = if let Some(declaration) = Self::local_source_type_declaration(&module.ast, type_name) {
                let visibility = declaration.visibility.resolve_declared_scope(owner.as_str())
                    .map_err(|message| TypeError::VisibilityError {
                        name: type_name.into(), visibility: message,
                        module_path: owner.clone(), span,
                    })?;
                if !VisibilityChecker::new().is_visible(
                    visibility.clone(), &module.path,
                    &ModulePath::from_str(self.current_module_path.as_str()),
                ) {
                    return Err(TypeError::VisibilityError {
                        name: type_name.into(), visibility: format!("{visibility:?}").into(),
                        module_path: owner, span,
                    });
                }
                Some((declaration, owner.clone()))
            } else {
                // Reuse the exact re-export walker, with no parent-module
                // fallback. Only the declaring file may follow private mounts.
                self.find_type_declaration_with_source_module_inner(
                    &module.ast, type_name, &owner, &registry, &mut Set::new(),
                    true, owner == self.current_module_path,
                )
            };
            if declaration.is_none() && module.ast.items.iter().any(|item| {
                matches!(&item.kind, verum_ast::ItemKind::Mount(mount)
                    if source_mount_binds_name(&mount.tree, type_name))
            }) {
                // Private, missing and cyclic source routes do not authorize
                // substituting an unrelated archived declaration of this name.
                return Err(TypeError::TypeNotFound { name: qualified.clone(), span });
            }
            (owner, declaration)
        };
        if let Some((declaration, owner)) = source_declaration {
            return self.load_source_type_declaration_with_parameter_key(&declaration, &owner, span)
                .map(Some);
        }

        // A source miss is not proof that the module has no archived type.
        // Require the descriptor's declaring identity, not an index spelling
        // or same-leaf entry, before loading its exact parameter registration.
        let key: Text = format!("{module_owner}.{type_name}").into();
        let is_public = self.core_metadata.as_ref().and_then(|metadata| {
            Self::metadata_type_with_declaring_key(metadata, &key)
                .map(|descriptor| descriptor.is_public)
        }).ok_or_else(|| TypeError::TypeNotFound { name: qualified.clone(), span })?;
        if !is_public && module_owner != self.current_module_path {
            return Err(TypeError::VisibilityError {
                name: type_name.into(), visibility: "Private".into(),
                module_path: module_owner, span,
            });
        }
        self.ensure_mounted_type_loaded_with_parameter_key(type_name, module_owner.as_str())
            .map(Some)
            .ok_or_else(|| TypeError::TypeNotFound { name: qualified.clone(), span })
    }

    pub(super) fn load_source_type_declaration_with_parameter_key(
        &mut self,
        declaration: &verum_ast::TypeDecl,
        owner: &Text,
        span: Span,
    ) -> Result<(Type, Text)> {
        let key: Text = format!("{owner}.{}", declaration.name.name).into();
        if let Some(ty) = self.ctx.lookup_type(key.as_str()) {
            return Ok((ty.clone(), key));
        }
        // Loading a return type is not a mount of its lexical name or
        // constructors. Keep qualified declaration metadata, while restoring
        // this one pre-existing consumer binding on success and failure.
        let local_name = declaration.name.name.clone();
        let previous_type = self.ctx.type_defs.get(&local_name).cloned();
        let previous_alias = self.ctx.type_aliases.get(&local_name).cloned();
        let previous_unifier_alias = self.unifier.alias_binding(local_name.as_str());
        self.ctx.enter_scope();
        let registration =
            self.register_type_declaration_in_module(declaration, owner.as_str());
        self.ctx.exit_scope();
        match previous_type {
            Some(ty) => {
                self.ctx.type_defs.insert(local_name.clone(), ty);
            }
            None => {
                self.ctx.type_defs.remove(&local_name);
            }
        }
        match previous_alias {
            Some(ty) => {
                self.ctx.type_aliases.insert(local_name.clone(), ty);
            }
            None => {
                self.ctx.type_aliases.remove(&local_name);
            }
        }
        self.unifier
            .restore_alias_binding(local_name, previous_unifier_alias);
        registration?;
        self.ctx
            .lookup_type(key.as_str())
            .cloned()
            .map(|ty| (ty, key.clone()))
            .ok_or_else(|| TypeError::TypeNotFound { name: key, span })
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
