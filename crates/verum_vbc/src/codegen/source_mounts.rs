//! Bind qualified value mounts to declared functions, constants, constructors and public re-exports.
use super::{FunctionInfo, VbcCodegen};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use verum_ast::{Item, ItemKind, MountTree, MountTreeKind};
use verum_common::Text;

pub(super) fn mount_path_parts(written: &str) -> Vec<String> {
    if let Some(tail) = written.strip_prefix("..") {
        std::iter::once(".".to_owned())
            .chain(tail.split('.').map(str::to_owned))
            .collect()
    } else {
        written.split('.').map(str::to_owned).collect()
    }
}

/// Grouped `self` selects the enclosing namespace. Both declaration discovery
/// and executable import registration must preserve that same selected path.
pub(super) fn named_mount_segments(path: &verum_ast::Path) -> &[verum_ast::PathSegment] {
    let segments = path.segments.as_slice();
    if segments.len() > 1 && matches!(segments.last(), Some(verum_ast::PathSegment::SelfValue)) {
        &segments[..segments.len() - 1]
    } else {
        segments
    }
}

pub(super) fn visit_named_mounts(
    tree: &MountTree,
    outer_alias: Option<&verum_ast::ty::Ident>,
    visit: &mut impl FnMut(&str, String),
) {
    use verum_ast::mount_paths::{MountPathKind, mount_paths};
    let outer_alias = matches!(tree.kind, MountTreeKind::Path(_)).then_some(outer_alias).flatten();
    for entry in mount_paths(tree).filter(|entry| entry.kind == MountPathKind::Named) {
        let selected_len = named_mount_segments(&entry.path).len();
        let mut target = entry.path;
        target.segments.truncate(selected_len);
        let full = target.to_string().replace("::", ".");
        let alias = entry.alias.or(outer_alias).map(|name| name.name.as_str())
            .unwrap_or_else(|| full.rsplit('.').next().unwrap());
        visit(&alias.to_owned(), full);
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
    // Only inline AST nesting establishes lexical ancestry. A file's dotted
    // namespace does not grant access to declarations in another file.
    lexical_parents: BTreeSet<String>,
    declarations: HashMap<String, Vec<DeclaredExport>>,
    named: HashMap<String, BTreeSet<WrittenMountPath>>,
    globs: BTreeSet<WrittenMountPath>,
    lexical_globs: BTreeSet<WrittenMountPath>,
    public_children: BTreeSet<String>,
}

/// Category and visibility belong to the same declaration. A public value
/// cannot change the visibility of a same-named private type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DeclarationCategory { Type, Callable, Value, Namespace }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum DeclarationNamespace { Type, Value, Module }

impl DeclarationCategory {
    fn namespace(self) -> DeclarationNamespace {
        match self {
            Self::Type => DeclarationNamespace::Type,
            Self::Callable | Self::Value => DeclarationNamespace::Value,
            Self::Namespace => DeclarationNamespace::Module,
        }
    }
}

#[derive(Clone, PartialEq)]
struct DeclaredExport {
    target: String,
    category: DeclarationCategory,
    visibility: verum_ast::Visibility,
}

impl ModuleExports {
    fn declare(&mut self, name: &str, target: String, category: DeclarationCategory,
        visibility: &verum_ast::Visibility) {
        let declaration = DeclaredExport { target, category, visibility: visibility.clone() };
        let entries = self.declarations.entry(name.to_owned()).or_default();
        if !entries.contains(&declaration) { entries.push(declaration); }
    }
}

/// Export discovery can precede namespace discovery. Keep the written edge
/// distinct from the canonical path queued by export traversal; selecting an
/// early partial roster would make merge order change the declaration owner.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct WrittenMountPath(verum_common::Text);

impl WrittenMountPath {
    fn select(&self, codegen: &VbcCodegen, owner: &str) -> verum_common::Text {
        codegen.select_source_mount_path(&self.0, owner)
    }
}

/// Retain every selected import until its declaration namespace is known.
/// A value import and a type import may legally share a local spelling.
#[derive(Clone)]
pub(super) enum SourceMountBinding {
    Targets(BTreeSet<Text>),
    Blocked,
}

impl From<Option<Text>> for SourceMountBinding {
    fn from(target: Option<Text>) -> Self {
        match target {
            Some(target) => Self::Targets(BTreeSet::from([target])),
            None => Self::Blocked,
        }
    }
}

#[derive(Default)]
struct ExportCandidates {
    declarations: BTreeMap<String, BTreeSet<DeclarationCategory>>,
    inaccessible: bool,
    unresolved: bool,
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
        for (name, declarations) in exports.declarations {
            for declaration in declarations {
                scope.declare(&name, declaration.target, declaration.category, &declaration.visibility);
            }
        }
        for (alias, targets) in exports.named {
            scope.named.entry(alias).or_default().extend(targets);
        }
        scope.globs.extend(exports.globs);
        scope.lexical_globs.extend(exports.lexical_globs);
        scope.public_children.extend(exports.public_children);
        scope.lexical_parents.extend(exports.lexical_parents);
    }
}

