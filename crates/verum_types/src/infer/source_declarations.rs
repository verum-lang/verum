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

#[derive(Clone, Copy)]
enum MountTargetKind {
    Named,
    Glob,
}

/// Visit explicitly named targets and the glob surfaces that could export a spelling.
fn source_mount_targets(
    mount: &verum_ast::MountDecl,
    owner: &ModulePath,
    name: &str,
    mut visit: impl FnMut(ModulePath, MountTargetKind),
) {
    use verum_ast::{
        decl::{MountTree, MountTreeKind},
        ty::{Path, PathSegment},
    };
    use verum_modules::imports::path_to_module_path_resolved;

    fn walk<'a>(
        tree: &'a MountTree,
        alias: Option<&verum_ast::Ident>,
        name: &str,
        prefixes: &mut Vec<&'a Path>,
        visit: &mut impl FnMut(&[&Path], &Path, MountTargetKind),
    ) {
        match &tree.kind {
            MountTreeKind::Path(leaf) => {
                let Some(PathSegment::Name(last)) = leaf.segments.last() else {
                    return;
                };
                let binding = tree.alias.as_ref().or(alias).unwrap_or(last);
                if binding.name == name {
                    visit(prefixes, leaf, MountTargetKind::Named);
                }
            }
            MountTreeKind::Nested { prefix, trees } => {
                prefixes.push(prefix);
                for child in trees {
                    walk(child, None, name, prefixes, visit);
                }
                prefixes.pop();
            }
            MountTreeKind::Glob(path) => visit(prefixes, path, MountTargetKind::Glob),
            MountTreeKind::File { .. } => {}
        }
    }
    walk(
        &mount.tree,
        mount.alias.as_ref(),
        name,
        &mut Vec::new(),
        &mut |prefixes, leaf, kind| {
            if prefixes.is_empty() {
                visit(path_to_module_path_resolved(leaf, owner), kind);
            } else {
                let segments = prefixes
                    .iter()
                    .copied()
                    .chain(std::iter::once(leaf))
                    .flat_map(|part| part.segments.iter().cloned())
                    .collect();
                visit(
                    path_to_module_path_resolved(&Path::new(segments, mount.span), owner),
                    kind,
                );
            }
        },
    );
}

