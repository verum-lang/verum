//! Source callable ownership, independent of linkage names and bytecode bases.
use crate::module::{FunctionDescriptor, VbcModule};
use verum_common::{Map, Text};

/// Validate the typed lexical graph. Legacy descriptors without this carrier do
/// not acquire source identity from their linkage spelling. Parent IDs are
/// absolute IDs in the containing module, never relative to `func_id_base`.
pub fn validate_module(module: &VbcModule) -> Result<(), Text> {
    let mut functions: Map<u32, &FunctionDescriptor> = Map::new();
    for function in &module.functions {
        if let Some(previous) = functions.insert(function.id.0, function)
            && (previous.declaration_identity.is_some() || function.declaration_identity.is_some())
        {
            return Err("duplicate callable ID in declaration graph".into());
        }
    }
    for function in &module.functions {
        let Some(identity) = &function.declaration_identity else { continue; };
        let signature = function.source_signature.as_ref()
            .ok_or_else(|| Text::from("callable declaration identity has no source signature"))?;
        let name = module.strings.get(identity.source_name)
            .ok_or_else(|| Text::from("callable declaration identity has invalid source name"))?;
        if name != signature.name.name.as_str() || name.is_empty()
            || signature.params.len() != function.params.len()
        {
            return Err("callable declaration identity disagrees with source signature".into());
        }
        let owner = declaring_module(module, function)?;
        let mut parent = identity.lexical_parent;
        let mut depth = 0;
        while let Some(id) = parent {
            depth += 1;
            if depth > 128 || id == function.id {
                return Err("cyclic or excessive callable lexical scope".into());
            }
            let ancestor = functions.get(&id.0)
                .ok_or_else(|| Text::from("dangling callable lexical parent"))?;
            if declaring_module(module, ancestor)? != owner {
                return Err("callable lexical parent belongs to another source module".into());
            }
            let ancestor_identity = ancestor.declaration_identity.as_ref()
                .ok_or_else(|| Text::from("callable lexical parent has no declaration identity"))?;
            if ancestor.source_signature.is_none() {
                return Err("callable lexical parent has no source signature".into());
            }
            parent = ancestor_identity.lexical_parent;
        }
    }
    Ok(())
}

fn declaring_module<'a>(module: &'a VbcModule, function: &FunctionDescriptor) -> Result<&'a str, Text> {
    match function.origin_module {
        Some(owner) => module.strings.get(owner)
            .ok_or_else(|| Text::from("callable declaration has invalid source module")),
        None => Ok(module.name.as_str()),
    }
}
