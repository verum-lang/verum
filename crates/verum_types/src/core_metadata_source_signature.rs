//! Bounded wire adapter for the typed declaration carrier inside a sidecar.
use serde::{Deserialize, Deserializer, Serializer, de::Error as _, ser::Error as _};
use verum_ast::FunctionDecl;
use verum_common::Text;

const MAX_SIGNATURE_BYTES: usize = 1024 * 1024;

fn parse(payload: &str) -> Result<FunctionDecl, Text> {
    if payload.len() > MAX_SIGNATURE_BYTES {
        return Err("metadata source signature exceeds its byte budget".into());
    }
    // JSON's recursion guard remains enabled, before constructing the AST.
    // A later validator alone cannot protect an unbounded binary AST decode.
    let signature: FunctionDecl = serde_json::from_str(payload)
        .map_err(|error| Text::from(format!("invalid metadata source signature: {error}")))?;
    verum_ast::source_signature::validate_source_signature(&signature, false)?;
    Ok(signature)
}

pub(super) fn serialize<S: Serializer>(
    signature: &Option<FunctionDecl>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let Some(signature) = signature else {
        return serializer.serialize_none();
    };
    verum_ast::source_signature::validate_source_signature(signature, false)
        .map_err(S::Error::custom)?;
    let payload = serde_json::to_string(signature).map_err(S::Error::custom)?;
    // A successful writer must produce a payload its bounded reader accepts.
    parse(&payload).map_err(S::Error::custom)?;
    serializer.serialize_some(&payload)
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<FunctionDecl>, D::Error> {
    let payload = Option::<Text>::deserialize(deserializer)?;
    payload
        .as_ref()
        .map(|payload| parse(payload).map_err(D::Error::custom))
        .transpose()
}
