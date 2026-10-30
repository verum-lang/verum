//! Declaration-owned module surfaces for precompiled type metadata.
//!
//! Archive entries aggregate files; their names do not authorize re-exports.
//! Synthetic ASTs here carry no source navigation or executable proof authority.
use crate::core_metadata::{CoreMetadata, TypeDescriptorKind};
use verum_ast::{FileId, Module, Span, Visibility};
use verum_common::{List, Maybe, OrderedMap, Set, Text};
use verum_modules::{ExportKind, ExportedItem, ModuleError, ModuleInfo, ModulePath, ModuleResult};

#[derive(Clone, PartialEq)]
struct Declaration {
    kind: ExportKind,
    visibility: Visibility,
    owner: Text,
    original: Text,
}
type Surfaces = OrderedMap<Text, OrderedMap<Text, Declaration>>;

fn owner<'a>(entry: &'a Text, origin: &'a Maybe<Text>) -> &'a Text {
    origin
        .as_ref()
        .filter(|name| !name.is_empty())
        .unwrap_or(entry)
}

/// Only strip a recorded module owner: `Type.method` is not a free `method`.
fn local_name<'a>(name: &'a str, owner: &str, entry: &str) -> Option<&'a str> {
    let mut local = name;
    for module in [owner, entry] {
        if !module.is_empty() {
            if let Some(rest) = name
                .strip_prefix(module)
                .and_then(|rest| rest.strip_prefix('.').or_else(|| rest.strip_prefix("::")))
            {
                local = rest;
                break;
            }
        }
    }
    // `$` separates generated nested-function symbols in VBC; it cannot occur
    // in a source identifier, even when that generated function is marked public.
    (!local.is_empty() && !local.contains(['.', ':', '$'])).then_some(local)
}

fn insert(
    surfaces: &mut Surfaces,
    module: &Text,
    name: &str,
    declaration: Declaration,
) -> ModuleResult<bool> {
    let surface = surfaces.entry(module.clone()).or_default();
    if let Some(previous) = surface.get(&Text::from(name)) {
        if previous == &declaration {
            return Ok(false);
        }
        return Err(ModuleError::Other {
            message: format!(
                "conflicting metadata export {module}.{name}: {:?} {}.{} and {:?} {}.{}",
                previous.kind,
                previous.owner,
                previous.original,
                declaration.kind,
                declaration.owner,
                declaration.original
            )
            .into(),
            span: None,
        });
    }
    surface.insert(name.into(), declaration);
    Ok(true)
}

