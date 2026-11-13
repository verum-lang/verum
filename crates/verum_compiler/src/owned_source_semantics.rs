//! Type-check each source in its lexical namespace, with an isolated local scope.

use verum_ast::cfg::{CfgEvaluator, TargetConfig};
use verum_ast::{ItemKind, Module};
use verum_common::{List, Map, Shared, Text};

pub(crate) type SourceRegistry = Shared<parking_lot::RwLock<ModuleRegistry>>;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

/// Register the actual source declarations used by local mount resolution.
pub(crate) fn source_registry<'a>(
    sources: impl IntoIterator<Item = (&'a str, &'a str, &'a Module)>,
) -> anyhow::Result<SourceRegistry> {
    let registry = Shared::new(parking_lot::RwLock::new(ModuleRegistry::new()));
    register_sources(&registry, sources)?;
    Ok(registry)
}

pub(crate) fn register_sources<'a>(
    registry: &SourceRegistry,
    sources: impl IntoIterator<Item = (&'a str, &'a str, &'a Module)>,
) -> anyhow::Result<()> {
    let sources: List<_> = sources.into_iter().collect();
    let mut inline = List::new();
    for (owner, path, module) in &sources {
        collect_inline_sources(owner, path, module, &mut inline);
    }
    let sources: List<_> = sources
        .into_iter()
        .chain(
            inline
                .iter()
                .map(|(owner, path, module)| (owner.as_str(), path.as_str(), module)),
        )
        .collect();
    register_source_entries(registry, &sources)?;
    let mut registry = registry.write();
    verum_modules::resolve_specific_reexport_kinds(&mut registry)?;
    verum_modules::resolve_glob_reexports(&mut registry)?;
    Ok(())
}

/// Inline bodies participate in the same declaration graph as source files.
/// Ordinary loading and original-source admission share their ownership rules.
pub(crate) fn register_inline_sources(
    registry: &SourceRegistry,
    parent: &str,
    module: &Module,
) -> anyhow::Result<()> {
    let mut inline = List::new();
    collect_inline_sources(parent, parent, module, &mut inline);
    let sources: List<_> = inline
        .iter()
        .map(|(owner, path, module)| (owner.as_str(), path.as_str(), module))
        .collect();
    register_source_entries(registry, &sources)
}
fn collect_inline_sources(
    owner: &str,
    path: &str,
    module: &Module,
    out: &mut List<(Text, Text, Module)>,
) {
    for item in &module.items {
        if let ItemKind::Module(declaration) = &item.kind
            && let Some(items) = &declaration.items
        {
            let owner: Text = ModulePath::from_str(owner)
                .join(declaration.name.as_str())
                .to_string()
                .into();
            let child = Module {
                items: items.clone(),
                attributes: item.attributes.clone(),
                file_id: module.file_id,
                span: item.span,
            };
            collect_inline_sources(owner.as_str(), path, &child, out);
            out.push((owner, path.into(), child));
        }
    }
}
fn register_source_entries(
    registry: &SourceRegistry,
    sources: &[(&str, &str, &Module)],
) -> anyhow::Result<()> {
    let sources = unique_source_entries(sources)?;
    let mut registry = registry.write();
    for (owner, source, module) in sources {
        let id = if let Some(existing) = registry.get_by_path(owner) {
            anyhow::ensure!(
                existing.ast == *module || existing.ast.items.is_empty(),
                "source {source} conflicts with existing declarations for owner {owner}"
            );
            // Completing a forward module preserves its identity, including
            // every exported declaration's source_module reference.
            existing.id
        } else {
            registry.allocate_id()
        };
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            module.clone(),
            module.file_id,
            Text::new(),
        );
        info.exports = extract_exports_from_module(module, id, &path)?;
        registry.register(info);
    }
    Ok(())
}

