//! Nominal dependencies of a source unit compiled against earlier bootstrap units.

use super::{
    CodegenError, CodegenResult, FunctionInfo, VbcCodegen, copy_remapped_data_type_carriers,
    remap_type_ref_archive,
};
use crate::module::{FunctionDescriptor, FunctionId, VbcModule};
use crate::types::{TypeDescriptor, TypeId, TypeKind, TypeRef};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use verum_ast::{ItemKind, Visitor};
use super::source_mounts::{visit_named_mounts, mount_path_parts};
use verum_common::{Map, Text};

pub(super) fn identity(module: &VbcModule, ty: &TypeDescriptor) -> Option<String> {
    let name = module.strings.get(ty.name)?;
    let owner = ty
        .origin_module
        .and_then(|id| module.strings.get(id))
        .unwrap_or(&module.name);
    Some(crate::module::qualify_module_name(owner, name))
}

/// A declaration in its owning bundle outranks transitive copies. Within that
/// bundle, collection retains an imported carrier under its qualified name
/// when installing the source declaration under its unqualified name.
pub(super) fn declaration_priority(module: &VbcModule, ty: &TypeDescriptor) -> u8 {
    let owner = ty.origin_module
        .and_then(|id| module.strings.get(id))
        .unwrap_or(&module.name);
    let name = module.strings.get(ty.name).unwrap_or("");
    verum_common::qualified_name::declaration_priority(&module.name, owner, name)
}

// Shadowed imports retain their fully qualified declared name. Normalize only
// against that descriptor's own owner, exactly as archive type import does.
// Otherwise preallocation can invent a nominal ID for a canonical carrier while
// descriptor import installs the reserved ID, leaving every copied reference dangling.
fn declared_name<'a>(module: &'a VbcModule, ty: &TypeDescriptor) -> Option<&'a str> {
    let name = module.strings.get(ty.name)?;
    let owner = ty.origin_module
        .and_then(|id| module.strings.get(id))
        .unwrap_or(&module.name);
    Some(name.strip_prefix(owner).and_then(|tail| tail.strip_prefix('.')).unwrap_or(name))
}

fn type_ids(ty: &TypeRef, ids: &mut Vec<TypeId>) {
    match ty {
        TypeRef::Concrete(id) => ids.push(*id),
        TypeRef::Instantiated { base, args } => {
            ids.push(*base);
            for arg in args {
                type_ids(arg, ids);
            }
        }
        TypeRef::Function {
            params,
            return_type,
            ..
        }
        | TypeRef::Rank2Function {
            params,
            return_type,
            ..
        } => {
            for param in params {
                type_ids(param, ids);
            }
            type_ids(return_type, ids);
        }
        TypeRef::Reference { inner, .. }
        | TypeRef::Slice(inner)
        | TypeRef::Array { element: inner, .. }
        | TypeRef::AssociatedProjection { base: inner, .. } => type_ids(inner, ids),
        TypeRef::Tuple(elements) => {
            for element in elements {
                type_ids(element, ids);
            }
        }
        TypeRef::Generic(_) | TypeRef::ConstValue(_) => {}
    }
}

fn descriptor_ids(ty: &TypeDescriptor) -> Vec<TypeId> {
    let mut ids = Vec::new();
    for field in &ty.fields {
        type_ids(&field.type_ref, &mut ids);
        if let Some(ty) = &field.declaration_type { type_ids(ty, &mut ids); }
    }
    for variant in &ty.variants {
        if let Some(payload) = &variant.payload {
            type_ids(payload, &mut ids);
        }
        for field in &variant.fields {
            type_ids(&field.type_ref, &mut ids);
        if let Some(ty) = &field.declaration_type { type_ids(ty, &mut ids); }
        }
    }
    for param in &ty.type_params {
        ids.extend(param.bounds.iter().map(|id| TypeId(id.0)));
        if let Some(default) = &param.default {
            type_ids(default, &mut ids);
        }
        for bound in &param.type_bounds {
            type_ids(bound, &mut ids);
        }
    }
    if let Some(target) = &ty.alias_target {
        type_ids(target, &mut ids);
    }
    for implementation in &ty.protocols {
        ids.push(TypeId(implementation.protocol.0));
        for (_, target) in &implementation.associated_types {
            type_ids(target, &mut ids);
        }
    }
    ids
}

