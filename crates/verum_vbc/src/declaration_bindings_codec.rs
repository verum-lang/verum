//! Bounded, deterministic lexical binding environments for semantic carriers.
use crate::error::{VbcError, VbcResult};
use verum_common::{OrderedMap, Text};

pub(crate) type Bindings = OrderedMap<Text, OrderedMap<Text, Option<Text>>>;
pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;
type Wire = Vec<(Text, Vec<(Text, Option<Text>)>)>;

/// An empty key denotes the anonymous root only when a retained declaration
/// explicitly selects that owner. A missing origin remains a legacy module fallback.
pub(crate) fn validate_module(module: &crate::module::VbcModule) -> Result<(), Text> {
    for (owner, names) in &module.declaration_type_bindings {
        if verum_common::qualified_name::has_empty_module_segment(owner) {
            return Err("malformed declaration owner".into());
        }
        if names
            .iter()
            .any(|(name, target)| name.is_empty() || target.as_ref().is_some_and(Text::is_empty))
        {
            return Err("empty binding name or target".into());
        }
    }
    if !module.declaration_type_bindings.contains_key(&Text::new()) {
        return Ok(());
    }
    let mut has_source = false;
    for ty in &module.types {
        has_source |= crate::module::declaration_type_scope_owners(ty, &module.name, |id| {
            module.strings.get(id)
        })?
        .contains(&Text::new());
    }
    for function in module
        .functions
        .iter()
        .filter(|f| f.source_signature.is_some())
    {
        has_source |=
            crate::module::declaration_origin(&module.name, function.origin_module, |id| {
                module.strings.get(id)
            })?
            .is_empty();
    }
    if !has_source {
        return Err("empty declaration owner has no explicit anonymous source declaration".into());
    }
    Ok(())
}

pub(crate) fn encode(bindings: &Bindings) -> VbcResult<Vec<u8>> {
    // Lists preserve duplicate evidence on decode, unlike serde's map visitor
    // which would silently replace an earlier owner or lexical binding.
    let wire: Wire = bindings
        .iter()
        .map(|(owner, names)| {
            (
                owner.clone(),
                names
                    .iter()
                    .map(|(name, path)| (name.clone(), path.clone()))
                    .collect(),
            )
        })
        .collect();
    let bytes = serde_json::to_vec(&wire)
        .map_err(|error| VbcError::Serialization(format!("declaration type bindings: {error}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(VbcError::Serialization(
            "declaration type bindings exceed byte budget".into(),
        ));
    }
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8]) -> VbcResult<Bindings> {
    let invalid =
        |message: &str| VbcError::Deserialization(format!("declaration type bindings: {message}"));
    if bytes.len() > MAX_BYTES {
        return Err(invalid("byte budget exceeded"));
    }
    let wire: Wire = serde_json::from_slice(bytes).map_err(|error| {
        VbcError::Deserialization(format!("declaration type bindings: {error}"))
    })?;
    let mut result = Bindings::new();
    for (owner, entries) in wire {
        let mut names = OrderedMap::new();
        for (name, target) in entries {
            if names.insert(name, target).is_some() {
                return Err(invalid("duplicate lexical binding"));
            }
        }
        if result.insert(owner, names).is_some() {
            return Err(invalid("duplicate declaration owner"));
        }
    }
    Ok(result)
}
