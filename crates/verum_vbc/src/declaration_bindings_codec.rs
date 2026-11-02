//! Bounded, deterministic lexical binding environments for semantic carriers.
use crate::error::{VbcError, VbcResult};
use verum_common::{OrderedMap, Text};

pub(crate) type Bindings = OrderedMap<Text, OrderedMap<Text, Option<Text>>>;
pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;
type Wire = Vec<(Text, Vec<(Text, Option<Text>)>)>;

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
    // Encoding and decoding accept the same language, including name validity.
    decode(&bytes)?;
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
        if owner.is_empty() {
            return Err(invalid("empty declaration owner"));
        }
        let mut names = OrderedMap::new();
        for (name, target) in entries {
            if name.is_empty() || target.as_ref().is_some_and(Text::is_empty) {
                return Err(invalid("empty binding name or target"));
            }
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