fn function_ids(function: &FunctionDescriptor) -> Vec<TypeId> {
    let mut ids = Vec::new();
    type_ids(&function.return_type, &mut ids);
    if let Some(result) = &function.semantic_return { type_ids(result, &mut ids); }
    if let Some(yield_type) = &function.yield_type {
        type_ids(yield_type, &mut ids);
    }
    for param in &function.params {
        type_ids(&param.type_ref, &mut ids);
    }
    if let Some(params) = function.semantic_parameter_types() {
        for ty in params.iter().flatten() { type_ids(ty, &mut ids); }
    }
    for param in &function.type_params {
        ids.extend(param.bounds.iter().map(|id| TypeId(id.0)));
        if let Some(default) = &param.default {
            type_ids(default, &mut ids);
        }
        for bound in &param.type_bounds {
            type_ids(bound, &mut ids);
        }
    }
    ids
}

fn function_identity(module: &VbcModule, function: &FunctionDescriptor) -> Option<String> {
    let name = module.strings.get(function.name)?;
    let owner = function
        .origin_module
        .and_then(|id| module.strings.get(id))
        .unwrap_or(&module.name);
    Some(crate::module::qualify_module_name(owner, name))
}

struct SourceNames<'a> {
    names: BTreeSet<String>,
    codegen: Option<&'a VbcCodegen>,
}
impl Visitor for SourceNames<'_> {
    fn visit_path(&mut self, path: &verum_ast::Path) {
        self.names.insert(path.to_string().replace("::", "."));
    }

    fn visit_expr(&mut self, expr: &verum_ast::Expr) {
        // The parser represents dotted free calls as MethodCall/Field. Reuse
        // the call lowering's module-path interpretation before matching the
        // exact source-owned function catalog.
        let Some(codegen) = self.codegen else {
            return verum_ast::visitor::walk_expr(self, expr);
        };
        match &expr.kind {
            verum_ast::ExprKind::MethodCall {
                receiver, method, ..
            } => {
                if let Some(mut path) = codegen.try_flatten_module_path_resolved(receiver) {
                    path.push(method.name.to_string());
                    self.names.insert(path.join("."));
                }
            }
            verum_ast::ExprKind::Field { .. } => {
                if let Some(path) = codegen.try_flatten_module_path_resolved(expr) {
                    self.names.insert(path.join("."));
                }
            }
            _ => {}
        }
        verum_ast::visitor::walk_expr(self, expr);
    }
}

impl VbcCodegen {
    /// Import the nominal dependency closure of source types, their methods,
    /// and free functions referenced by qualified paths or explicit mounts. Source module identities and source string pools remain the
    /// authority; unrelated same-leaf types never share a consumer identity.
    /// All ids are allocated before any structural TypeRef is copied.
    pub fn import_bootstrap_nominal_dependencies(
        &mut self,
        sources: &[&verum_ast::Module],
        available: &[&VbcModule],
    ) -> CodegenResult<usize> {
        self.import_bootstrap_nominal_dependencies_with_resolver(
            sources, available, |name| Some(Text::from(name)),
        )
    }

