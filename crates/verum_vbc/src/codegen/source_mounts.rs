//! Bind qualified callable mounts to declared functions, constructors and public re-exports.
use super::{FunctionInfo, VbcCodegen};
use std::collections::{BTreeSet, HashMap, HashSet};
use verum_ast::{Item, ItemKind, MountTree, MountTreeKind};

pub(super) fn mount_path_parts(written: &str) -> Vec<String> {
    if let Some(tail) = written.strip_prefix("..") {
        std::iter::once(".".to_owned())
            .chain(tail.split('.').map(str::to_owned))
            .collect()
    } else {
        written.split('.').map(str::to_owned).collect()
    }
}

pub(super) fn visit_named_mounts(
    tree: &MountTree,
    prefix: &str,
    outer_alias: Option<&verum_ast::ty::Ident>,
    visit: &mut impl FnMut(&str, String),
) {
    let path = |path: &verum_ast::Path| {
        let path = path.to_string().replace("::", ".");
        if prefix.is_empty() {
            path
        } else {
            format!("{prefix}.{path}")
        }
    };
    match &tree.kind {
        MountTreeKind::Path(target) => {
            let full = path(target);
            let alias = tree
                .alias
                .as_ref()
                .or(outer_alias)
                .map(|id| id.name.as_str())
                .unwrap_or_else(|| full.rsplit('.').next().unwrap());
            let alias = alias.to_owned();
            visit(&alias, full);
        }
        MountTreeKind::Nested { prefix, trees } => {
            let prefix = path(prefix);
            for tree in trees {
                visit_named_mounts(tree, &prefix, None, visit);
            }
        }
        MountTreeKind::Glob(_) | MountTreeKind::File { .. } => {}
    }
}

/// Source declaration owners and public re-export edges. The index contains
/// no module-local function or type IDs and can be shared by other VBC units.
#[derive(Default)]
pub struct SourceExports {
    pub(super) ambiguous_variants: HashSet<String>,
    modules: HashMap<String, ModuleExports>,
}

#[derive(Default)]
struct ModuleExports {
    declarations: HashMap<String, BTreeSet<String>>,
    non_functions: HashSet<String>,
    visibility: HashMap<String, verum_ast::Visibility>,
    named: HashMap<String, BTreeSet<String>>,
    globs: BTreeSet<String>,
    lexical_globs: BTreeSet<String>,
    public_children: BTreeSet<String>,
    namespaces: HashSet<String>,
}

#[derive(Default)]
struct ExportCandidates {
    declarations: BTreeSet<String>,
    non_functions: HashSet<String>,
    inaccessible: bool,
}

impl SourceExports {
    pub fn mark_ambiguous_variant(&mut self, name: &str) {
        self.ambiguous_variants.insert(name.to_owned());
    }

    fn merge_module(&mut self, owner: String, exports: ModuleExports) {
        use std::collections::hash_map::Entry;
        let scope = match self.modules.entry(owner) {
            Entry::Vacant(entry) => { entry.insert(exports); return; }
            Entry::Occupied(entry) => entry.into_mut(),
        };
        for (name, targets) in exports.declarations {
            scope.declarations.entry(name).or_default().extend(targets);
        }
        scope.non_functions.extend(exports.non_functions);
        scope.visibility.extend(exports.visibility);
        for (alias, targets) in exports.named {
            scope.named.entry(alias).or_default().extend(targets);
        }
        scope.globs.extend(exports.globs);
        scope.lexical_globs.extend(exports.lexical_globs);
        scope.public_children.extend(exports.public_children);
        scope.namespaces.extend(exports.namespaces);
    }
}

impl ExportCandidates {
    fn nominal(&self) -> Option<&String> {
        let name = self.declarations.first()?;
        (self.declarations.len() == 1 && self.non_functions.contains(name)).then_some(name)
    }
}

impl VbcCodegen {
    /// Publish the completed declaration pass without copying bodies or
    /// interpreting function IDs from another compilation unit.
    pub fn publish_source_exports(&mut self, index: &mut SourceExports) {
        let unit = std::mem::take(&mut self.source_exports);
        for (owner, exports) in unit.modules {
            index.merge_module(owner, exports);
        }
    }

    /// Constructors are inline values. Their source-owned names, tags and
    /// payloads are transferable even when the defining unit has no bodies yet.
    pub fn declared_variant_constructors(&self) -> std::collections::BTreeMap<&str, &FunctionInfo> {
        self.source_exports.modules.values()
            .flat_map(|module| module.declarations.values().flatten())
            .filter_map(|key| {
                let info = self.ctx.functions.get(key.as_str())?;
                info.variant_tag.map(|_| (key.as_str(), info))
            })
            .collect()
    }

