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

#[derive(Default)]
pub(super) struct CallableExports {
    modules: HashMap<String, ModuleExports>,
    declared_names: HashSet<String>,
}

#[derive(Default)]
struct ModuleExports {
    declarations: HashMap<String, BTreeSet<String>>,
    values: HashSet<String>,
    named: HashMap<String, BTreeSet<String>>,
    globs: BTreeSet<String>,
    public_children: BTreeSet<String>,
}

impl VbcCodegen {
    pub(super) fn collect_callable_exports(&mut self, owner: &str, items: &[Item]) {
        let mut exports = ModuleExports::default();
        for item in items {
            if !self.should_compile_item(item) {
                continue;
            }
            match &item.kind {
                ItemKind::Function(declaration) => {
                    let name = declaration.name.name.as_str();
                    exports
                        .declarations
                        .entry(name.to_owned())
                        .or_default()
                        .insert(format!("{owner}.{name}"));
                }
                ItemKind::Type(declaration) => {
                    exports.values.insert(declaration.name.name.to_string());
                }
                ItemKind::Const(declaration) => {
                    exports.values.insert(declaration.name.name.to_string());
                }
                ItemKind::Static(declaration) => {
                    exports.values.insert(declaration.name.name.to_string());
                }
                ItemKind::Mount(mount) if mount.visibility.is_public() => {
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
                    for parts in Self::extract_glob_mount_paths(&mount.tree, &[]) {
                        let path = Self::resolve_module_path_head_in_scope(&parts, owner)
                            .unwrap_or(parts)
                            .join(".");
                        exports.globs.insert(path);
                    }
                }
                ItemKind::Module(module) if module.items.is_some() => {
                    let child =
                        crate::module::qualify_module_name(owner, module.name.name.as_str());
                    self.collect_callable_exports(&child, module.items.as_ref().unwrap());
                    if module.visibility.is_public() {
                        exports.public_children.insert(child);
                    }
                }
                _ => {}
            }
        }
        self.source_callable_exports
            .declared_names
            .extend(exports.declarations.keys().cloned());
        let scope = self
            .source_callable_exports
            .modules
            .entry(owner.to_owned())
            .or_default();
        for (name, targets) in exports.declarations {
            scope.declarations.entry(name).or_default().extend(targets);
        }
        scope.values.extend(exports.values);
        for (alias, targets) in exports.named {
            scope.named.entry(alias).or_default().extend(targets);
        }
        scope.globs.extend(exports.globs);
        scope.public_children.extend(exports.public_children);
    }

    /// Called only by the sum declaration producer after it emits a real
    /// constructor. The parser also permits single-name aliases in variant
    /// syntax; only the emitted nominal can witness a constructor export.
    pub(super) fn record_source_constructor(&mut self, owner: &str, name: &str, key: &str) {
        let Some(source) = self.source_callable_exports.modules.get_mut(owner) else {
            return;
        };
        source
            .declarations
            .entry(name.to_owned())
            .or_default()
            .insert(key.to_owned());
        self.source_callable_exports
            .declared_names
            .insert(name.to_owned());
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
        let source = self.source_callable_exports.modules.get(owner)?;
        let named_callable = source.declarations.contains_key(name)
            || (!source.values.contains(name)
                && !self.type_name_to_id.contains_key(path.as_str())
                && [name, alias].into_iter().any(|leaf| {
                    self.source_callable_exports.declared_names.contains(leaf)
                        || self.ctx.functions.get(leaf).is_some_and(|info| {
                            info.variant_tag.is_some()
                                || info.has_source_body
                                    && info.parent_type_name.is_none()
                                    && !info.is_const
                                    && info.param_names.first().is_none_or(|first| first != "self")
                        })
                }));
        named_callable.then(|| path.into_string())
    }

    pub(super) fn resolve_source_callable_export(
        &self,
        path: &str,
    ) -> Result<(String, FunctionInfo), String> {
        let mut pending = vec![(path.to_owned(), true)];
        let mut visited = HashSet::new();
        let mut declarations = BTreeSet::new();
        while let Some((path, includes_declarations)) = pending.pop() {
            if !visited.insert((path.clone(), includes_declarations)) {
                continue;
            }
            let Some((owner, name)) = path.rsplit_once('.') else {
                continue;
            };
            let Some(source) = self.source_callable_exports.modules.get(owner) else {
                // An earlier archive can witness an exact external declaration.
                if self.ctx.functions.contains_key(&path) {
                    declarations.insert(path);
                }
                continue;
            };
            let anchored = |written: &str| {
                self.source_module_owners
                    .member(written, owner, self.source_module_root())
                    .map(|key| key.into_string())
                    .unwrap_or_else(|| written.to_owned())
            };
            if includes_declarations && let Some(targets) = source.declarations.get(name) {
                declarations.extend(targets.iter().cloned());
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
        if declarations.len() > 1 {
            return Err(format!(
                "ambiguous callable mount `{path}`: {}",
                declarations.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        let selected = declarations.pop_first().and_then(|key| {
            self.ctx
                .lookup_function(&key)
                .cloned()
                .map(|info| (key, info))
        });
        selected.ok_or_else(|| format!("no callable declaration for mount `{path}`"))
    }
}