    /// Resolve written type export paths through the caller's source module
    /// authority before selecting descriptors. Resolutions are carried into
    /// the same ID table used by field and declaration TypeRefs; source text
    /// is never substituted for a lost ID after serialization.
    pub fn import_bootstrap_nominal_dependencies_with_resolver(
        &mut self,
        sources: &[&verum_ast::Module],
        available: &[&VbcModule],
        resolve_export: impl Fn(&str) -> Option<Text>,
    ) -> CodegenResult<usize> {
        // Own declarations must have their final IDs before any imported field,
        // bound, or method signature is translated into this codegen's space.
        // Later source prepasses reuse these exact reservations.
        self.claim_unit_source_type_identities(sources)?;
        let mut catalog = BTreeMap::<String, (usize, usize)>::new();
        let mut functions = BTreeMap::<String, (usize, usize)>::new();
        let mut source_keys = HashMap::new();
        let score = |index: usize, key: &str| {
            let name = &available[index].name;
            (
                key.starts_with(&format!("{name}.")),
                name.len(),
                name.clone(),
            )
        };
        for (mi, module) in available.iter().enumerate() {
            for (ti, ty) in module.types.iter().enumerate() {
                let Some(key) = identity(module, ty) else {
                    return Err(CodegenError::internal("bootstrap type has no source name"));
                };
                source_keys.insert((mi, ty.id), key.clone());
                // Prefer the defining unit over an imported copy. Resolve ties
                // independently of the caller's module iteration order.
                if catalog.get(&key).is_none_or(|&(old, old_type)| {
                    (declaration_priority(module, ty), score(mi, &key))
                        > (declaration_priority(available[old], &available[old].types[old_type]),
                            score(old, &key))
                })
                {
                    catalog.insert(key, (mi, ti));
                }
            }
            for (fi, function) in module.functions.iter().enumerate() {
                if function.parent_type.is_some() {
                    continue;
                }
                let Some(key) = function_identity(module, function) else {
                    continue;
                };
                if functions
                    .get(&key)
                    .is_none_or(|&(old, _)| score(mi, &key) > score(old, &key))
                {
                    functions.insert(key, (mi, fi));
                }
            }
        }
        // A forward interface may reference a reserved carrier before its owner
        // is compiled. Resolve it when the dependency closure reaches that ID.
        // An explicit descriptor in the source pool always takes precedence.
        let source_key = |module: usize, id: TypeId| -> Option<&String> {
            if let Some(key) = source_keys.get(&(module, id)) {
                return Some(key);
            }
            if id.is_builtin() { return None; }
            let name = id.well_known_name()?;
            let owner = verum_common::well_known_types::WellKnownType::from_name(name)?
                .canonical_archive_modules().first()?;
            catalog.get_key_value(&format!("{owner}.{name}")).map(|(key, _)| key)
        };
        let mut bare = HashMap::<String, Option<String>>::new();
        for key in catalog.keys() {
            let leaf = key.rsplit('.').next().unwrap().to_owned();
            bare.entry(leaf)
                .and_modify(|value| *value = None)
                .or_insert(Some(key.clone()));
        }
        let mut names = BTreeSet::new();
        let mut scopes: Vec<_> = sources.iter().map(|source| {
            let owner = self.source_file_owner(source, &self.config.module_name)
                .unwrap_or_else(|| self.config.module_name.clone());
            (owner, source.items.as_slice(), verum_ast::prelude::implicit_prelude_path(source))
        }).collect();
        while let Some((owner, items, prelude)) = scopes.pop() {
            let qualify = |written: String| {
                // Path's Relative segment renders as `.::`, hence `..`
                // after the same separator normalization used by mounts.
                let parts = mount_path_parts(&written);
                Self::resolve_module_path_head_in_scope(&parts, &owner)
                    .map(|parts| parts.join(".")).unwrap_or(written)
            };
            let mut paths = SourceNames {
                names: BTreeSet::new(),
                codegen: Some(self),
            };
            let mut bindings = HashMap::new();
            let mut globs = Vec::new();
            for item in items {
                if !self.should_compile_item(item) { continue; }
                if let ItemKind::Module(child) = &item.kind && let Some(items) = &child.items {
                    let scope = crate::module::qualify_module_name(&owner, child.name.name.as_str());
                    scopes.push((scope, items.as_slice(), prelude));
                    continue;
                }
                paths.visit_item(item);
                if let ItemKind::Mount(decl) = &item.kind {
                    visit_named_mounts(&decl.tree, "", decl.alias.as_ref(), &mut |alias, target| {
                        bindings.insert(alias.to_owned(), target);
                    });
                    globs.extend(Self::extract_glob_mount_paths(&decl.tree, &[])
                        .into_iter().map(|parts| {
                            Self::resolve_module_path_head_in_scope(&parts, &owner)
                                .unwrap_or(parts).join(".")
                        }));
                }
            }
            names.extend(bindings.values().cloned().map(&qualify));
            for name in paths.names {
                let (head, tail) = name.split_once('.').unwrap_or((&name, ""));
                if !bindings.contains_key(head) && (!globs.is_empty() || prelude.is_some()) {
                    // Resolve only the written glob's public exports. Retain
                    // its source authority so semantic signatures see the
                    // same binding while collecting and compiling this file.
                    let candidates: BTreeSet<_> = globs.iter().map(String::as_str).chain(prelude).filter_map(|glob| {
                        let written = format!("{glob}.{head}");
                        let declared = resolve_export(&written)
                            .or_else(|| resolve_export(&format!("core.{written}")))?;
                        let key = if catalog.contains_key(declared.as_str()) {
                            declared.into_string()
                        } else {
                            format!("core.{declared}")
                        };
                        let &(mi, ti) = catalog.get(&key)?;
                        matches!(available[mi].types[ti].visibility, crate::types::Visibility::Public)
                            .then_some(key)
                    }).collect();
                    if !candidates.is_empty() {
                        let selected = (candidates.len() == 1)
                            .then(|| candidates.first().unwrap().clone());
                        self.source_type_mounts.entry(owner.as_str().into()).or_default()
                            .insert(head.into(), selected.clone().map(Text::from));
                        if let Some(selected) = selected {
                            names.insert(if tail.is_empty() { selected }
                                else { format!("{selected}.{tail}") });
                        }
                        continue;
                    }
                    if tail.is_empty() && !globs.is_empty() {
                        // A glob miss cannot borrow an unrelated globally
                        // unique leaf. Local declarations are collected later.
                        continue;
                    }
                }
                names.insert(qualify(match bindings.get(head) {
                    Some(owner) if tail.is_empty() => owner.clone(),
                    Some(owner) => format!("{owner}.{tail}"),
                    None => name,
                }));
            }
        }
        let mut selected = BTreeSet::new();
        // import_protocols reserves protocol IDs in the emitted type table. A
        // reserved ID refers to its declaration even if no source item names it.
        // Complete semantic protocols through the normal descriptor+bindings path.
        for protocol in self.protocol_registry.values() {
            let key = crate::module::qualify_module_name(&protocol.source_module, &protocol.name);
            if self.type_name_to_id.contains_key(&key)
                && catalog.get(&key).is_some_and(|&(mi, ti)| {
                    available[mi].types[ti].source_declaration.is_some()
                })
            {
                selected.insert(key);
            }
        }

        let mut resolved_exports = Map::<Text, Text>::new();
        let mut function_sites = BTreeSet::new();
        for name in names {
            // Free-function roots require a written module path or an exact
            // mount binding. An unrelated bare registry alias is not authority.
            if name.contains('.') {
                if let Some(&site) = functions
                    .get(&name)
                    .or_else(|| functions.get(&format!("core.{name}")))
                {
                    function_sites.insert(site);
                }
            }
            let resolved = if name.contains('.') {
                resolve_export(&name).or_else(|| {
                    // The source callback adds type re-exports. An already
                    // exact archive declaration (including restricted
                    // exports) keeps the original import contract on a miss.
                    // Never broaden this to a same-leaf lookup.
                    (catalog.contains_key(&name)
                        || catalog.contains_key(&format!("core.{name}")))
                        .then(|| Text::from(name.as_str()))
                })
            } else {
                Some(Text::from(name.as_str()))
            };
            let Some(resolved) = resolved else { continue; };
            let key = if catalog.contains_key(resolved.as_str()) {
                Some(resolved.to_string())
            } else if catalog.contains_key(&format!("core.{resolved}")) {
                Some(format!("core.{resolved}"))
            } else if !resolved.contains(".") {
                bare.get(resolved.as_str()).cloned().flatten()
            } else {
                None
            };
            if let Some(key) = key {
                if name.contains('.') {
                    resolved_exports.insert(name.into(), Text::from(key.as_str()));
                }
                selected.insert(key);
            }
        }
        let mut pending: Vec<_> = selected.iter().cloned().collect();
        let mut referenced = HashMap::new();
        let mut add_dependencies = |mi: usize,
                                key: &str,
                                ids: Vec<TypeId>,
                                selected: &mut BTreeSet<String>,
                                pending: &mut Vec<String>|
         -> CodegenResult<()> {
            for id in ids {
                if let Some(dependency) = source_key(mi, id) {
                    if !source_keys.contains_key(&(mi, id)) { referenced.insert((mi, id), dependency); }
                    if selected.insert(dependency.clone()) {
                        pending.push(dependency.clone());
                    }
                } else if !id.is_builtin() && id.well_known_name().is_none() {
                    return Err(CodegenError::internal(format!(
                        "bootstrap nominal dependency {key} references unknown source TypeId {} in {}",
                        id.0, available[mi].name
                    )));
                }
            }
            Ok(())
        };
        for &(mi, fi) in &function_sites {
            let function = &available[mi].functions[fi];
            let key = function_identity(available[mi], function).ok_or_else(|| {
                CodegenError::internal(format!(
                    "bootstrap function {} has no source identity in {}",
                    function.id.0, available[mi].name
                ))
            })?;
            add_dependencies(
                mi,
                &key,
                function_ids(function),
                &mut selected,
                &mut pending,
            )?;
        }
        // Build each visited module's method index once. The owner is the
        // declaration identity, not its module-local carrier ID: cyclic
        // interfaces can retain several IDs for the same declaration.
        let mut methods_by_owner = HashMap::<usize, HashMap<&str, Vec<usize>>>::new();
        while let Some(key) = pending.pop() {
            let (mi, ti) = catalog[&key];
            let module = available[mi];
            let ty = &module.types[ti];
            // Inherited default bodies and their where-clauses carry source
            // paths beyond the protocol's runtime TypeRefs. Select those
            // dependencies from the declaring file, even if the implementor
            // never names them.
            if ty.kind == TypeKind::Protocol
                && let Some((owner, name)) = key.rsplit_once('.')
                && let Some(protocol) = self.protocol_registry.get(name)
                && protocol.source_module == owner && !protocol.default_methods.is_empty()
            {
                let mounts = module.declaration_type_bindings.get(&Text::from(owner));
                let mut paths = SourceNames { names: BTreeSet::new(), codegen: None };
                for method in protocol.default_methods.values() {
                    paths.visit_function(method);
                }
                for name in paths.names {
                    let local = crate::module::qualify_module_name(owner, &name);
                    let dependency = if catalog.contains_key(&local) {
                        Some(local)
                    } else if let Some(mount) = mounts.and_then(|scope| scope.get(&Text::from(name.as_str()))) {
                        mount.as_ref().map(|path| path.to_string())
                    } else if name.contains('.') {
                        Some(name)
                    } else {
                        None
                    };
                    if let Some(dependency) = dependency
                        && catalog.contains_key(&dependency) && selected.insert(dependency.clone())
                    {
                        pending.push(dependency);
                    }
                }
                self.retain_declaration_type_scope(module, owner)?;
            }
            add_dependencies(mi, &key, descriptor_ids(ty), &mut selected, &mut pending)?;
            let methods = methods_by_owner.entry(mi).or_insert_with(|| {
                let mut methods = HashMap::<&str, Vec<usize>>::new();
                for (fi, function) in module.functions.iter().enumerate() {
                    if let Some(owner) = function.parent_type
                        .and_then(|id| source_key(mi, id))
                    {
                        methods.entry(owner.as_str()).or_default().push(fi);
                    }
                }
                methods
            });
            if let Some(methods) = methods.get(key.as_str()) {
                for &fi in methods {
                    let function = &module.functions[fi];
                    function_sites.insert((mi, fi));
                    let method = function_identity(module, function).ok_or_else(|| {
                        CodegenError::internal("bootstrap method has no source identity")
                    })?;
                    add_dependencies(mi, &method, function_ids(function), &mut selected, &mut pending)?;
                }
            }
        }
        let mut target_ids = HashMap::new();
        for key in &selected {
            let (mi, ti) = catalog[key];
            let source = &available[mi].types[ti];
            let leaf = declared_name(available[mi], source).unwrap();
            let id = source.canonical_scalar_type_id(leaf)
                .or_else(|| source.canonical_record_type_id(
                    leaf,
                    source.origin_module.and_then(|owner| available[mi].strings.get(owner)),
                ))
                .or_else(|| self.type_name_to_id.get(key).copied())
                .unwrap_or_else(|| {
                    self.type_name_to_id
                        .get(leaf)
                        .copied()
                        .filter(|id| {
                            id.well_known_name() == Some(leaf)
                                // This legacy ABI allocation guard is separate
                                // from export resolution above. Even canonical
                                // source descriptors have module-local IDs; only
                                // their declaring owner may complete a reserved
                                // carrier, never an unrelated same-leaf type.
                                && verum_common::well_known_types::WellKnownType::from_name(leaf)
                                    .and_then(|known| known.canonical_archive_modules().first().copied())
                                    == key.rsplit_once('.').map(|(owner, _)| owner)
                                // A same-named nominal is not a scalar. Genuine
                                // scalar carriers were selected above by kind+ID.
                                && TypeId::from_well_known_scalar_name(leaf).is_none()
                                && self.type_by_id(*id).is_none()
                                && (!verum_common::well_known_types::BUILTIN_VARIANT_CARRIERS
                                    .iter()
                                    .any(|(name, _)| *name == leaf)
                                    || key.rsplit_once('.').is_some_and(|(owner, name)| {
                                        Self::canonical_sum_type_id(owner, name) == Some(*id)
                                    }))
                        })
                        .unwrap_or_else(|| self.alloc_user_type_id())
                });
            self.type_name_to_id.insert(key.clone(), id);
            if let Some(short) = key.strip_prefix("core.") {
                self.type_name_to_id.insert(short.to_owned(), id);
            }
            target_ids.insert(key.clone(), id);
        }
        let mut method_exports = HashMap::<String, Vec<String>>::new();
        for (written, declared) in resolved_exports {
            if let Some(id) = target_ids.get(declared.as_str()) {
                self.type_name_to_id.insert(written.to_string(), *id);
                if written != declared {
                    method_exports.entry(declared.into_string()).or_default()
                        .push(written.into_string());
                }
            }
        }
        // Source IDs are local to their archive module. Partition the exact
        // (module, source ID) identities once instead of rescanning the whole
        // catalogue for every module; equal numeric IDs never share a map.
        let mut maps = vec![HashMap::<u32, u32>::new(); available.len()];
        for (&(owner, source), key) in &source_keys {
            if let Some(target) = target_ids.get(key) {
                maps[owner].insert(source.0, target.0);
            }
        }
        for ((owner, source), key) in referenced {
            maps[owner].insert(source.0, target_ids[key].0);
        }
        for key in &selected {
            let (mi, ti) = catalog[key];
            let module = available[mi];
            if module.types[ti].source_declaration.is_some() {
                let owner = module.types[ti].origin_module.and_then(|id| module.strings.get(id))
                    .unwrap_or(&module.name);
                self.retain_declaration_type_scope(module, owner)?;
            }
            let mut ty = module.types[ti].clone();
            // Retain this fact from the original descriptor: unresolved glue
            // becomes None below, but that loss cannot authorize absorption.
            let carrier_eligible = declared_name(module, &module.types[ti])
                .is_some_and(|name| module.types[ti].is_impl_only_scalar_carrier(name));
            let remap = &maps[mi];
            let tr = |value: &TypeRef| remap_type_ref_archive(value, remap);
            copy_remapped_data_type_carriers(&module.types[ti], &mut ty, remap);
            for implementation in &mut ty.protocols {
                implementation.protocol.0 = *remap
                    .get(&implementation.protocol.0)
                    .unwrap_or(&implementation.protocol.0);
                for (_, target) in &mut implementation.associated_types {
                    *target = tr(target);
                }
                // Protocol method slots in existing archives have no per-slot
                // module provenance. Keep their declaration bindings,
                // and let canonical method lookup select bodies; do not turn
                // a coincident source-local number into an unrelated method.
                implementation.methods.clear();
            }
            ty.drop_fn = ty
                .drop_fn
                .and_then(|id| self.bootstrap_function(module, id).map(|info| info.id.0));
            ty.clone_fn = ty
                .clone_fn
                .and_then(|id| self.bootstrap_function(module, id).map(|info| info.id.0));
            let owner = ty
                .origin_module
                .and_then(|id| module.strings.get(id))
                .unwrap_or(&module.name);
            self.import_archive_type_with_carrier_eligibility(
                &ty,
                &module.strings,
                &HashMap::new(),
                Some(owner),
                Some(carrier_eligible),
            );
            if let Some((owner, leaf)) = key.rsplit_once('.')
                && self.user_claimed_type_names.contains(&(owner.to_owned(), leaf.to_owned()))
            {
                self.bootstrap_local_forwards.insert(Text::from(key.as_str()));
            }
            let local = target_ids[key];
            // Old descriptors without origin still have a known owning unit.
            // Carry it now so a second import cannot attribute them to us.
            let origin = crate::types::StringId(self.ctx.intern_string_raw(owner));
            // Scalar aliases share an ID, but their named carriers retain
            // separate source metadata (for example Byte and UInt8).
            let scalar_name = declared_name(module, &ty)
                .filter(|name| ty.canonical_scalar_type_id(name).is_some())
                .map(|name| crate::types::StringId(self.ctx.intern_string_raw(name)));
            if let Some(imported) = self.types.iter_mut().find(|imported| {
                imported.id == local && scalar_name.is_none_or(|name| {
                    imported.name == name && imported.kind == ty.kind
                })
            }) {
                imported.origin_module = Some(origin);
                // Unlike a general archive import, these two fields have
                // already been translated through the bootstrap registry.
                imported.drop_fn = ty.drop_fn;
                imported.clone_fn = ty.clone_fn;
            }
            let params = ty
                .type_params
                .iter()
                .filter_map(|param| module.strings.get(param.name).map(str::to_owned))
                .collect::<Vec<_>>();
            self.ctx
                .type_generic_params
                .insert(key.clone(), params.clone());
            let leaf = declared_name(module, &ty).unwrap();
            if self.type_name_to_id.get(leaf) == Some(&local) {
                self.ctx.type_generic_params.insert(leaf.to_owned(), params);
            }
        }
        // FunctionInfo is another pool-owned TypeRef carrier: leaving its
        // return in the source pool would undo the type import on a call chain.
        let mut signatures = Map::new();
        let mut callable_exports = Vec::new();
        for (mi, fi) in function_sites {
            let module = available[mi];
            let function = &module.functions[fi];
            if function.source_signature.is_some() {
                let owner = function.origin_module.and_then(|id| module.strings.get(id))
                    .unwrap_or(&module.name);
                self.retain_declaration_type_scope(module, owner)?;
            }
            if let Some(info) = self.bootstrap_function(module, function.id.0) {
                let id = info.id.0;
                // Forward registrations may have no source signature yet.
                // The declaration interface owns the exact semantic result
                // and parameters; ABI return types are not evidence.
                let callable = super::formal_parameters::remapped_callable(function, &maps[mi]).or_else(|| {
                    info.callable_signature.as_ref().map(|ty| remap_type_ref_archive(ty, &maps[mi]))
                });
                // Pattern and overload lookup still consume a type spelling.
                // Derive it from the remapped source signature, so an optional
                // forward hint cannot lose or substitute the payload's owner.
                let result_name = callable.as_ref().and_then(|signature| {
                    let TypeRef::Function { return_type, .. } = signature else { return None };
                    let scope = function.type_params.iter().filter_map(|parameter|
                        module.strings.get(parameter.name).map(|name| (name.to_owned(), parameter.id.0)))
                        .collect();
                    self.scoped_callable_type_name(return_type, Some(&scope)).map(Text::into_string)
                });
                // A type re-export also names its declaration-owned methods.
                // Keep their actual FunctionInfo identity and generic roster;
                // synthesizing an external stub here would erase CallG witnesses.
                if let Some(parent) = function.parent_type
                    .and_then(|parent| source_key(mi, parent))
                    && let Some(exports) = method_exports.get(parent)
                    && let Some(canonical) = function_identity(module, function)
                    && let Some(member) = canonical.strip_prefix(&format!("{parent}."))
                {
                    for written in exports {
                        callable_exports.push((format!("{written}.{member}"), canonical.clone()));
                    }
                }
                let result = remap_type_ref_archive(&function.return_type, &maps[mi]);
                // Bootstrap consumers compile before imported bodies exist in
                // `self.functions`. Preserve the same declaration-owned input
                // facts used by ordinary archive call lowering, in this pool.
                self.ctx.archive_fn_param_types.insert(
                    id,
                    function.params.iter().map(|parameter|
                        remap_type_ref_archive(&parameter.type_ref, &maps[mi])).collect(),
                );
                self.ctx.semantic_fn_params.remove(&FunctionId(id));
                if let Some(params) = super::formal_parameters::remapped_parameters(function, &maps[mi]) {
                    self.ctx.semantic_fn_params.insert(FunctionId(id), params);
                }
                self.ctx.archive_fn_parameter_generics.insert(
                    id,
                    (0..function.params.len()).map(|index|
                        function.parameter_generic_id(index, |name| module.strings.get(name))).collect(),
                );
                // Preserve the ordered sites' last update for a shared ID.
                // Resolution reads names/IDs only, so signatures can be applied
                // together after all source-owned parameter facts are carried.
                signatures.insert(FunctionId(id), ImportedSignature {
                    result,
                    yielded: function.yield_type.as_ref()
                        .map(|ty| remap_type_ref_archive(ty, &maps[mi])),
                    callable,
                    result_name,
                    parameter_names: function.params.iter().map(|parameter|
                        module.strings.get(parameter.name).map(str::to_owned))
                        .collect::<Option<_>>()
                        .ok_or_else(|| CodegenError::internal("missing source parameter name"))?,
                });
            }
        }
        apply_function_signatures(self.ctx.functions.values_mut(), &signatures);
        for (export, canonical) in callable_exports {
            if let Some(info) = self.ctx.lookup_qualified_function(&canonical).cloned() {
                self.ctx.register_function_authoritative(export, info);
            }
        }
        Ok(selected.len())
    }