    /// Resolve external facades through their original source declarations.
    pub fn set_source_exports(&mut self, index: std::sync::Arc<SourceExports>) {
        self.dependency_source_exports = Some(index);
    }

    fn source_export_scope(&self, owner: &str) -> Option<&ModuleExports> {
        self.source_exports.modules.get(owner).or_else(|| {
            self.dependency_source_exports.as_ref()?.modules.get(owner)
        })
    }

    /// A namespace binding requires its declaration in the parent and the loaded child scope.
    pub(super) fn declared_namespace_mount(&self, path: &str, consumer: &str) -> super::CodegenResult<bool> {
        let Some((owner, name)) = path.rsplit_once('.') else { return Ok(false) };
        let Some(parent) = self.source_export_scope(owner) else { return Ok(false) };
        if !parent.namespaces.contains(name) || self.source_export_scope(path).is_none() {
            return Ok(false);
        }
        let visible = parent.visibility.get(name).is_some_and(|policy| {
            policy.resolve_declared_scope(owner).is_ok_and(|policy| {
                policy.visible_from(owner.split('.'), consumer.split('.'))
            })
        });
        if !visible {
            return Err(super::CodegenError::new(super::CodegenErrorKind::InvalidSourceMount(vec![
                format!("module `{path}` is inaccessible from `{consumer}`"),
            ])));
        }
        Ok(true)
    }

    pub(super) fn collect_callable_exports(&mut self, owner: &str, items: &[Item]) {
        let mut exports = ModuleExports::default();
        for item in items {
            if !self.should_compile_item(item) {
                continue;
            }
            let mut non_function = |name: &verum_ast::Ident, visibility: &verum_ast::Visibility| {
                exports.non_functions.insert(name.name.to_string());
                exports
                    .visibility
                    .insert(name.name.to_string(), visibility.clone());
            };
            match &item.kind {
                ItemKind::Function(declaration) => {
                    let name = declaration.name.name.as_str();
                    exports
                        .visibility
                        .insert(name.to_owned(), declaration.visibility.clone());
                    exports
                        .declarations
                        .entry(name.to_owned())
                        .or_default()
                        .insert(format!("{owner}.{name}"));
                }
                ItemKind::Type(declaration) => {
                    non_function(&declaration.name, &declaration.visibility);
                }
                ItemKind::Const(declaration) => {
                    non_function(&declaration.name, &declaration.visibility);
                }
                ItemKind::Static(declaration) => {
                    non_function(&declaration.name, &declaration.visibility);
                }
                // These declarations can occupy a named import even when another
                // module has a function or constructor with the same spelling.
                ItemKind::Protocol(d) => non_function(&d.name, &d.visibility),
                ItemKind::Context(d) => non_function(&d.name, &d.visibility),
                ItemKind::ContextGroup(d) => non_function(&d.name, &d.visibility),
                ItemKind::Meta(d) => non_function(&d.name, &d.visibility),
                ItemKind::Predicate(d) => non_function(&d.name, &d.visibility),
                ItemKind::Theorem(d) | ItemKind::Lemma(d) | ItemKind::Corollary(d) => {
                    non_function(&d.name, &d.visibility);
                }
                ItemKind::Axiom(d) => non_function(&d.name, &d.visibility),
                ItemKind::Mount(mount) => {
                    for parts in Self::extract_glob_mount_paths(&mount.tree, &[]) {
                        let path = Self::resolve_module_path_head_in_scope(&parts, owner)
                            .unwrap_or(parts)
                            .join(".");
                        if mount.visibility.is_public() {
                            exports.globs.insert(path.clone());
                        }
                        exports.lexical_globs.insert(path);
                    }
                    if !mount.visibility.is_public() {
                        continue;
                    }
                    visit_named_mounts(
                        &mount.tree,
                        "",
                        mount.alias.as_ref(),
                        &mut |alias, target| {
                            let parts = mount_path_parts(&target);
                            let target = Self::resolve_module_path_head_in_scope(&parts, owner)
                                .map(|parts| parts.join("."))
                                .unwrap_or(target);
                            exports
                                .named
                                .entry(alias.to_owned())
                                .or_default()
                                .insert(target);
                        },
                    );
                }
                ItemKind::Module(module) => {
                    let name = module.name.name.as_str();
                    let child = crate::module::qualify_module_name(owner, name);
                    if child != owner {
                        exports.namespaces.insert(name.to_owned());
                        exports.visibility.insert(name.to_owned(), module.visibility.clone());
                    }
                    if let Some(items) = &module.items {
                        self.collect_callable_exports(&child, items);
                        if module.visibility.is_public() {
                            exports.public_children.insert(child);
                        }
                    }
                }
                _ => {}
            }
        }
        self.source_callable_names
            .extend(exports.declarations.keys().cloned());
        self.source_exports.merge_module(owner.to_owned(), exports);
    }

