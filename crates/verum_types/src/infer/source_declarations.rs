//! Declaration-owned source signatures and inferred receiver method closure.

use super::TypeChecker;
use crate::{Result, Type, TypeError};
use verum_ast::{Span, decl::ImplItemKind};
use verum_common::{List, Set, Shared, Text};
use verum_modules::{ModuleInfo, ModulePath, visibility::VisibilityChecker};

/// An immutable declaration location, not a memoized lookup result. Registered
/// modules and active lexical scopes already own their syntax: retain that exact
/// snapshot until registration finishes instead of copying its complete body.
#[derive(Clone, Debug)]
pub(super) enum SourceTypeDeclaration {
    Registered { module: Shared<ModuleInfo>, item: usize },
    Lexical { module: Shared<verum_ast::Module>, item: usize },
    Owned(Shared<verum_ast::TypeDecl>),
}

impl std::ops::Deref for SourceTypeDeclaration {
    type Target = verum_ast::TypeDecl;

    fn deref(&self) -> &Self::Target {
        let (items, index) = match self {
            Self::Registered { module, item } => (&module.ast.items, *item),
            Self::Lexical { module, item } => (&module.items, *item),
            Self::Owned(declaration) => return declaration,
        };
        let verum_ast::ItemKind::Type(declaration) = &items[index].kind else {
            unreachable!("declaration locations select a type in an immutable snapshot")
        };
        declaration
    }
}

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
pub(super) enum MountTargetKind {
    Named,
    Glob,
}