    pub(super) fn nominal_type_id(&self, name: &str) -> Option<TypeId> {
        let lookup = |name: &str| {
            self.type_name_to_id.get(name).copied().or_else(|| {
                // Bootstrap can compile an intrinsic before core.base. Its
                // explicit mount still denotes the reserved canonical sum;
                // an unrelated same-leaf mount has no such authority.
                let (owner, leaf) = name.rsplit_once('.')?;
                Self::canonical_sum_type_id(owner, leaf)
            })
        };
        let owner = self.lexical_type_scope();
        // A declaration in this file shadows its imports. A same-named type in
        // a different file of the bootstrap unit owns only its qualified name.
        if !name.contains('.')
            && let Some(local) = self.type_name_to_id.get(&format!("{owner}.{name}"))
        {
            return Some(*local);
        }
        let owner_key = Text::from(owner);
        let name_key = Text::from(name);
        let binding = match self.source_type_mounts.get(&owner_key) {
            Some(scope) => scope.get(&name_key),
            None => self.imported_declaration_type_bindings.get(&owner_key)
                .and_then(Option::as_ref).and_then(|scope| scope.get(&name_key)),
        };
        if binding == Some(&None) { return None; }
        if let Some(mounted) = self.mounted_type_path(name) {
            return lookup(mounted).or_else(|| lookup(&format!("core.{mounted}")))
                .or_else(|| self.source_export_type_id(mounted, owner));
        }
        if let Some(selected) = self.source_glob_type_id(name, owner) { return selected; }
        // A language scalar spelling has declaration authority in every scope.
        // An unrelated module's bare compatibility entry cannot shadow it.
        TypeId::from_well_known_scalar_name(name).or_else(|| lookup(name))
    }