    /// Constructors and foreign calls acquire a canonical callable identity
    /// during declaration registration, after target and ABI selection.
    pub(super) fn record_declared_callable(&mut self, owner: &str, name: &str, key: &str) {
        let Some(source) = self.source_exports.modules.get_mut(owner) else {
            return;
        };
        source
            .declarations
            .entry(name.to_owned())
            .or_default()
            .insert(key.to_owned());
        self.source_callable_names
            .insert(name.to_owned());
    }

    /// An implementation may precede its protocol body in bootstrap. Reserve
    /// the declared import target so its vtable row survives descriptor import.
    pub(super) fn implementation_protocol_id(
        &mut self,
        path: &verum_ast::Path,
        leaf: &str,
    ) -> super::CodegenResult<crate::types::TypeId> {
        use crate::types::{StringId, TypeDescriptor, TypeKind};
        let scope = self.lexical_type_scope();
        let written = self.declared_dyn_path(path);
        let key = Self::resolve_module_path_head_in_scope(&mount_path_parts(&written), scope)
            .map(|parts| parts.join("."))
            .unwrap_or(written);
        let selected = if key == leaf
            && let Some(glob) = self.source_glob_type_id(leaf, scope)
        {
            Some(glob.ok_or_else(|| {
                super::CodegenError::with_span(
                    super::CodegenErrorKind::TypeInference(format!(
                        "no unique visible protocol for `{leaf}`"
                    )),
                    path.span,
                )
            })?)
        } else {
            self.source_export_type_id(&key, scope)
                .or_else(|| self.type_name_to_id.get(&key).copied())
        };
        if let Some(id) = selected {
            if self
                .type_by_id(id)
                .is_some_and(|decl| decl.kind != TypeKind::Protocol)
            {
                return Err(super::CodegenError::with_span(
                    super::CodegenErrorKind::TypeInference(format!(
                        "implementation target `{key}` is not a protocol"
                    )),
                    path.span,
                ));
            }
            return Ok(id);
        }
        let (module, name) = key
            .rsplit_once('.')
            .map_or((None, key.as_str()), |(m, n)| (Some(m), n));
        let id = self.alloc_user_type_id();
        let name = StringId(self.ctx.intern_string_raw(name));
        let origin_module = module.map(|name| StringId(self.ctx.intern_string_raw(name)));
        self.push_type_descriptor(TypeDescriptor {
            id,
            name,
            kind: TypeKind::Protocol,
            origin_module,
            ..Default::default()
        });
        self.type_name_to_id.insert(key, id);
        Ok(id)
    }

    pub(super) fn source_callable_mount(
        &self,
        written: &str,
        alias: &str,
        scope: &str,
    ) -> Option<String> {
        let path = self
            .source_module_owners
            .member(written, scope, self.source_module_root())?;
        let (owner, name) = path.rsplit_once('.')?;
        let Some(source) = self.source_export_scope(owner) else {
            return self
                .type_name_to_id
                .contains_key(path.as_str())
                .then(|| path.into_string());
        };
        let named_callable = source.declarations.contains_key(name) || source.named.contains_key(name)
            || [name, alias].into_iter().any(|leaf| {
                self.source_callable_names.contains(leaf)
                    || self.ctx.functions.get(leaf).is_some_and(|info| {
                        info.variant_tag.is_some()
                            || info.has_source_body
                                && info.parent_type_name.is_none()
                                && !info.is_const
                                && info.param_names.first().is_none_or(|first| first != "self")
                    })
            });
        named_callable.then(|| path.into_string())
    }

