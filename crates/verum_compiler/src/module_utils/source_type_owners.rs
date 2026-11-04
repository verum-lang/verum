//! File-owned names shared by the bootstrap's forward registries.
use std::collections::{HashMap, HashSet};
use verum_ast::{ItemKind, Module};

pub(crate) struct SourceTypeAlias {
    pub qualified_name: String,
    pub local_name: String,
    pub target: String,
}

pub(crate) struct SourceTypeNames {
    pub ambiguous: HashSet<String>,
    pub aliases: Vec<SourceTypeAlias>,
}

/// Sources carry their resolved file owner, independently of archive grouping.
/// Alias-vs-marker classification sees only the declaring file's lexical scope;
/// a bare runtime representation hint is usable only for one declaring owner.
pub(crate) fn source_type_names<'a>(
    sources: impl IntoIterator<Item = (&'a str, &'a Module)>,
) -> SourceTypeNames {
    let mut declared_in = HashMap::<&str, &str>::new();
    let mut names = SourceTypeNames {
        ambiguous: HashSet::new(),
        aliases: Vec::new(),
    };
    for (owner, module) in sources {
        let scope = verum_ast::decl::locally_visible_type_names(&module.items);
        for item in &module.items {
            let ItemKind::Type(declaration) = &item.kind else {
                continue;
            };
            let name = declaration.name.name.as_str();
            if let Some(previous) = declared_in.insert(name, owner)
                && previous != owner
            {
                names.ambiguous.insert(name.to_owned());
            }
            if let Some(target) =
                verum_vbc::codegen::VbcCodegen::detect_variant_form_alias(declaration, &scope)
            {
                names.aliases.push(SourceTypeAlias {
                    qualified_name: verum_vbc::module::qualify_module_name(owner, name),
                    local_name: name.to_owned(),
                    target,
                });
            }
        }
    }
    names
}

/// Merge forward or completed aliases using the same ownership policy.
/// Ambiguous short names cannot regain authority through a later merge; full
/// declaration keys remain independent and unambiguous compatibility survives.
pub(crate) fn merge_type_aliases(
    registry: &mut HashMap<String, String>,
    incoming: impl IntoIterator<Item = (String, String)>,
    ambiguous: &HashSet<String>,
) {
    for name in ambiguous {
        registry.remove(name);
    }
    for (name, target) in incoming {
        if !ambiguous.contains(&name) {
            registry.entry(name).or_insert(target);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/source_type_owners.rs"]
mod tests;