/// Multiple distinct source parts need an explicit merge contract. A registry's
/// first-registration-wins behavior must not silently decide their declarations.
fn unique_source_entries<'a>(
    sources: &[(&'a str, &'a str, &'a Module)],
) -> anyhow::Result<List<(&'a str, &'a str, &'a Module)>> {
    let mut seen = Map::with_capacity(sources.len());
    let mut entries = List::with_capacity(sources.len());
    for &(owner, source, module) in sources {
        if let Some(&(previous_source, previous)) = seen.get(&owner) {
            anyhow::ensure!(
                previous_source == source && previous == module,
                "conflicting source parts for owner {owner}: {previous_source} and {source}"
            );
        } else {
            seen.insert(owner, (source, module));
            entries.push((owner, source, module));
        }
    }
    Ok(entries)
}

/// Apply file and ancestor restrictions before registering declarations or checking bodies.
pub(crate) fn select_source_roster(
    sources: &[(&str, &str, &Module)],
    target: &TargetConfig,
) -> anyhow::Result<List<(Text, Text, Module)>> {
    let sources = unique_source_entries(sources)?;
    let evaluator = CfgEvaluator::with_config(target.clone());
    let mut selections = Map::with_capacity(sources.len());
    for &(owner, _, module) in &sources {
        let selection = evaluator
            .source_module_selection(module, owner)
            .map_err(|span| anyhow::anyhow!("invalid module @cfg in {owner} at {span:?}"))?;
        selections.insert(owner, selection);
    }
    let mut selected = List::with_capacity(sources.len());
    for (owner, path, module) in sources {
        let mut enabled = selections[&owner].include;
        let mut ancestor = owner;
        while enabled && let Some((parent, _)) = ancestor.rsplit_once('.') {
            if let Some(selection) = selections.get(&parent) {
                enabled = selection.include && selection.includes_child(&owner[parent.len() + 1..]);
            }
            ancestor = parent;
        }
        if enabled {
            let mut module = module.clone();
            select_members(&mut module.items, &evaluator)
                .map_err(|span| anyhow::anyhow!("invalid @cfg in {owner} ({path}) at {span:?}"))?;
            selected.push((owner.into(), path.into(), module));
        }
    }
    Ok(selected)
}

/// The caller provides a fresh checker with its language and backend policy.
/// Registry declarations retain their owner; only mounts populate local names.
/// Recovered/deferred errors are failures, just like direct `check_item` errors.
#[cfg(test)]
pub(crate) fn check_source_module(
    checker: &mut TypeChecker,
    owner: &str,
    module: &Module,
    registry: &SourceRegistry,
) -> Result<(), List<verum_diagnostics::Diagnostic>> {
    check_source_types(checker, owner, module, registry, verum_types::TypeError::to_diagnostic)
}

/// Keep original spans until the session responsible for parsing renders them.
pub(crate) fn check_source_types(
    checker: &mut TypeChecker,
    owner: &str,
    module: &Module,
    registry: &SourceRegistry,
    render: impl Fn(&verum_types::TypeError) -> verum_diagnostics::Diagnostic,
) -> Result<(), List<verum_diagnostics::Diagnostic>> {
    let render = |error: &verum_types::TypeError| {
        let diagnostic = render(error);
        match enclosing_function(module, error.span()) {
            Some(name) => diagnostic.add_note(format!("while checking function `{name}`")),
            None => diagnostic,
        }
    };
    checker.set_current_module_path(owner);
    checker.set_module_registry(registry.clone());
    checker.register_stdlib_types_for_module(module);

    // A file header names this scope. It is not a nested module declaration.
    // Spans and all body/declaration nodes remain those of the original source.
    let mut body = module.clone();
    // A minimal source graph may have no standard library. When it does,
    // use its actual prelude and the shared opt-out/idempotence policy.
    // Written mounts still undergo ordinary resolution, including failures.
    if verum_ast::prelude::implicit_prelude_path(&body)
        .is_some_and(|path| registry.read().get_by_path(path).is_some())
    {
        verum_ast::prelude::inject_implicit_prelude_mount(&mut body);
    }
    if let Some(header) = body.file_header_index() {
        body.items.remove(header);
    }
    let mut errors: List<_> = checker.prepare_module_declarations(&body).iter().map(&render).collect();
    if errors.is_empty() {
        for item in &body.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(render(&error));
                // A nested checker may return before restoring its local scope.
                // Do not check another item under the failed item's authority.
                break;
            }
        }
    }
    errors.extend(checker.take_deferred_errors().iter().map(&render));
    errors.extend(checker.diagnostic_sources().iter().map(render));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Recover declaration context from the original source spans, before a