impl ExportCandidates {
    fn insert(&mut self, target: String, category: DeclarationCategory) {
        self.declarations.entry(target).or_default().insert(category);
    }

    fn nominal(&self) -> Option<&String> {
        if self.unresolved { return None; }
        let mut types = self.declarations.iter()
            .filter(|(_, kinds)| kinds.contains(&DeclarationCategory::Type));
        let first = types.next()?.0;
        types.next().is_none().then_some(first)
    }

    fn only_values(&self) -> bool {
        !self.inaccessible && !self.unresolved && !self.declarations.is_empty()
            && self.declarations.values().all(|kinds| kinds.iter()
                .all(|kind| matches!(kind, DeclarationCategory::Callable | DeclarationCategory::Value)))
    }
}

impl VbcCodegen {
    /// Source consumers share the AST's implicit-import policy. Publishing the
    /// private lexical edge avoids cloning complete modules solely to prepend
    /// the same mount that ordinary file compilation injects.
    pub(super) fn collect_module_exports(&mut self, owner: &str, module: &verum_ast::Module) {
        self.collect_callable_exports(owner, &module.items);
        if let Some(path) = verum_ast::prelude::implicit_prelude_path(module) {
            self.source_exports.modules.entry(owner.to_owned()).or_default()
                .lexical_globs.insert(WrittenMountPath(path.into()));
        }
    }

    /// Select a written import once. Explicit relative/cog paths and the
    /// source's root-qualified paths already identify their namespace; only
    /// an unqualified module head participates in lexical namespace lookup.
    pub(super) fn select_source_mount_path(&self, written: &str, owner: &str) -> verum_common::Text {
        let mut parts = mount_path_parts(written);
        // In a mount, `self` names the current namespace. Keep this selection
        // separate from expression paths where `self` is a value receiver.
        if parts.first().is_some_and(|head| head == "self") {
            parts[0] = ".".to_owned();
        }
        if let Some(selected) = Self::resolve_module_path_head_in_scope(&parts, owner) {
            return selected.join(".").into();
        }
        let root = owner.split('.').next().unwrap_or_default();
        if !root.is_empty() && parts.first().is_some_and(|head| head == root) {
            return written.into();
        }
        self.source_module_owners.namespace(written, owner, root)
            .or_else(|| self.source_module_owners.member(written, owner, root))
            .unwrap_or_else(|| written.into())
    }

    /// Import intent belongs to the declaring source, independently of item
    /// order. Capture paths before signatures, without binding callable IDs or
    /// requiring the imported descriptors to have been collected yet.
    pub(super) fn publish_source_type_mount(
        &mut self,
        owner: &str,
        mount: &verum_ast::MountDecl,
    ) {
        visit_named_mounts(&mount.tree, mount.alias.as_ref(), &mut |alias, written| {
            let target = self.select_source_mount_path(&written, owner);
            self.record_source_type_mount(owner, alias, &target);
        });
    }