    fn bootstrap_function(&self, module: &VbcModule, id: u32) -> Option<&FunctionInfo> {
        if let Some((_, name)) = module
            .external_function_names
            .iter()
            .find(|(source, _)| source.0 == id)
        {
            return module
                .strings
                .get(*name)
                .and_then(|name| self.ctx.functions.get(name));
        }
        let function = module
            .functions
            .iter()
            .find(|function| function.id.0 == id)?;
        self.ctx
            .functions
            .get(&function_identity(module, function)?)
    }
}

// Read current alias ownership once; a persistent reverse index could retain
// names that authoritative registration has since rebound to another ID.
struct ImportedSignature {
    result: TypeRef,
    yielded: Option<TypeRef>,
    callable: Option<TypeRef>,
    result_name: Option<String>,
    parameter_names: verum_common::List<String>,
}

fn apply_function_signatures<'a>(
    functions: impl Iterator<Item = &'a mut FunctionInfo>,
    signatures: &Map<FunctionId, ImportedSignature>,
) {
    if signatures.is_empty() {
        return;
    }
    for info in functions {
        if let Some(signature) = signatures.get(&info.id) {
            info.return_type = Some(signature.result.clone());
            info.yield_type = signature.yielded.clone();
            // Stage-one placeholders do not establish receiver roles or named
            // arguments. Restore them from the same source-owned declaration
            // that supplies structural parameter and result types.
            info.param_names = signature.parameter_names.as_slice().to_vec();
            info.param_count = info.param_names.len();
            // Preserve the source signature, including reference qualifiers,
            // while translating every nominal identity into the consumer pool.
            // Reconstructing it from ABI parameters would lose those qualifiers.
            info.callable_signature = signature.callable.clone();
            if let Some(name) = &signature.result_name {
                info.return_type_name = Some(name.clone());
                info.return_type_inner = None;
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/codegen/bootstrap_signature_updates.rs"]
mod signature_update_tests;
