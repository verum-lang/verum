//! Bounded wire adapter for the typed declaration carrier inside a sidecar.
use serde::{Deserialize, Deserializer, Serializer, de::Error as _, ser::Error as _};
use verum_ast::{FunctionDecl, TypeDecl};
use verum_common::Text;

const MAX_SIGNATURE_BYTES: usize = 1024 * 1024;

pub(super) trait Declaration: serde::Serialize + serde::de::DeserializeOwned {
    fn validate(&self) -> Result<(), Text>;
}
impl Declaration for FunctionDecl {
    fn validate(&self) -> Result<(), Text> {
        verum_ast::source_signature::validate_source_signature(self, false)
    }
}
impl Declaration for TypeDecl {
    fn validate(&self) -> Result<(), Text> {
        verum_ast::source_signature::validate_source_type_declaration(self)
    }
}

fn parse<T: Declaration>(payload: &str) -> Result<T, Text> {
    if payload.len() > MAX_SIGNATURE_BYTES {
        return Err("metadata source signature exceeds its byte budget".into());
    }
    // JSON's recursion guard remains enabled, before constructing the AST.
    // A later validator alone cannot protect an unbounded binary AST decode.
    let signature: T = serde_json::from_str(payload)
        .map_err(|error| Text::from(format!("invalid metadata source signature: {error}")))?;
    signature.validate()?;
    Ok(signature)
}

pub(super) fn serialize<T: Declaration, S: Serializer>(
    signature: &Option<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let Some(signature) = signature else {
        return serializer.serialize_none();
    };
    signature.validate()
        .map_err(S::Error::custom)?;
    let payload = serde_json::to_string(signature).map_err(S::Error::custom)?;
    // A successful writer must produce a payload its bounded reader accepts.
    parse::<T>(&payload).map_err(S::Error::custom)?;
    serializer.serialize_some(&payload)
}

pub(super) fn deserialize<'de, T: Declaration, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    let payload = Option::<Text>::deserialize(deserializer)?;
    payload
        .as_ref()
        .map(|payload| parse(payload).map_err(D::Error::custom))
        .transpose()
}