impl TypeChecker {
    /// Bind refinement names in the file that declared the type, using the
    /// same declaration inventory and import resolver as module loading. This
    /// establishes name availability only; it does not prove the predicate.
    pub(super) fn refinement_source_bindings(&self, owner: &Text) -> Option<Set<Text>> {
        use verum_modules::{ImportResolver, exports::{extract_exports_with_policy, ExportVisibilityPolicy}};
        let mut registry = self.module_registry.read().clone();
        // Inline bodies have the same lexical authority as file modules. Give
        // the existing resolver a snapshot entry instead of borrowing the
        // parent's imports or the consumer's flat bindings.
        if let Some(inline) = self.inline_modules.get(owner)
            && let Some(items) = &inline.items
        {
            let id = registry.allocate_id();
            let ast = verum_ast::Module::new(items.clone(), inline.span.file_id, inline.span);
            registry.register(ModuleInfo::new(
                id, ModulePath::from_str(owner), ast, inline.span.file_id, Text::new(),
            ));
        }
        let module = self.get_module_with_path_aliases(owner.as_str(), &registry)?;
        if module.ast.items.is_empty() {
            return None;
        }
        // WholeSurface is local to this lexical inventory: private declarations
        // are visible to their own predicates, but are not exported to callers.
        let Ok(local) = extract_exports_with_policy(
            &module.ast, module.id, &module.path, ExportVisibilityPolicy::WholeSurface,
        ) else { return Some(Set::new()); };
        let mut names: Set<Text> = local.all_exports()
            .filter(|(_, item)| !item.is_reexport())
            .map(|(name, _)| name.clone()).collect();
        let exports = registry.all_modules().map(|(id, m)| (*id, m.exports.clone())).collect();
        let paths = registry.all_modules().map(|(id, m)| (*id, m.path.clone())).collect();
        let mut resolver = ImportResolver::new();
        for item in &module.ast.items {
            if let verum_ast::ItemKind::Mount(mount) = &item.kind
                && let Ok(import) = resolver.resolve_import(mount, module.id, &exports, &paths)
            {
                names.extend(import.items.iter().map(|item| item.name.clone()));
            }
        }
        Some(names)
    }

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
        // A lexical declaration in the signature's own file takes precedence.
        // A flat archive index is not the declaration's import environment.
        let local_owner = self.core_metadata.as_ref().and_then(|metadata| {
            let key: Text = format!("{}.{name}", self.current_module_path).into();
            Self::metadata_type_with_declaring_key(metadata, &key)
                .map(|_| self.current_module_path.clone())
        });
        if let Some(owner) = local_owner
            && let Some((ty, key)) = self.ensure_mounted_type_loaded_with_parameter_key(name, owner.as_str())
        {
            return Ok((ty, Some(key)));
        }
        let (head, tail) = name.split_once('.').unwrap_or((name, ""));
        if let Some((ty, key)) = self.source_mounted_archive_type(head, tail, span)? {
            return Ok((ty, Some(key)));
        }
        let binding = self.core_metadata.as_ref().and_then(|metadata|
            metadata.declaration_type_bindings.get(&self.current_module_path)
                .and_then(|bindings| bindings.get(&Text::from(head))).cloned());
        if let Some(binding) = binding {
            // Retain an ambiguous or unresolved lexical binding as a refusal.
            // It must not fall through to a different declaration of `name`.
            if let Some(path) = binding {
                let path = if tail.is_empty() { path } else { format!("{path}.{tail}").into() };
                if let Some((ty, key)) = self.load_declaring_archive_type(&path, span)? {
                    return Ok((ty, Some(key)));
                }
            }
        } else if let Some((ty, key)) = self.load_declaring_archive_type(name, span)? {
            return Ok((ty, Some(key)));
        }
        Err(TypeError::TypeNotFound {
            name: name.into(),
            span,
        })
    }

    /// A retained namespace binding selects a declaration; it does not widen
    /// that declaration's visibility. Its own source may still name private types.
    fn load_declaring_archive_type(
        &mut self, path: &str, span: Span,
    ) -> Result<Option<(Type, Text)>> {
        let Some((owner, name)) = path.rsplit_once('.') else { return Ok(None) };
        let (declared_name, declared_owner) = self.reexport_source_module_for(owner, name)
            .unwrap_or_else(|| (name.into(), owner.into()));
        let key: Text = format!("{declared_owner}.{declared_name}").into();
        if let Some(descriptor) = self.core_metadata.as_ref()
            .and_then(|metadata| Self::metadata_type_with_declaring_key(metadata, &key))
            && !descriptor.is_public && declared_owner != self.current_module_path
        {
            return Err(TypeError::VisibilityError {
                name: declared_name, visibility: "Private".into(),
                module_path: declared_owner, span,
            });
        }
        Ok(self.ensure_mounted_type_loaded_with_parameter_key(name, owner))
    }

    /// A source module may privately mount an archive-only declaration in a public signature.
    fn source_mounted_archive_type(
        &mut self,
        head: &str,
        tail: &str,
        span: Span,
    ) -> Result<Option<(Type, Text)>> {
        let Some(metadata) = self.core_metadata.as_ref() else {
            return Ok(None);
        };
        let owner = ModulePath::from_str(self.current_module_path.as_str());
        let mut targets = Set::new();
        let mut explicit_globs = Vec::new();
        let mut implicit_globs = Vec::new();
        {
            let registry = self.module_registry.read();
            let inline = self
                .inline_modules
                .get(&self.current_module_path)
                .and_then(|module| module.items.as_ref());
            let file =
                self.get_module_with_path_aliases(self.current_module_path.as_str(), &registry);
            if let Some(items) = inline.or_else(|| file.as_ref().map(|module| &module.ast.items)) {
                for item in items {
                    if let verum_ast::ItemKind::Mount(mount) = &item.kind {
                        source_mount_targets(mount, &owner, head, |path, kind| match kind {
                            MountTargetKind::Named => {
                                targets.insert(path);
                            }
                            MountTargetKind::Glob if tail.is_empty() => {
                                let module = path.to_string();
                                let (leaf, declared_owner) = self
                                    .reexport_source_module_for(&module, head)
                                    .unwrap_or_else(|| (head.into(), module.into()));
                                let key: Text = format!("{declared_owner}.{leaf}").into();
                                if Self::metadata_type_with_declaring_key(metadata, &key)
                                    .is_some_and(|descriptor| descriptor.is_public)
                                {
                                    // The language injector prepends a synthetic prelude mount.
                                    // User-written mounts have priority, independent of source order.
                                    let globs = if mount.span.is_empty() {
                                        &mut implicit_globs
                                    } else {
                                        &mut explicit_globs
                                    };
                                    globs.push((leaf, declared_owner, path));
                                }
                            }
                            MountTargetKind::Glob => {}
                        });
                    }
                }
            }
        }
        let mut paths = targets.into_iter();
        let Some(path) = paths.next() else {
            let globs = if explicit_globs.is_empty() {
                implicit_globs
            } else {
                explicit_globs
            };
            let Some((leaf, declared_owner, _)) = globs.first() else {
                return Ok(None);
            };
            if globs.iter().any(|(name, owner, _)| name != leaf || owner != declared_owner) {
                return Err(TypeError::TypeNotFound { name: head.into(), span });
            }
            // A live declaration or an explicit re-export controls its own spelling,
            // including visibility. Stale archive metadata cannot override that route.
            for (_, _, surface) in &globs {
                let qualified: Text = format!("{surface}.{head}").into();
                let source_claims_name = {
                    let registry = self.module_registry.read();
                    self.get_module_with_path_aliases(&surface.to_string(), &registry)
                        .is_some_and(|module| module.ast.items.iter().any(|item| match &item.kind {
                            verum_ast::ItemKind::Type(decl) => decl.name.name == head,
                            verum_ast::ItemKind::Mount(mount) => source_mount_binds_name(&mount.tree, head),
                            _ => false,
                        }))
                };
                if source_claims_name
                    && let Some(resolved) = self.resolve_qualified_source_type_with_parameter_key(&qualified, span)?
                {
                    return Ok(Some(resolved));
                }
            }
            let key: Text = format!("{declared_owner}.{leaf}").into();
            if let Some(resolved) = self.resolve_qualified_source_type_with_parameter_key(&key, span)? {
                return Ok(Some(resolved));
            }
            return self.load_mounted_type(leaf, declared_owner, true)
                .map(Some)
                .ok_or(TypeError::TypeNotFound { name: head.into(), span });
        };
        if paths.next().is_some() {
            return Err(TypeError::TypeNotFound {
                name: head.into(),
                span,
            });
        }
        let path: Text = if tail.is_empty() {
            path.to_string()
        } else {
            format!("{path}.{tail}")
        }
        .into();
        let Some((module, leaf)) = path.rsplit_once('.') else {
            return Err(TypeError::TypeNotFound { name: path, span });
        };
        let (declared, owner) = self
            .reexport_source_module_for(module, leaf)
            .unwrap_or_else(|| (leaf.into(), module.into()));
        let key: Text = format!("{owner}.{declared}").into();
        if Self::metadata_type_with_declaring_key(metadata, &key).is_none() {
            return Err(TypeError::TypeNotFound { name: path, span });
        }
        if let Some(resolved) =
            self.resolve_qualified_source_type_with_parameter_key(&path, span)?
        {
            return Ok(Some(resolved));
        }
        self.load_mounted_type(leaf, module, true)
            .map(Some)
            .ok_or(TypeError::TypeNotFound { name: path, span })
    }

    /// Resolve a lexical source name through its declaration and explicit
    /// mounts, retaining renamed declarations without inspecting the type body.
    pub(super) fn declaring_source_type_declaration(
        &self,
        name: &str,
    ) -> Option<(verum_ast::TypeDecl, Text)> {
        // Inline and file modules own their local declarations equally. The
        // registry contains file modules; inline bodies live in this source
        // index and must not fall through to the consumer's flat bindings.
        if let Some(module) = self.inline_modules.get(&self.current_module_path)
            && let Some(items) = &module.items
            && let Some(declaration) = items.iter().find_map(|item| match &item.kind {
                verum_ast::ItemKind::Type(declaration) if declaration.name.name == name => Some(declaration),
                _ => None,
            })
        {
            return Some((declaration.clone(), self.current_module_path.clone()));
        }
        let registry = self.module_registry.read();
        let module = self.get_module_with_path_aliases(self.current_module_path.as_str(), &registry)?;
        let declared = self.find_type_declaration_with_source_module_inner(
            &module.ast, name, &Text::from(module.path.to_string()),
            &registry, &mut Set::new(), true, true,
        );
        // Written bindings remain authoritative even when their target is
        // missing or inaccessible; the prelude cannot repair an invalid mount.
        let explicitly_bound = module.ast.items.iter().any(|item| matches!(
            &item.kind, verum_ast::ItemKind::Mount(mount)
                if mount.alias.as_ref().is_some_and(|alias| alias.name.as_str() == name)
                || source_mount_binds_name(&mount.tree, name)
        ));
        if declared.is_some() || explicitly_bound { return declared; }
        let prelude = verum_ast::prelude::implicit_prelude_path(&module.ast)?;
        let prelude = self.get_module_with_path_aliases(prelude, &registry)?;
        self.find_type_declaration_with_source_module_inner(
            &prelude.ast, name, &Text::from(prelude.path.to_string()),
            &registry, &mut Set::new(), true, false,
        )
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
        // A nominal entry may only be a placeholder left by failed source
        // elaboration. Completed loads, and recursive loads in this active
        // transaction, are the only authority for a cache hit.
        if let Some(accepted) = self.source_type_registration.completed.get(&key)
            .or_else(|| self.source_type_registration.pending.get(&key))
        {
            if accepted != declaration {
                let message: Text = format!("conflicting source declarations while loading {key}").into();
                if self.source_type_registration.depth != 0 {
                    self.source_type_registration.failure = Some(message.clone());
                }
                return Err(TypeError::UniverseBinding { message, span });
            }
            let result = self.ctx.lookup_type(key.as_str()).cloned()
                .map(|ty| (ty, key.clone()))
                .ok_or_else(|| TypeError::TypeNotFound { name: key, span });
            if self.source_type_registration.depth != 0 && let Err(error) = &result {
                self.source_type_registration.failure = Some(error.to_string().into());
            }
            return result;
        }
        let outermost = self.source_type_registration.depth == 0;
        if outermost {
            self.ctx.begin_type_definition_journal();
            self.unifier.begin_variant_registration_journal();
        }
        self.source_type_registration.pending.insert(key.clone(), declaration.clone());
        self.source_type_registration.aliases.insert(key.clone(), self.unifier.alias_binding(key.as_str()));
        if matches!(declaration.body, verum_ast::TypeDeclBody::Protocol(_) | verum_ast::TypeDeclBody::Coinductive(_)) {
            self.source_type_registration.protocols.insert(key.clone(), super::source_type_registration::SourceProtocolRegistration {
                publications: Vec::new(),
                unifier_name: self.unifier.protocol_name_is_registered(&key),
                defaults: self.protocols_with_known_defaults.get(&key).cloned(),
            });
        }
        self.source_type_registration.depth += 1;
        self.source_type_registration.variants.arities.entry(declaration.name.name.clone())
            .or_insert_with(|| self.type_generics_count.get(&declaration.name.name).copied());
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
        let result = registration.and_then(|()| {
            self.ctx.lookup_type(key.as_str()).cloned()
                .map(|ty| (ty, key.clone()))
                .ok_or_else(|| TypeError::TypeNotFound { name: key.clone(), span })
        });
        self.source_type_registration.depth -= 1;
        if let Err(error) = &result {
            self.source_type_registration.failure = Some(error.to_string().into());
        }
        if outermost {
            let pending = std::mem::take(&mut self.source_type_registration.pending);
            let aliases = std::mem::take(&mut self.source_type_registration.aliases);
            let protocols = std::mem::take(&mut self.source_type_registration.protocols);
            if let Some(failure) = self.source_type_registration.failure.take() {
                // A nested declaration may have completed using this load's
                // provisional type. None of this group's nominal bindings may
                // remain usable after any member fails; retries must elaborate
                // them again. Lexical environment entries were already scoped.
                self.ctx.finish_type_definition_journal(false);
                self.finish_source_variant_registrations(false);
                for (name, binding) in aliases {
                    self.unifier.restore_alias_binding(name, binding);
                }
                for (name, binding) in protocols {
                    for publication in binding.publications.into_iter().rev() {
                        self.protocol_checker.write().restore_protocol_binding(publication);
                    }
                    self.unifier.restore_protocol_name(name.clone(), binding.unifier_name);
                    match binding.defaults {
                        Some(defaults) => { self.protocols_with_known_defaults.insert(name, defaults); }
                        None => { self.protocols_with_known_defaults.remove(&name); }
                    }
                }
                for name in pending.keys() {
                    self.ctx.type_defs.remove(name);
                    self.ctx.type_aliases.remove(name);
                    self.unifier.restore_alias_binding(name.clone(), (None, None));
                }
                return match result {
                    Err(error) => Err(error),
                    Ok(_) => Err(TypeError::UniverseBinding {
                        message: format!("source declaration load for {key} failed: {failure}").into(),
                        span,
                    }),
                };
            }
            self.ctx.finish_type_definition_journal(true);
            self.finish_source_variant_registrations(true);
            self.source_type_registration.completed.extend(pending);
        }
        result
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

    /// Load only the exact nominal owner's implementations of this required
    /// protocol before asking the immutable proof engine for evidence. An
    /// in-progress module import is not evidence, and unrelated implementations
    /// must not be checked first merely because they appear earlier in source.
    pub(super) fn load_source_bound_implementations(
        &mut self,
        receiver: &Type,
        bound: &crate::protocol::ProtocolBound,
        assumptions: &[crate::protocol::WhereClause],
        span: Span,
    ) -> Result<()> {
        let Some((module, declaration)) = self.source_receiver_declaration(receiver) else {
            return Ok(());
        };
        let owner = module.path.to_string();
        let protocol = self.path_to_string(&bound.protocol);
        let key = (format!("{owner}.{}", declaration.name.name).into(), protocol.clone());
        if !self.source_type_registration.active_impl_bounds.insert(key.clone()) {
            // An active declaration is not evidence. A previously published
            // implementation or an explicit caller assumption may still prove
            // this exact instantiated goal independently of that declaration.
            return self.protocol_checker.read()
                .check_universal_bounds(receiver, std::slice::from_ref(bound), assumptions)
                .map_err(|error| TypeError::OtherWithCodeSpanned {
                    code: "E405".into(),
                    msg: format!("recursive source implementation obligation has no independent evidence: {error}").into(),
                    span,
                });
        }
        // The selected declaration owns its binders. The suspended caller's
        // same-named parameters and projection assumptions cannot capture them.
        let frames = std::mem::take(&mut self.decl_param_frames);
        let assumptions = std::mem::take(
            &mut self.source_type_registration.projection_assumptions,
        );
        let result = self.import_impl_blocks_for_type_in_module_selected(
            &module.ast, declaration.name.name.as_str(), Some(owner.as_str()), Some(&protocol),
        );
        self.decl_param_frames = frames;
        self.source_type_registration.projection_assumptions = assumptions;
        self.source_type_registration.active_impl_bounds.remove(&key);
        result
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