/// Visit explicitly named targets and the glob surfaces that could export a spelling.
pub(super) fn source_mount_targets(
    mount: &verum_ast::MountDecl,
    owner: &ModulePath,
    name: &str,
    mut visit: impl FnMut(ModulePath, MountTargetKind),
) {
    use verum_ast::{decl::MountTreeKind, ty::PathSegment};
    use verum_modules::{
        imports::path_to_module_path_resolved,
        mount_paths::{mount_paths, MountPathKind},
    };
    let root_alias = match &mount.tree.kind {
        MountTreeKind::Path(_) => mount.alias.as_ref(),
        _ => None,
    };
    let leaves = mount_paths(&mount.tree).filter_leaves(|leaf, kind, alias| {
        if kind == MountPathKind::Glob {
            return true;
        }
        // A grouped self leaf selects a module, never an individual type or callable.
        matches!(leaf.segments.last(), Some(PathSegment::Name(original))
            if alias.or(root_alias).unwrap_or(original).name == name)
    });
    for leaf in leaves {
        let kind = match leaf.kind {
            MountPathKind::Named => MountTargetKind::Named,
            MountPathKind::Glob => MountTargetKind::Glob,
        };
        visit(path_to_module_path_resolved(&leaf.path, owner), kind);
    }
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
    pub(super) fn local_source_type_declaration<'a>(
        &self,
        ast: &'a verum_ast::Module,
        name: &str,
    ) -> Option<&'a verum_ast::TypeDecl> {
        let index = self.local_source_type_declaration_index(ast, name)?;
        let verum_ast::ItemKind::Type(declaration) = &ast.items[index].kind else {
            unreachable!("type declaration index")
        };
        Some(declaration)
    }

    pub(super) fn local_source_type_declaration_index(
        &self, ast: &verum_ast::Module, name: &str,
    ) -> Option<usize> {
        ast.items.iter().position(|item| {
            self.cfg_evaluator.should_include_item(item)
                && matches!(&item.kind, verum_ast::ItemKind::Type(declaration)
                    if declaration.name.name.as_str() == name)
        })
    }

    /// A signature from a different source module resolves nominal declarations
    /// and private mounts through its defining file, in the caller's registry.
    pub fn with_source_signature_scope<T>(
        &mut self,
        owner: &str,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        // A single nullary variant can also spell an alias. Its defining
        // module supplies the type names visible for that decision.
        let aliases = self.source_type_registration.local_declarations.iter().rev()
            .find(|(path, _)| path.as_str() == owner)
            .map(|(_, module)| Self::compute_alias_scope(&module.items))
            .or_else(|| self.inline_modules.get(&Text::from(owner))?.items.as_ref()
                .map(|items| Self::compute_alias_scope(items)))
            .or_else(|| {
                let registry = self.module_registry.read();
                let module = self.get_module_with_path_aliases(owner, &registry)?;
                Some(Self::compute_alias_scope(&module.ast.items))
            });
        let previous_owner = std::mem::replace(&mut self.current_module_path, owner.into());
        let previous_scope = std::mem::replace(&mut self.resolving_source_signature, true);
        let previous_aliases = std::mem::replace(&mut self.alias_scope, aliases);
        let result = operation(self);
        self.alias_scope = previous_aliases;
        self.resolving_source_signature = previous_scope;
        self.current_module_path = previous_owner;
        result
    }

    /// A declaration's integer arguments and array lengths use constants from
    /// its source file. Inline declarations retain the file's lexical parents.
    pub(super) fn checked_integer_in_scope(&self, expr: &verum_ast::Expr) -> crate::const_eval::Result<i64> {
        let owner = self.current_module_path.as_str();
        if !self.resolving_source_signature {
            return self.const_eval.eval_array_count_in_source(
                expr, owner, true, &self.ctx.env.unavailable_constant_names(),
            );
        }
        crate::const_eval::ConstEvaluator::eval_array_count_in_declaration(expr, owner, || {
            let from_items = |items: &[verum_ast::Item], file_owner: &str| {
                let mut constants = crate::const_eval::ConstEvaluator::new();
                constants.declare_count_file_selected(items, file_owner, &self.cfg_evaluator);
                constants
            };
            let mut path = Some(owner);
            while let Some(candidate) = path {
                if let Some((_, file)) = self.source_type_registration.local_declarations.iter().rev()
                    .find(|(name, file)| name.as_str() == candidate && file.file_id == expr.span.file_id)
                {
                    return Some(from_items(&file.items, candidate));
                }
                let registered = self.get_module_with_path_aliases(candidate, &self.module_registry.read());
                if let Some(file) = registered
                    && file.ast.file_id == expr.span.file_id && !file.ast.items.is_empty()
                {
                    return Some(from_items(&file.ast.items, candidate));
                }
                path = candidate.rsplit_once('.').map(|(parent, _)| parent);
            }
            // Standalone inline modules can be prepared without a registry.
            self.inline_modules.get(&self.current_module_path)
                .filter(|module| module.span.file_id == expr.span.file_id)
                .and_then(|module| module.items.as_deref())
                .map(|items| from_items(items, owner))
        })
    }

    /// Elaborate source-library declarations under their owning module. Only
    /// names visible from this consumer may also enter its lexical type scope.
    pub fn prepare_source_library_types(
        &mut self,
        modules: &[(Text, std::sync::Arc<verum_ast::Module>)],
    ) {
        let consumer = self.current_module_path.clone();
        let previous_aliases = self.alias_scope.take();
        let mut declarations = List::new();
        for (owner, module) in modules {
            self.set_current_module_path(owner.clone());
            let mut items = List::new();
            for item in &module.items {
                let verum_ast::ItemKind::Type(declaration) = &item.kind else {
                    continue;
                };
                if !self.cfg_evaluator.should_include_item(item) {
                    continue;
                }
                let visible = declaration.visibility.resolve_declared_scope(owner).is_ok_and(|scope| {
                    scope.visible_from(owner.as_str().split('.'), consumer.as_str().split('.'))
                });
                if visible {
                    self.register_type_name_only(declaration);
                }
                items.push((declaration, visible));
            }
            declarations.push((owner, module, items));
        }
        let mut resolving = List::new();
        for (owner, module, items) in declarations {
            self.set_current_module_path(owner.clone());
            self.set_alias_scope_from_items(&module.items);
            for (declaration, visible) in items {
                let result = if visible {
                    self.resolve_type_definition(declaration, &mut resolving)
                } else {
                    self.load_source_type_declaration_with_parameter_key(declaration, owner, declaration.span)
                        .map(|_| ())
                };
                if let Err(error) = result {
                    tracing::debug!("Source type preparation for {}.{}: {error}", owner, declaration.name.name);
                }
            }
        }
        self.alias_scope = previous_aliases;
        self.set_current_module_path(consumer);
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

        if let Some(ty @ (Type::Var(_) | Type::TypeConstructor { .. })) = self.ctx.lookup_type(name) {
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
        } else {
            if name.contains('.')
                && let Some((ty, key)) = self.resolve_qualified_source_type_with_parameter_key(&Text::from(name), span)?
            {
                return Ok((ty, Some(key)));
            }
            if let Some((ty, key)) = self.load_declaring_archive_type(name, span)? {
                return Ok((ty, Some(key)));
            }
            // Ambient language declarations are the last scope. An explicit
            // source mount or retained binding owns its name even on refusal.
            if let Some(ty) = self.ctx.builtin_type_definition(name) {
                return Ok((ty.clone(), None));
            }
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
                    if !self.cfg_evaluator.should_include_item(item) {
                        continue;
                    }
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
        let descriptor = Self::metadata_type_with_declaring_key(metadata, &key)
            .ok_or_else(|| TypeError::TypeNotFound { name: path.clone(), span })?;
        if !descriptor.is_public {
            return Err(TypeError::VisibilityError {
                name: declared, visibility: "Private".into(), module_path: owner, span,
            });
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
    ) -> Option<(SourceTypeDeclaration, Text)> {
        for (owner, module) in self.source_type_registration.local_declarations.iter().rev() {
            if owner == &self.current_module_path {
                if let Some(item) = self.local_source_type_declaration_index(module, name) {
                    return Some((SourceTypeDeclaration::Lexical { module: module.clone(), item }, owner.clone()));
                }
            }
        }
        // Inline and file modules own their local declarations equally. The
        // registry contains file modules; inline bodies live in this source
        // index and must not fall through to the consumer's flat bindings.
        if let Some(module) = self.inline_modules.get(&self.current_module_path)
            && let Some(items) = &module.items
            && let Some(declaration) = items.iter()
                .filter(|item| self.cfg_evaluator.should_include_item(item))
                .find_map(|item| match &item.kind {
                verum_ast::ItemKind::Type(declaration) if declaration.name.name == name => Some(declaration),
                _ => None,
            })
        {
            return Some((SourceTypeDeclaration::Owned(Shared::new(declaration.clone())), self.current_module_path.clone()));
        }
        let registry = self.module_registry.read();
        let Some(module) = self.get_module_with_path_aliases(self.current_module_path.as_str(), &registry) else {
            // Low-level declaration APIs may have no file registry. Only an
            // exact active declaration or successfully committed source owns
            // this name. Recursive bounds consult their current transaction.
            let key = self.declared_type_key(name);
            return self.source_type_registration.declaration(&key)
                .map(|declaration| (SourceTypeDeclaration::Owned(Shared::new(declaration.clone())), self.current_module_path.clone()));
        };
        let declared = self.find_type_declaration_with_source_module_inner(
            &module.ast, name, &Text::from(module.path.to_string()),
            &registry, &mut Set::new(), true, true,
        );
        if declared.is_some() {
            return declared;
        }
        // Written bindings remain authoritative even when their target is
        // missing or inaccessible; the prelude cannot repair an invalid mount.
        let explicitly_bound = module.ast.items.iter()
            .filter(|item| self.cfg_evaluator.should_include_item(item))
            .any(|item| matches!(
            &item.kind, verum_ast::ItemKind::Mount(mount)
                if mount.alias.as_ref().is_some_and(|alias| alias.name.as_str() == name)
                || source_mount_binds_name(&mount.tree, name)
        ));
        if explicitly_bound { return None; }
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
        let Some((_, type_name)) = qualified.rsplit_once('.') else { return Ok(None) };
        let Some((module_owner, source_declaration)) = self.qualified_source_declaration(qualified, span)? else {
            return Ok(None)
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

    pub(super) fn qualified_source_declaration(
        &self, qualified: &Text, span: Span,
    ) -> Result<Option<(Text, Option<(SourceTypeDeclaration, Text)>)>> {
        let Some((module_name, type_name)) = qualified.rsplit_once('.') else { return Ok(None) };
        {
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
            let declaration = if let Some(item) = self.local_source_type_declaration_index(&module.ast, type_name) {
                let declaration = SourceTypeDeclaration::Registered { module: module.clone(), item };
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
            if declaration.is_none() && module.ast.items.iter()
                .filter(|item| self.cfg_evaluator.should_include_item(item))
                .any(|item| {
                matches!(&item.kind, verum_ast::ItemKind::Mount(mount)
                    if source_mount_binds_name(&mount.tree, type_name))
            }) {
                // Private, missing and cyclic source routes do not authorize
                // substituting an unrelated archived declaration of this name.
                return Err(TypeError::TypeNotFound { name: qualified.clone(), span });
            }
            Ok(Some((owner, declaration)))
        }
    }

    pub(super) fn load_source_type_declaration_with_parameter_key(
        &mut self,
        declaration: &verum_ast::TypeDecl,
        owner: &Text,
        span: Span,
    ) -> Result<(Type, Text)> {
        let key = Self::source_type_declaration_key(owner, &declaration.name.name);
        if let Some(forward) = self.source_type_registration.forward_payloads.get(&key)
            && &**forward == declaration
            && !self.source_type_registration.staged_heads.contains_key(&key)
            && !self.source_type_registration.is_completed(&key)
        {
            return Ok((Type::Named { path: Self::text_to_path(&key), args: Default::default() }, key));
        }
        // A nominal entry may only be a placeholder left by failed source
        // elaboration. Completed loads, and recursive loads in this active
        // transaction, are the only authority for a cache hit.
        if let Some(accepted) = self.source_type_registration.declaration(&key)
        {
            if accepted != declaration {
                let message: Text = format!("conflicting source declarations while loading {key}").into();
                if self.source_type_registration.depth != 0 {
                    self.source_type_registration.failure = Some(message.clone());
                }
                return Err(TypeError::UniverseBinding { message, span });
            }
            if !self.source_type_registration.pending.contains_key(&key) {
                self.require_source_dependencies(&key, span)?;
            }
            let result = self.ctx.lookup_type(key.as_str()).cloned()
                .map(|ty| (ty, key.clone()))
                .ok_or_else(|| TypeError::TypeNotFound { name: key, span });
            if self.source_type_registration.depth != 0 && let Err(error) = &result {
                self.source_type_registration.failure = Some(error.to_string().into());
            }
            return result;
        }
        self.with_source_type_registration(declaration, key.clone(), |checker| {
            checker.register_source_payloads(declaration, owner, &key, span)
        })
    }

    pub(super) fn register_source_declaration_in_scope(
        &mut self, declaration: &verum_ast::TypeDecl, owner: &Text, key: &Text, span: Span,
    ) -> Result<(Type, Text)> {
        // A return type load is not a mount of its local spelling. Retain
        // declaration metadata, restoring the consumer binding on both exits.
        let local_name = declaration.name.name.clone();
        let previous_type = self.ctx.type_defs.get(&local_name).cloned();
        let previous_alias = self.ctx.type_aliases.get(&local_name).cloned();
        let previous_unifier_alias = self.unifier.alias_binding(local_name.as_str());
        self.ctx.enter_scope();
        let registration = self.register_type_declaration_in_module(declaration, owner.as_str());
        self.ctx.exit_scope();
        match previous_type {
            Some(ty) => { self.ctx.type_defs.insert(local_name.clone(), ty); }
            None => { self.ctx.type_defs.remove(&local_name); }
        }
        match previous_alias {
            Some(ty) => { self.ctx.type_aliases.insert(local_name.clone(), ty); }
            None => { self.ctx.type_aliases.remove(&local_name); }
        }
        self.unifier.restore_alias_binding(local_name, previous_unifier_alias);
        registration.and_then(|()| {
            self.ctx.lookup_type(key.as_str()).cloned()
                .map(|ty| (ty, key.clone()))
                .ok_or_else(|| TypeError::TypeNotFound { name: key.clone(), span })
        })
    }

    /// Resolve only a nominal key whose exact source module declares the type.
    /// An umbrella spelling, unknown owner, or same-leaf sibling is not proof.
    pub(super) fn source_receiver_declaration(
        &self,
        receiver: &Type,
    ) -> Option<(Shared<ModuleInfo>, SourceTypeDeclaration)> {
        let key = self.get_type_name(receiver)?;
        let (owner, name) = key.rsplit_once('.')?;
        let registry = self.module_registry.read();
        let module = self.get_module_with_path_aliases(owner, &registry)?;
        if module.path.to_string() != owner {
            return None;
        }
        let item = self.local_source_type_declaration_index(&module.ast, name)?;
        let declaration = SourceTypeDeclaration::Registered { module: module.clone(), item };
        Some((module, declaration))
    }

    /// Load the required protocol from the receiver's declaration sources
    /// before asking the immutable proof engine for evidence. An
    /// in-progress module import is not evidence, and unrelated implementations
    /// must not be checked first merely because they appear earlier in source.
    pub(super) fn load_source_bound_implementations(
        &mut self,
        receiver: &Type,
        bound: &crate::protocol::ProtocolBound,
        assumptions: &[crate::protocol::WhereClause],
        callables: &verum_common::Map<crate::ty::TypeVar, Type>,
        span: Span,
    ) -> Result<()> {
        use verum_common::well_known_types::{WellKnownType, type_names};

        let protocol = self.path_to_string(&bound.protocol);
        let mut owners = Vec::new();
        let (key_name, names) = if let Some((module, declaration)) =
            self.source_receiver_declaration(receiver)
        {
            let name = declaration.name.name.clone();
            let key = Self::source_type_declaration_key(module.path.to_string().as_str(), &name);
            owners.push(module);
            (key, vec![name])
        } else if let Some(name) = self.protocol_checker.read().builtin_source_receiver_name(receiver)
        {
            // A language scalar has no nominal TypeDecl to mount. Only source
            // declarations can supply implementations, including those in the
            // protocol owner, the scalar library and the value's own module.
            let scalar_module = WellKnownType::Int.canonical_archive_modules().first().copied();
            let protocol_module = protocol.rsplit_once('.').map(|(owner, _)| owner);
            let value_module = WellKnownType::from_name(&name)
                .and_then(|kind| kind.canonical_archive_modules().first().copied());
            let registry = self.module_registry.read();
            for owner in [protocol_module, scalar_module, value_module].into_iter().flatten() {
                if let Some(module) = registry.get_by_path(owner)
                    && module.path.to_string() == owner
                    && !owners.iter().any(|prior| prior.id == module.id)
                {
                    owners.push(module);
                }
            }
            let spellings = type_names::primitive_identity_spellings(&name);
            let names = if spellings.is_empty() {
                vec![name.clone()]
            } else {
                spellings.iter().map(|spelling| Text::from(*spelling)).collect()
            };
            (name, names)
        } else {
            return Ok(());
        };
        if owners.is_empty() {
            return Ok(());
        }
        let key = (key_name, protocol.clone());
        if !self.source_type_registration.active_impl_bounds.insert(key.clone()) {
            // An active declaration is not evidence. A previously published
            // implementation or an explicit caller assumption may still prove
            // this exact instantiated goal independently of that declaration.
            return self.protocol_checker.read()
                .check_universal_bounds_in(receiver, std::slice::from_ref(bound),
                    self.source_type_registration.proof_environment(assumptions, callables))
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
        let callables = std::mem::take(&mut self.source_type_registration.callable_assumptions);
        let result = owners.iter().try_for_each(|module| {
            let owner = module.path.to_string();
            names.iter().try_for_each(|name| {
                self.import_impl_blocks_for_type_in_module_selected(
                    &module.ast, name.as_str(), owner.as_str(), Some(&protocol),
                )
            })
        });
        self.decl_param_frames = frames;
        self.source_type_registration.projection_assumptions = assumptions;
        self.source_type_registration.callable_assumptions = callables;
        self.source_type_registration.active_impl_bounds.remove(&key);
        result
    }

    pub(super) fn load_source_receiver_methods(&mut self, receiver: &Type) -> Result<()> {
        let Some((module, declaration)) = self.source_receiver_declaration(receiver) else {
            return Ok(());
        };
        let name = declaration.name.name.clone();
        let owner = module.path.to_string();
        let key: Text = format!("{owner}.{name}").into();
        if !self.loaded_source_receiver_methods.insert(key.clone()) {
            return Ok(());
        }
        let result = self.import_impl_blocks_for_type_in_module(
            &module.ast,
            name.as_str(),
            owner.as_str(),
        );
        if result.is_err() {
            self.loaded_source_receiver_methods.remove(&key);
        }
        result
    }

    pub(super) fn load_source_index_methods(&mut self, receiver: &Type) -> Result<()> {
        if self.protocol_checker.read().resolve_index_protocol(receiver).is_none() {
            self.load_source_receiver_methods(receiver)?;
        }
        Ok(())
    }

    /// A source roster without core may declare the operator in its own
    /// lexical scope. An unrelated registered protocol with the same leaf
    /// supplies no such binding.
    pub(super) fn source_operator_name(
        &self,
        role: verum_common::well_known_types::WellKnownProtocol,
    ) -> Option<Text> {
        let canonical = crate::protocol::ProtocolChecker::operator_protocol_source_name(role)?;
        let (owner, name) = canonical.rsplit_once('.')?;
        if self.protocol_checker.read().has_protocol(&canonical)
            || self.module_registry.read().get_by_path(owner)
                .is_some_and(|module| self.local_source_type_declaration(&module.ast, name).is_some())
        {
            return Some(canonical);
        }
        if let Some((declaration, owner)) = self.declaring_source_type_declaration(role.as_str())
            && matches!(declaration.body, verum_ast::TypeDeclBody::Protocol(_))
        {
            return Some(Self::source_type_declaration_key(owner.as_str(), declaration.name.name.as_str()));
        }
        Some(canonical)
    }

    pub(super) fn load_source_operator_implementation(
        &mut self,
        receiver: &Type,
        role: verum_common::well_known_types::WellKnownProtocol,
        span: Span,
    ) -> Result<()> {
        use crate::protocol::ProtocolBound;
        if let Some(name) = self.source_operator_name(role) {
            let bound = ProtocolBound::simple(Self::text_to_path(&name));
            self.load_source_bound_implementations(receiver, &bound, &[], &Default::default(), span)?;
        }
        Ok(())
    }

    pub(super) fn resolve_source_future(
        &mut self,
        receiver: &Type,
        span: Span,
    ) -> Result<Option<crate::protocol::FutureResolution>> {
        use verum_common::well_known_types::WellKnownProtocol;
        if let Some(resolution) = self.protocol_checker.read().resolve_future_protocol(receiver) {
            return Ok(Some(resolution));
        }
        self.load_source_operator_implementation(receiver, WellKnownProtocol::Future, span)?;
        Ok(self.protocol_checker.read().resolve_future_protocol(receiver))
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
        let name = declaration.name.name.clone();
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

#[cfg(test)]
#[path = "../../tests/unit/source_declarations.rs"]
mod tests;