    /// Record a selected canonical path, never an unresolved source spelling.
    /// Repeated declaration passes are idempotent. Distinct explicit targets
    /// remain ambiguous; a later item cannot silently replace their evidence.
    pub(super) fn record_source_type_mount(&mut self, owner: &str, alias: &str, target: &str) {
        let binding = self.source_type_mounts.entry(owner.into()).or_default()
            .entry(alias.into()).or_insert_with(|| SourceMountBinding::Targets(BTreeSet::new()));
        if let SourceMountBinding::Targets(targets) = binding {
            targets.insert(target.into());
        }
    }

    /// A known value declaration contributes no binding in type position.
    /// Unknown, inaccessible and conflicting imports retain their refusal.
    pub(super) fn select_type_mount<'a>(
        &self, binding: &'a SourceMountBinding, owner: &str,
    ) -> Option<Option<&'a Text>> {
        let SourceMountBinding::Targets(targets) = binding else { return Some(None) };
        let mut selected = None;
        for target in targets {
            if self.source_export_candidates(target, Some(owner)).only_values() { continue; }
            if selected.is_some() { return Some(None); }
            selected = Some(target);
        }
        selected.map(Some)
    }

    /// Archive readers receive the declaration's selected type environment,
    /// not import syntax that would need a different module graph to resolve.
    pub(super) fn declaration_source_type_bindings(
        &self,
        owner: &str,
    ) -> Option<verum_common::OrderedMap<Text, Option<Text>>> {
        let named = self.source_type_mounts.get(&Text::from(owner))?;
        let mut bindings: verum_common::OrderedMap<Text, Option<Text>> = named.iter()
            .filter_map(|(name, binding)| {
                let selected = self.select_type_mount(binding, owner)?;
                let canonical = selected.and_then(|target| {
                    let types = self.source_export_candidates_in(
                        target, Some(owner), Some(DeclarationNamespace::Type),
                    );
                    if types.inaccessible || types.unresolved { return None; }
                    if let Some(canonical) = types.nominal() { return Some(canonical.clone().into()); }
                    if !types.declarations.is_empty() { return None; }
                    let namespaces = self.source_export_candidates_in(
                        target, Some(owner), Some(DeclarationNamespace::Module),
                    );
                    if namespaces.inaccessible || namespaces.unresolved { return None; }
                    if namespaces.declarations.len() == 1 {
                        return namespaces.declarations.keys().next().cloned().map(Into::into);
                    }
                    if !namespaces.declarations.is_empty() { return None; }
                    // An unavailable declaration still has an exact selected
                    // import identity. Preserve it for completion/conflict
                    // checks; the reader must resolve it before using a type.
                    Some(target.clone())
                });
                Some((name.clone(), canonical))
            }).collect();

        let Some(exports) = self.source_export_scope(owner) else { return Some(bindings) };
        let mut pending: Vec<_> = exports.lexical_globs.iter()
            .map(|path| path.select(self, owner).into_string()).collect();
        let mut visited = HashSet::new();
        let mut candidates = BTreeSet::new();
        while let Some(path) = pending.pop() {
            if !visited.insert(path.clone()) { continue; }
            let Some(scope) = self.source_export_scope(&path) else { continue };
            candidates.extend(scope.declarations.iter().filter(|(_, declarations)|
                declarations.iter().any(|decl| decl.category == DeclarationCategory::Type))
                .map(|(name, _)| name.clone()));
            candidates.extend(scope.named.keys().cloned());
            pending.extend(scope.globs.iter().map(|edge| edge.select(self, &path).into_string()));
            pending.extend(scope.public_children.iter().cloned());
        }
        for name in candidates {
            if bindings.contains_key(&Text::from(name.as_str()))
                || self.type_name_to_id.contains_key(&format!("{owner}.{name}"))
            { continue; }
            if let Some(selected) = self.source_glob_type_path(&name, owner) {
                bindings.insert(name.into(), selected.map(Into::into));
            }
        }
        Some(bindings)
    }

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
            .filter_map(|declaration| {
                let key = declaration.target.as_str();
                let info = self.ctx.functions.get(key)?;
                info.variant_tag.map(|_| (key, info))
            })
            .collect()
    }

    /// Resolve external facades through their original source declarations.
    pub fn set_source_exports(&mut self, index: std::sync::Arc<SourceExports>) {
        for owner in index.modules.keys() {
            self.source_module_owners.declare(owner);
        }
        self.dependency_source_exports = Some(index);
    }

    fn source_export_scope(&self, owner: &str) -> Option<&ModuleExports> {
        self.source_exports.modules.get(owner).or_else(|| {
            self.dependency_source_exports.as_ref()?.modules.get(owner)
        })
    }

    pub(super) fn record_inline_type_parent(&mut self, child: &str, parent: &str) {
        self.source_exports.modules.entry(child.to_owned()).or_default()
            .lexical_parents.insert(parent.to_owned());
    }

    /// `Some(None)` refuses conflicting lexical ownership; it never selects an
    /// arbitrary parent from a merged declaration graph.
    pub(super) fn lexical_type_parent(&self, owner: &str) -> Option<Option<&str>> {
        let parents = &self.source_export_scope(owner)?.lexical_parents;
        if parents.is_empty() { return None; }
        Some((parents.len() == 1).then(|| parents.first().unwrap().as_str()))
    }

    /// A namespace binding requires its declaration in the parent and the loaded child scope.
    pub(super) fn declared_namespace_mount(&self, path: &str, consumer: &str) -> super::CodegenResult<bool> {
        let Some((owner, name)) = path.rsplit_once('.') else { return Ok(false) };
        let Some(parent) = self.source_export_scope(owner) else { return Ok(false) };
        let Some(declarations) = parent.declarations.get(name) else { return Ok(false) };
        let namespaces: Vec<_> = declarations.iter().filter(|declaration|
            declaration.category == DeclarationCategory::Namespace && declaration.target == path).collect();
        if namespaces.is_empty() || self.source_export_scope(path).is_none() { return Ok(false); }
        let visible = namespaces.iter().all(|declaration| {
            declaration.visibility.resolve_declared_scope(owner).is_ok_and(|policy| {
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
            let mut declare = |name: &verum_ast::Ident, visibility: &verum_ast::Visibility, category| {
                exports.declare(&name.name, crate::module::qualify_module_name(owner, &name.name), category, visibility);
            };
            use DeclarationCategory::{Callable, Namespace, Type, Value};
            match &item.kind {
                ItemKind::Function(d) => declare(&d.name, &d.visibility, Callable),
                ItemKind::ExternBlock(block) => {
                    for function in &block.functions {
                        if self.cfg_evaluator.should_include(&function.attributes) {
                            declare(&function.name, &function.visibility, Callable);
                        }
                    }
                }
                ItemKind::Type(d) => {
                    declare(&d.name, &d.visibility, Type);
                    if let verum_ast::TypeDeclBody::Variant(variants)
                        | verum_ast::TypeDeclBody::Inductive(variants) = &d.body
                    {
                        let parent = crate::module::qualify_module_name(owner, &d.name.name);
                        for variant in variants {
                            let name = variant.name.name.as_str();
                            exports.declare(name, format!("{parent}.{name}"), Callable, &d.visibility);
                        }
                    }
                }
                ItemKind::Protocol(d) => declare(&d.name, &d.visibility, Type),
                ItemKind::Const(d) => declare(&d.name, &d.visibility, Value),
                ItemKind::Static(d) => declare(&d.name, &d.visibility, Value),
                ItemKind::Context(d) => declare(&d.name, &d.visibility, Value),
                ItemKind::ContextGroup(d) => declare(&d.name, &d.visibility, Value),
                ItemKind::Meta(d) => declare(&d.name, &d.visibility, Value),
                ItemKind::Predicate(d) => declare(&d.name, &d.visibility, Value),
                ItemKind::Theorem(d) | ItemKind::Lemma(d) | ItemKind::Corollary(d) => {
                    declare(&d.name, &d.visibility, Value);
                }
                ItemKind::Axiom(d) => declare(&d.name, &d.visibility, Value),
                ItemKind::Mount(mount) => {
                    for parts in Self::extract_glob_mount_paths(&mount.tree) {
                        let path = WrittenMountPath(parts.join(".").into());
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
                        mount.alias.as_ref(),
                        &mut |alias, target| {
                            let target = WrittenMountPath(target.into());
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
                        exports.declare(name, child.clone(), Namespace, &module.visibility);
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
        self.source_callable_names.extend(exports.declarations.iter()
            .filter(|(_, declarations)| declarations.iter().any(|d| d.category == DeclarationCategory::Callable))
            .map(|(name, _)| name.clone()));
        self.source_exports.merge_module(owner.to_owned(), exports);
    }

    /// Constructors and foreign calls acquire a canonical callable identity
    /// during declaration registration, after target and ABI selection.
    pub(super) fn record_declared_callable(&mut self, owner: &str, name: &str, key: &str) {
        let Some(source) = self.source_exports.modules.get_mut(owner) else {
            return;
        };
        // Source discovery owns the declaration and its visibility; later
        // executable registration may only confirm that exact selected target.
        if source.declarations.get(name).is_some_and(|declarations| declarations.iter()
            .any(|d| d.target == key && d.category == DeclarationCategory::Callable))
        {
            self.source_callable_names.insert(name.to_owned());
        }
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

    pub(super) fn source_value_mount(
        &self,
        path: &str,
        alias: &str,
    ) -> Option<String> {
        let (owner, name) = path.rsplit_once('.')?;
        let Some(source) = self.source_export_scope(owner) else {
            return self
                .type_name_to_id
                .contains_key(path)
                .then(|| path.to_owned());
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
        named_callable.then(|| path.to_owned())
    }

    /// Classify reachable declarations without selecting a precedence tier.
    /// This finite, cycle-safe inventory distinguishes a valid other-namespace
    /// export from an unresolved named re-export. Selection still happens below.
    fn export_namespaces(&self, path: &str) -> BTreeSet<DeclarationNamespace> {
        let mut found = BTreeSet::new();
        let mut pending = vec![(path.to_owned(), true)];
        let mut visited = HashSet::new();
        while let Some((path, include_declarations)) = pending.pop() {
            if !visited.insert((path.clone(), include_declarations)) { continue; }
            let Some((owner, name)) = path.rsplit_once('.') else { continue };
            let Some(source) = self.source_export_scope(owner) else {
                if self.type_name_to_id.get(&path).and_then(|id| self.type_by_id(*id))
                    .and_then(|ty| self.type_declaration_identity(ty)).is_some_and(|key| key.as_str() == path)
                { found.insert(DeclarationNamespace::Type); }
                if self.ctx.functions.contains_key(&path) { found.insert(DeclarationNamespace::Value); }
                continue;
            };
            if include_declarations && let Some(declarations) = source.declarations.get(name) {
                found.extend(declarations.iter().filter(|declaration|
                    declaration.category != DeclarationCategory::Namespace
                        || self.source_export_scope(&declaration.target).is_some())
                    .map(|declaration| declaration.category.namespace()));
            }
            if let Some(targets) = source.named.get(name) {
                pending.extend(targets.iter().map(|target| (target.select(self, owner).into_string(), true)));
            }
            pending.extend(source.globs.iter()
                .map(|module| (format!("{}.{name}", module.select(self, owner)), true)));
            pending.extend(source.public_children.iter().map(|child| (format!("{child}.{name}"), false)));
        }
        found
    }

    fn source_export_candidates(&self, path: &str, consumer: Option<&str>) -> ExportCandidates {
        self.source_export_candidates_in(path, consumer, None)
    }

    /// Precedence is local to the requested namespace. A declared value cannot
    /// hide a type re-export, and a named value import cannot hide a type glob.
    /// The worklist retains cycle detection; its per-query category inventory
    /// cannot become a stale cross-unit resolution cache.
    fn source_export_candidates_in(
        &self, path: &str, consumer: Option<&str>, namespace: Option<DeclarationNamespace>,
    ) -> ExportCandidates {
        let root = path;
        let mut found = ExportCandidates::default();
        let mut pending: Vec<_> = [DeclarationNamespace::Type, DeclarationNamespace::Value, DeclarationNamespace::Module]
            .into_iter().filter(|candidate| namespace.is_none_or(|ns| ns == *candidate))
            .map(|ns| (path.to_owned(), true, ns)).collect();
        let mut visited = HashSet::new();
        let mut categories = HashMap::<String, BTreeSet<DeclarationNamespace>>::new();
        while let Some((path, include_declarations, namespace)) = pending.pop() {
            if !visited.insert((path.clone(), include_declarations, namespace)) { continue; }
            let Some((owner, name)) = path.rsplit_once('.') else { continue };
            let Some(source) = self.source_export_scope(owner) else {
                if namespace == DeclarationNamespace::Type
                    && let Some(id) = self.type_name_to_id.get(&path)
                    && let Some(descriptor) = self.type_by_id(*id)
                    && self.type_declaration_identity(descriptor).is_some_and(|identity| identity.as_str() == path)
                {
                    if let Some(consumer) = consumer {
                        use crate::types::Visibility;
                        let visible = match descriptor.visibility {
                            Visibility::Public => true,
                            Visibility::Cog => path == root && owner.split('.').next() == consumer.split('.').next(),
                            Visibility::Private => path == root && owner == consumer,
                        };
                        if !visible { found.inaccessible = true; continue; }
                    }
                    found.insert(path, DeclarationCategory::Type);
                } else if namespace == DeclarationNamespace::Value && self.ctx.functions.contains_key(&path) {
                    found.insert(path, DeclarationCategory::Callable);
                }
                continue;
            };
            let declarations: Vec<_> = source.declarations.get(name).into_iter().flatten()
                .filter(|declaration| include_declarations && declaration.category.namespace() == namespace).collect();
            if !declarations.is_empty() {
                for declaration in declarations {
                    let visible = if path != root {
                        declaration.visibility.is_public()
                    } else if let Some(consumer) = consumer {
                        declaration.visibility.resolve_declared_scope(owner).is_ok_and(|policy|
                            policy.visible_from(owner.split('.'), consumer.split('.')))
                    } else { true };
                    if !visible { found.inaccessible = true; continue; }
                    if declaration.category == DeclarationCategory::Namespace
                        && self.source_export_scope(&declaration.target).is_none()
                    { continue; }
                    found.insert(declaration.target.clone(), declaration.category);
                }
                continue;
            }
            let mut named_selected = false;
            if let Some(targets) = source.named.get(name) {
                for target in targets {
                    let target = target.select(self, owner).into_string();
                    let exported = categories.entry(target.clone()).or_insert_with(|| self.export_namespaces(&target));
                    if exported.contains(&namespace) {
                        pending.push((target, true, namespace));
                        named_selected = true;
                    } else if exported.is_empty() {
                        // An unresolved explicit target is not evidence that
                        // it occupies some different namespace.
                        found.unresolved = true;
                        named_selected = true;
                    }
                }
            }
            if named_selected { continue; }
            pending.extend(source.globs.iter()
                .map(|module| (format!("{}.{name}", module.select(self, owner)), true, namespace)));
            pending.extend(source.public_children.iter().map(|child| (format!("{child}.{name}"), false, namespace)));
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
        self.source_glob_type_path(name, scope)
            .map(|selected| selected.and_then(|canonical| self.type_name_to_id.get(&canonical).copied()))
    }

    /// Preserve the selected declaration path before lowering it to a local
    /// TypeId; archives need this identity independently of descriptor slots.
    fn source_glob_type_path(&self, name: &str, scope: &str) -> Option<Option<String>> {
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
        let mut unresolved = false;
        for root in roots {
            let path = format!("{}.{name}", root.select(self, scope));
            let candidates = self.source_export_candidates_in(&path, Some(scope), Some(DeclarationNamespace::Type));
            inaccessible |= candidates.inaccessible || candidates.unresolved;
            unresolved |= candidates.unresolved;
            if candidates.declarations.is_empty() || candidates.only_values() {
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
        if unresolved { return Some(None); }
        match found {
            Some(canonical) => Some(Some(canonical)),
            None => inaccessible.then_some(None),
        }
    }

    pub(super) fn source_export_type_id(
        &self,
        path: &str,
        scope: &str,
    ) -> Option<crate::types::TypeId> {
        let selected = self.select_source_mount_path(path, scope);
        self.canonical_source_export_type_id(&selected, scope)
    }

    /// Resolve an already selected canonical path without rewriting its owner.
    pub(super) fn canonical_source_export_type_id(
        &self,
        path: &str,
        scope: &str,
    ) -> Option<crate::types::TypeId> {
        let (owner, name) = path.rsplit_once('.')?;
        // The language's reserved sums exist before their source bundle is
        // baked. Once a source scope or descriptor exists, its declaration and
        // visibility are authoritative; this cannot override a refusal.
        if let Some(id) = Self::canonical_sum_type_id(owner, name)
            && self.source_export_scope(owner).is_none()
            && self.type_by_id(id).is_none()
            && self.type_name_to_id.get(path).is_none_or(|selected| *selected == id)
        {
            return Some(id);
        }
        let mut consumer = scope;
        loop {
            let candidates = self.source_export_candidates_in(path, Some(consumer), Some(DeclarationNamespace::Type));
            if !candidates.inaccessible {
                return self.type_name_to_id.get(candidates.nominal()?).copied();
            }
            // Inline lexical children share their enclosing declaration scope;
            // dotted module names in separate files do not grant that access.
            consumer = self.lexical_type_parent(consumer)??;
        }
    }

    pub(super) fn resolve_source_value_export(
        &self, path: &str, consumer: &str,
    ) -> Result<Option<(String, FunctionInfo)>, String> {
        let mut scope = consumer;
        let candidates = loop {
            let candidates = self.source_export_candidates_in(path, Some(scope), Some(DeclarationNamespace::Value));
            if !candidates.inaccessible { break candidates; }
            let Some(Some(parent)) = self.lexical_type_parent(scope) else {
                return Err(format!("callable mount `{path}` is inaccessible from `{consumer}`"));
            };
            scope = parent;
        };
        if candidates.unresolved {
            return Err(format!("unresolved callable re-export for mount `{path}`"));
        }
        if candidates.declarations.len() > 1 {
            return Err(format!("ambiguous callable mount `{path}`: {}",
                candidates.declarations.keys().cloned().collect::<Vec<_>>().join(", ")));
        }
        if let Some((key, kinds)) = candidates.declarations.first_key_value() {
            if let Some(info) = self.ctx.lookup_function(key)
                && (kinds.contains(&DeclarationCategory::Callable)
                    || kinds.contains(&DeclarationCategory::Value) && info.is_const)
            {
                return Ok(Some((key.clone(), info.clone())));
            }
            if !kinds.contains(&DeclarationCategory::Callable) { return Ok(None); }
        } else {
            // A nominal constructor is available only when no source value
            // declaration occupies this name. A private function cannot borrow
            // a public same-named type's visibility through this path.
            let nominal = self.source_export_candidates_in(path, Some(scope), Some(DeclarationNamespace::Type));
            if nominal.inaccessible { return Err(format!("type mount `{path}` is inaccessible from `{consumer}`")); }
            if let Some(key) = nominal.nominal() {
                return Ok(self.ctx.lookup_function(key).map(|info| (key.clone(), info.clone())));
            }
            if !nominal.declarations.is_empty() { return Ok(None); }
            let namespace = self.source_export_candidates_in(path, Some(scope), Some(DeclarationNamespace::Module));
            if !namespace.declarations.is_empty() && !namespace.inaccessible { return Ok(None); }
        }
        Err(format!("no callable declaration for mount `{path}`"))
    }
}