/// consumer converts them into editor or command-line locations. Do not infer
/// the enclosing function from diagnostic text or flattened declaration names.
fn enclosing_function(module: &Module, span: verum_ast::span::Span) -> Option<&str> {
    use verum_ast::decl::{ImplItemKind, ProtocolItemKind, TypeDeclBody};

    if span.is_empty() {
        return None;
    }
    let mut pending: List<_> = module.items.iter().collect();
    while let Some(item) = pending.pop() {
        if !item.span.contains(span) {
            continue;
        }
        let protocol_items = match &item.kind {
            ItemKind::Function(function) => return Some(function.name.as_str()),
            ItemKind::Module(module) => {
                if let Some(items) = &module.items {
                    pending.extend(items.iter());
                }
                None
            }
            ItemKind::Impl(implementation) => {
                for item in &implementation.items {
                    if item.span.contains(span)
                        && let ImplItemKind::Function(function) = &item.kind
                    {
                        return Some(function.name.as_str());
                    }
                }
                None
            }
            ItemKind::Protocol(protocol) => Some(&protocol.items),
            ItemKind::Type(declaration) => match &declaration.body {
                TypeDeclBody::Protocol(protocol) => Some(&protocol.items),
                _ => None,
            },
            _ => None,
        };
        if let Some(items) = protocol_items {
            for item in items {
                if item.span.contains(span)
                    && let ProtocolItemKind::Function { decl, .. } = &item.kind
                {
                    return Some(decl.name.as_str());
                }
            }
        }
    }
    None
}

/// Project a list in place, preserving order and the allocation. On error the
/// caller discards the entire unpublished roster.
fn select_in_place<T>(
    items: &mut List<T>,
    mut include: impl FnMut(&mut T) -> Result<bool, verum_ast::Span>,
) -> Result<(), verum_ast::Span> {
    let mut result = Ok(());
    items.retain_mut(|item| {
        if result.is_err() {
            return true;
        }
        match include(item) {
            Ok(enabled) => enabled,
            Err(span) => {
                result = Err(span);
                true
            }
        }
    });
    result
}

fn select_members(
    items: &mut List<verum_ast::Item>,
    evaluator: &CfgEvaluator,
) -> Result<(), verum_ast::Span> {
    use verum_ast::decl::{ImplItemKind, ProtocolItem, ProtocolItemKind, RecordField, TypeDeclBody, VariantData};

    let select_fields = |fields: &mut List<RecordField>| {
        select_in_place(fields, |field| evaluator.checked_attributes(&field.attributes))
    };
    let select_protocol = |items: &mut List<ProtocolItem>| {
        select_in_place(items, |item| match &item.kind {
            ProtocolItemKind::Function { decl, .. } => evaluator.checked_attributes(&decl.attributes),
            _ => Ok(true),
        })
    };

    select_in_place(items, |item| {
        let mut enabled = evaluator.checked_attributes(&item.attributes)?;
        match &mut item.kind {
            ItemKind::Module(module) => {
                if let Some(children) = &mut module.items {
                    select_members(children, evaluator)?;
                }
            }
            ItemKind::Function(function) => enabled &= evaluator.checked_attributes(&function.attributes)?,
            ItemKind::Type(declaration) => {
                enabled &= evaluator.checked_attributes(&declaration.attributes)?;
                match &mut declaration.body {
                    TypeDeclBody::Record(fields) => select_fields(fields)?,
                    TypeDeclBody::Variant(cases) | TypeDeclBody::Inductive(cases) => select_in_place(cases, |case| {
                        let keep = evaluator.checked_attributes(&case.attributes)?;
                        if let Some(VariantData::Record(fields)) = &mut case.data {
                            select_fields(fields)?;
                        }
                        Ok(keep)
                    })?,
                    TypeDeclBody::Protocol(protocol) | TypeDeclBody::Coinductive(protocol) => {
                        select_protocol(&mut protocol.items)?
                    }
                    _ => {}
                }
            }
            ItemKind::Protocol(protocol) => select_protocol(&mut protocol.items)?,
            ItemKind::Impl(declaration) => select_in_place(&mut declaration.items, |member| {
                let mut keep = evaluator.checked_attributes(&member.attributes)?;
                if let ImplItemKind::Function(function) = &member.kind {
                    keep &= evaluator.checked_attributes(&function.attributes)?;
                }
                Ok(keep)
            })?,
            _ => {}
        }
        Ok(enabled)
    })
}

pub(crate) fn declared_owner(module: &Module, fallback: &str) -> Text {
    verum_vbc::codegen::VbcCodegen::resolve_full_module_path(module, fallback)
        .map(Into::into)
        .unwrap_or_else(|| fallback.into())
}

#[cfg(test)]
#[path = "../tests/unit/owned_source_semantics.rs"]
mod tests;
