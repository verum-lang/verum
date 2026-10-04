//! Nominal dependencies of a source unit compiled against earlier bootstrap units.

use super::{CodegenError, CodegenResult, VbcCodegen, remap_type_ref_archive};
use crate::module::{FunctionDescriptor, VbcModule};
use crate::types::{TypeDescriptor, TypeId, TypeRef};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use verum_ast::{ItemKind, MountTree, MountTreeKind, Visitor};

pub(super) fn identity(module: &VbcModule, ty: &TypeDescriptor) -> Option<String> {
    let name = module.strings.get(ty.name)?;
    let owner = ty
        .origin_module
        .and_then(|id| module.strings.get(id))
        .unwrap_or(&module.name);
    Some(crate::module::qualify_module_name(owner, name))
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
    }
    for variant in &ty.variants {
        if let Some(payload) = &variant.payload {
            type_ids(payload, &mut ids);
        }
        for field in &variant.fields {
            type_ids(&field.type_ref, &mut ids);
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
    if let Some(yield_type) = &function.yield_type {
        type_ids(yield_type, &mut ids);
    }
    for param in &function.params {
        type_ids(&param.type_ref, &mut ids);
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

fn mounts(
    tree: &MountTree,
    prefix: &str,
    outer_alias: Option<&verum_ast::ty::Ident>,
    bindings: &mut HashMap<String, String>,
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
            bindings.insert(alias.to_owned(), full);
        }
        MountTreeKind::Nested { prefix, trees } => {
            let prefix = path(prefix);
            for tree in trees {
                mounts(tree, &prefix, None, bindings);
            }
        }
        MountTreeKind::Glob(_) | MountTreeKind::File { .. } => {}
    }
}

struct SourceNames<'a> {
    names: BTreeSet<String>,
    codegen: &'a VbcCodegen,
}
impl Visitor for SourceNames<'_> {
    fn visit_path(&mut self, path: &verum_ast::Path) {
        self.names.insert(path.to_string().replace("::", "."));
    }

    fn visit_expr(&mut self, expr: &verum_ast::Expr) {
        // The parser represents dotted free calls as MethodCall/Field. Reuse
        // the call lowering's module-path interpretation before matching the
        // exact source-owned function catalog.
        match &expr.kind {
            verum_ast::ExprKind::MethodCall {
                receiver, method, ..
            } => {
                if let Some(mut path) = self.codegen.try_flatten_module_path_resolved(receiver) {
                    path.push(method.name.to_string());
                    self.names.insert(path.join("."));
                }
            }
            verum_ast::ExprKind::Field { .. } => {
                if let Some(path) = self.codegen.try_flatten_module_path_resolved(expr) {
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
                if catalog
                    .get(&key)
                    .is_none_or(|&(old, _)| score(mi, &key) > score(old, &key))
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
        let mut bare = HashMap::<String, Option<String>>::new();
        for key in catalog.keys() {
            let leaf = key.rsplit('.').next().unwrap().to_owned();
            bare.entry(leaf)
                .and_modify(|value| *value = None)
                .or_insert(Some(key.clone()));
        }
        let mut names = BTreeSet::new();
        for source in sources {
            let mut paths = SourceNames {
                names: BTreeSet::new(),
                codegen: self,
            };
            let mut bindings = HashMap::new();
            for item in &source.items {
                paths.visit_item(item);
                if let ItemKind::Mount(decl) = &item.kind {
                    mounts(&decl.tree, "", decl.alias.as_ref(), &mut bindings);
                }
            }
            names.extend(bindings.values().cloned());
            for name in paths.names {
                let (head, tail) = name.split_once('.').unwrap_or((&name, ""));
                names.insert(match bindings.get(head) {
                    Some(owner) if tail.is_empty() => owner.clone(),
                    Some(owner) => format!("{owner}.{tail}"),
                    None => name,
                });
            }
        }
        let mut selected = BTreeSet::new();
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
            let key = if catalog.contains_key(&name) {
                Some(name.clone())
            } else if catalog.contains_key(&format!("core.{name}")) {
                Some(format!("core.{name}"))
            } else if !name.contains('.') {
                bare.get(&name).cloned().flatten()
            } else {
                None
            };
            if let Some(key) = key {
                selected.insert(key);
            }
        }
        let mut pending: Vec<_> = selected.iter().cloned().collect();
        let add_dependencies = |mi: usize,
                                key: &str,
                                ids: Vec<TypeId>,
                                selected: &mut BTreeSet<String>,
                                pending: &mut Vec<String>|
         -> CodegenResult<()> {
            for id in ids {
                if let Some(dependency) = source_keys.get(&(mi, id)) {
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
        while let Some(key) = pending.pop() {
            let (mi, ti) = catalog[&key];
            let module = available[mi];
            let ty = &module.types[ti];
            let mut ids = descriptor_ids(ty);
            for (fi, function) in module.functions.iter().enumerate() {
                if function.parent_type == Some(ty.id) {
                    function_sites.insert((mi, fi));
                    ids.extend(function_ids(function));
                }
            }
            add_dependencies(mi, &key, ids, &mut selected, &mut pending)?;
        }
        let mut target_ids = HashMap::new();
        for key in &selected {
            let (mi, ti) = catalog[key];
            let source = &available[mi].types[ti];
            let leaf = available[mi].strings.get(source.name).unwrap();
            let id = self.type_name_to_id.get(key).copied().unwrap_or_else(|| {
                self.type_name_to_id
                    .get(leaf)
                    .copied()
                    .filter(|id| {
                        id.well_known_name() == Some(leaf)
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
        let maps: Vec<HashMap<u32, u32>> = available
            .iter()
            .enumerate()
            .map(|(mi, _)| {
                source_keys
                    .iter()
                    .filter_map(|(&(owner, source), key)| {
                        (owner == mi)
                            .then(|| target_ids.get(key).map(|id| (source.0, id.0)))
                            .flatten()
                    })
                    .collect()
            })
            .collect();
        for key in &selected {
            let (mi, ti) = catalog[key];
            let module = available[mi];
            let mut ty = module.types[ti].clone();
            let remap = &maps[mi];
            let tr = |value: &TypeRef| remap_type_ref_archive(value, remap);
            for field in &mut ty.fields {
                field.type_ref = tr(&field.type_ref);
            }
            for variant in &mut ty.variants {
                if let Some(payload) = &mut variant.payload {
                    *payload = tr(payload);
                }
                for field in &mut variant.fields {
                    field.type_ref = tr(&field.type_ref);
                }
            }
            for param in &mut ty.type_params {
                for bound in &mut param.bounds {
                    bound.0 = *remap.get(&bound.0).unwrap_or(&bound.0);
                }
                param.default = param.default.as_ref().map(tr);
                for bound in &mut param.type_bounds {
                    *bound = tr(bound);
                }
            }
            ty.alias_target = ty.alias_target.as_ref().map(tr);
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
                .and_then(|id| self.bootstrap_function_id(module, id));
            ty.clone_fn = ty
                .clone_fn
                .and_then(|id| self.bootstrap_function_id(module, id));
            let owner = ty
                .origin_module
                .and_then(|id| module.strings.get(id))
                .unwrap_or(&module.name);
            self.import_archive_type_with_protocol_remap_qualified(
                &ty,
                &module.strings,
                &HashMap::new(),
                Some(owner),
            );
            let local = target_ids[key];
            // Old descriptors without origin still have a known owning unit.
            // Carry it now so a second import cannot attribute them to us.
            let origin = crate::types::StringId(self.ctx.intern_string_raw(owner));
            if let Some(imported) = self.types.iter_mut().find(|ty| ty.id == local) {
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
            let leaf = module.strings.get(ty.name).unwrap();
            if self.type_name_to_id.get(leaf) == Some(&local) {
                self.ctx.type_generic_params.insert(leaf.to_owned(), params);
            }
        }
        // FunctionInfo is another pool-owned TypeRef carrier: leaving its
        // return in the source pool would undo the type import on a call chain.
        for (mi, fi) in function_sites {
            let module = available[mi];
            let function = &module.functions[fi];
            if let Some(id) = self.bootstrap_function_id(module, function.id.0) {
                let result = remap_type_ref_archive(&function.return_type, &maps[mi]);
                for info in self
                    .ctx
                    .functions
                    .values_mut()
                    .filter(|info| info.id.0 == id)
                {
                    info.return_type = Some(result.clone());
                    info.yield_type = function
                        .yield_type
                        .as_ref()
                        .map(|ty| remap_type_ref_archive(ty, &maps[mi]));
                }
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
        if !self.local_concrete_types.contains(name)
            && let Some(mounted) = self.ctx.mounted_types.get(name)
        {
            return lookup(mounted).or_else(|| lookup(&format!("core.{mounted}")));
        }
        lookup(name)
    }

    fn bootstrap_function_id(&self, module: &VbcModule, id: u32) -> Option<u32> {
        if let Some((_, name)) = module
            .external_function_names
            .iter()
            .find(|(source, _)| source.0 == id)
        {
            return module
                .strings
                .get(*name)
                .and_then(|name| self.ctx.functions.get(name))
                .map(|info| info.id.0);
        }
        let function = module
            .functions
            .iter()
            .find(|function| function.id.0 == id)?;
        self.ctx
            .functions
            .get(&function_identity(module, function)?)
            .map(|info| info.id.0)
    }
}