fn visibility(public: bool) -> Visibility {
    if public {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

fn selected_modules(metadata: &CoreMetadata, scope: Option<&Set<Text>>) -> Option<Set<Text>> {
    let mut selected = scope?.clone();
    // Finite closure: umbrellas and explicit dependencies, never descendants
    // selected by string prefix. No fixed depth cap is necessary.
    loop {
        let previous = selected.len();
        for module in selected.clone() {
            let mut prefix = module.as_str();
            while let Some(cut) = prefix.rfind('.') {
                prefix = &prefix[..cut];
                selected.insert(prefix.into());
            }
            if let Some(exports) = metadata.module_reexports.get(&module) {
                selected.extend(exports.iter().map(|(_, _, source)| source.clone()));
            }
        }
        if selected.len() == previous {
            return Some(selected);
        }
    }
}

/// Project metadata into module records shared by compiler and analysis hosts.
///
/// `None` selects all modules. A scope includes umbrellas and explicit re-export
/// dependencies. Allocate a distinct stable ID for each path, reusing existing
/// registry IDs where present. Callers own insertion, caching and source maps.
pub fn project_modules(
    metadata: &CoreMetadata,
    scope: Option<&Set<Text>>,
    mut allocate_id: impl FnMut(&ModulePath) -> verum_modules::ModuleId,
) -> ModuleResult<List<ModuleInfo>> {
    let selected = selected_modules(metadata, scope);
    let in_scope =
        |module: &Text| !module.is_empty() && selected.as_ref().is_none_or(|s| s.contains(module));
    let mut surfaces = Surfaces::new();
    // Bare and qualified map keys are lookup aliases, not extra declarations.
    for descriptor in metadata.types.values() {
        let module = owner(&descriptor.module_path, &descriptor.origin_module_path);
        if !in_scope(module) {
            continue;
        }
        surfaces.entry(module.clone()).or_default();
        let Some(name) = local_name(&descriptor.name, module, &descriptor.module_path) else {
            continue;
        };
        let name_key = Text::from(name);
        let context = metadata
            .context_declaring_modules
            .get(&name_key)
            .filter(|owner| *owner == module)
            .and_then(|_| metadata.context_decl_nodes.get(&name_key));
        let declared_visibility = context
            .map(|decl| decl.visibility.clone())
            .unwrap_or_else(|| visibility(descriptor.is_public));
        if declared_visibility != Visibility::Public {
            continue;
        }
        insert(
            &mut surfaces,
            module,
            name,
            Declaration {
                kind: if context.is_some() {
                    ExportKind::Context
                } else {
                    ExportKind::Type
                },
                visibility: declared_visibility,
                owner: module.clone(),
                original: name.into(),
            },
        )?;
    }
    // Protocol descriptors have no independent visibility field. The matching
    // declared type above owns visibility; protocol-only legacy data cannot
    // authorize a public export.
    for descriptor in metadata.types.values() {
        let module = owner(&descriptor.module_path, &descriptor.origin_module_path);
        if !in_scope(module) {
            continue;
        }
        let TypeDescriptorKind::Variant { cases } = &descriptor.kind else {
            continue;
        };
        if !descriptor.is_public {
            continue;
        }
        for case in cases {
            let Some(name) = local_name(&case.name, module, &descriptor.module_path) else {
                continue;
            };
            // A payloadless case naming a declared type is a type union, matching
            // source export extraction; it does not introduce a constructor.
            if case.payload.is_none()
                && surfaces
                    .get(module)
                    .and_then(|s| s.get(&Text::from(name)))
                    .is_some_and(|d| d.kind == ExportKind::Type)
            {
                continue;
            }
            insert(
                &mut surfaces,
                module,
                name,
                Declaration {
                    kind: ExportKind::Function,
                    visibility: visibility(descriptor.is_public),
                    owner: module.clone(),
                    original: name.into(),
                },
            )?;
        }
    }
    for descriptor in metadata.functions.values() {
        if descriptor.parent_type.is_some() {
            continue;
        }
        let module = owner(&descriptor.module_path, &descriptor.origin_module_path);
        if !in_scope(module) {
            continue;
        }
        surfaces.entry(module.clone()).or_default();
        if !descriptor.is_public {
            continue;
        }
        let Some(name) = local_name(&descriptor.name, module, &descriptor.module_path) else {
            continue;
        };
        insert(
            &mut surfaces,
            module,
            name,
            Declaration {
                kind: if descriptor.is_const {
                    ExportKind::Const
                } else {
                    ExportKind::Function
                },
                visibility: visibility(descriptor.is_public),
                owner: module.clone(),
                original: name.into(),
            },
        )?;
    }
    for descriptor in metadata.statics.values() {
        let module = &descriptor.module_path;
        if !in_scope(module) {
            continue;
        }
        let Some(name) = local_name(&descriptor.name, module, module) else {
            continue;
        };
        // The producer populates this channel exclusively from public statics.
        insert(
            &mut surfaces,
            module,
            name,
            Declaration {
                kind: ExportKind::Static,
                visibility: Visibility::Public,
                owner: module.clone(),
                original: name.into(),
            },
        )?;
    }
    for (name, module) in &metadata.context_declaring_modules {
        if !in_scope(module) {
            continue;
        }
        let Some(decl) = metadata.context_decl_nodes.get(name) else {
            continue;
        };
        surfaces.entry(module.clone()).or_default();
        if decl.visibility != Visibility::Public {
            continue;
        }
        insert(
            &mut surfaces,
            module,
            name,
            Declaration {
                kind: ExportKind::Context,
                visibility: decl.visibility.clone(),
                owner: module.clone(),
                original: name.clone(),
            },
        )?;
    }
    for module in metadata
        .module_reexports
        .keys()
        .filter(|module| in_scope(module))
    {
        surfaces.entry(module.clone()).or_default();
    }
    // Resolve chains only from existing declarations. Missing/cyclic targets
    // create no fabricated exports; re-export lists cannot elevate private names.
    loop {
        let mut added = false;
        for (module, exports) in &metadata.module_reexports {
            if !in_scope(module) {
                continue;
            }
            for (local, original, source) in exports {
                let Some(declaration) = surfaces.get(source).and_then(|s| s.get(original)).cloned()
                else {
                    continue;
                };
                if declaration.visibility != Visibility::Public {
                    continue;
                }
                added |= insert(&mut surfaces, module, local, declaration)?;
            }
        }
        if !added {
            break;
        }
    }
    let ids: OrderedMap<_, _> = surfaces
        .keys()
        .map(|module| (module.clone(), allocate_id(&ModulePath::from_str(module))))
        .collect();
    let mut modules = List::new();
    for (module, surface) in surfaces {
        let id = *ids.get(&module).expect("every projected module has an ID");
        let file_id = FileId::new(0);
        let ast = Module {
            items: List::new(),
            attributes: List::new(),
            file_id,
            span: Span::dummy(),
        };
        let mut info =
            ModuleInfo::new(id, ModulePath::from_str(&module), ast, file_id, Text::new());
        info.exports.set_module_id(id);
        info.exports.set_module_path(info.path.clone());
        for (name, declaration) in surface {
            let source = *ids
                .get(&declaration.owner)
                .expect("export owners belong to the projected module set");
            let export = if declaration.owner != module || name != declaration.original {
                ExportedItem::with_rename(
                    name,
                    declaration.original,
                    declaration.kind,
                    declaration.visibility,
                    source,
                    Span::dummy(),
                )
            } else {
                ExportedItem::new(
                    name,
                    declaration.kind,
                    declaration.visibility,
                    source,
                    Span::dummy(),
                )
            };
            info.exports.add_export(export)?;
        }
        modules.push(info);
    }
    Ok(modules)
}