    /// Follow public export edges once for both nominal IDs and callable bindings.
    /// Only selections which converge on one declaration identify a symbol.
    fn source_export_candidates(
        &self,
        path: &str,
        glob_consumer: Option<&str>,
    ) -> ExportCandidates {
        let root = path;
        let mut found = ExportCandidates::default();
        let mut pending = vec![(path.to_owned(), true)];
        let mut visited = HashSet::new();
        while let Some((path, includes_declarations)) = pending.pop() {
            if !visited.insert((path.clone(), includes_declarations)) {
                continue;
            }
            let Some((owner, name)) = path.rsplit_once('.') else {
                continue;
            };
            let Some(source) = self.source_export_scope(owner) else {
                // Imported nominal IDs and callable descriptors are independent
                // witnesses. A constructor elsewhere cannot turn a type into a call.
                if let Some(id) = self.type_name_to_id.get(&path) {
                    if let Some(consumer) = glob_consumer {
                        let visible = self.type_by_id(*id).is_some_and(|descriptor| {
                            use crate::types::Visibility;
                            match descriptor.visibility {
                                Visibility::Public => true,
                                Visibility::Cog => {
                                    path == root
                                        && owner.split('.').next() == consumer.split('.').next()
                                }
                                Visibility::Private => path == root && owner == consumer,
                            }
                        });
                        if !visible {
                            found.inaccessible = true;
                            continue;
                        }
                    }
                    found.non_functions.insert(path.clone());
                    found.declarations.insert(path);
                } else if self.ctx.functions.contains_key(&path) {
                    found.declarations.insert(path);
                }
                continue;
            };
            let anchored = |written: &str| {
                self.source_module_owners
                    .member(written, owner, self.source_module_root())
                    .map(|key| key.into_string())
                    .unwrap_or_else(|| written.to_owned())
            };
            if includes_declarations && let Some(visibility) = source.visibility.get(name) {
                // A direct wildcard uses the importer's visibility. Every
                // public re-export edge preserves the declaration's access.
                let visible = if path != root {
                    visibility.is_public()
                } else if let Some(consumer) = glob_consumer {
                    if matches!(visibility, verum_ast::Visibility::PublicIn(_)) {
                        visibility.resolve_declared_scope(owner).is_ok_and(|policy| policy.visible_from(owner.split('.'), consumer.split('.')))
                    } else {
                        visibility.visible_from(owner.split('.'), consumer.split('.'))
                    }
                } else {
                    true
                };
                if !visible {
                    found.inaccessible = true;
                    continue;
                }
            }
            if includes_declarations && let Some(targets) = source.declarations.get(name) {
                found.declarations.extend(targets.iter().cloned());
            } else if includes_declarations && source.namespaces.contains(name) {
                // An external declaration denotes a namespace only if its module
                // was actually loaded. It cannot borrow an unrelated callable.
                if self.source_export_scope(&path).is_some() {
                    found.non_functions.insert(path.clone());
                    found.declarations.insert(path);
                }
            } else if includes_declarations && source.non_functions.contains(name) {
                found.non_functions.insert(path.clone());
                found.declarations.insert(path);
            } else if let Some(targets) = source.named.get(name) {
                pending.extend(targets.iter().map(|target| (anchored(target), true)));
            } else {
                pending.extend(
                    source
                        .globs
                        .iter()
                        .map(|module| (anchored(&format!("{module}.{name}")), true)),
                );
                // Public inline modules expose their re-exports to the
                // enclosing facade. Their own declarations stay qualified.
                pending.extend(
                    source
                        .public_children
                        .iter()
                        .map(|child| (format!("{child}.{name}"), false)),
                );
            }
        }
        found
    }

    /// A lexical wildcard has lower precedence than a declaration or a named
    /// import. A populated but ambiguous export must block the bare-name path.
    pub(super) fn source_glob_type_id(
        &self,
        name: &str,
        scope: &str,
    ) -> Option<Option<crate::types::TypeId>> {
        if name.contains('.') {
            return None;
        }
        let roots = &self
            .source_exports
            .modules
            .get(scope)?
            .lexical_globs;
        let mut found = None;
        let mut inaccessible = false;
        for root in roots {
            let Some(path) = self.source_module_owners.member(
                &format!("{root}.{name}"),
                scope,
                self.source_module_root(),
            ) else {
                continue;
            };
            let candidates = self.source_export_candidates(&path, Some(scope));
            inaccessible |= candidates.inaccessible;
            if candidates.declarations.is_empty() {
                continue;
            }
            let Some(nominal) = candidates.nominal() else {
                return Some(None);
            };
            if found.as_ref().is_some_and(|other| other != nominal) {
                return Some(None);
            }
            found = Some(nominal.clone());
        }
        match found {
            Some(canonical) => Some(self.type_name_to_id.get(&canonical).copied()),
            None => inaccessible.then_some(None),
        }
    }

    pub(super) fn source_export_type_id(
        &self,
        path: &str,
        scope: &str,
    ) -> Option<crate::types::TypeId> {
        let path = self
            .source_module_owners
            .member(path, scope, self.source_module_root())?;
        let candidates = self.source_export_candidates(&path, None);
        self.type_name_to_id.get(candidates.nominal()?).copied()
    }

    pub(super) fn resolve_source_callable_export(
        &self,
        path: &str,
    ) -> Result<Option<(String, FunctionInfo)>, String> {
        let ExportCandidates {
            mut declarations,
            non_functions,
            ..
        } = self.source_export_candidates(path, None);
        if declarations.len() > 1 {
            return Err(format!(
                "ambiguous callable mount `{path}`: {}",
                declarations.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some(key) = declarations.pop_first() {
            if let Some(info) = self.ctx.lookup_function(&key) {
                return Ok(Some((key, info.clone())));
            }
            if non_functions.contains(&key) {
                return Ok(None);
            }
        }
        Err(format!("no callable declaration for mount `{path}`"))
    }
}
