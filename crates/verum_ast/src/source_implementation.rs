//! A bounded, immutable source authority for an implementation's type families.
//! Equality and hashing use its canonical encoding; no unhashable AST field is
//! omitted from identity. Decoding remains typed and validated before replay.
use serde::{Deserialize, Deserializer, Serialize};
use crate::decl::ImplDecl;
use verum_common::Text;

const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct AssociatedImplementationSource {
    origin_module: Text,
    declaration: Text,
}

fn parse(payload: &str) -> Result<ImplDecl, Text> {
    if payload.len() > MAX_BYTES {
        return Err("associated implementation source exceeds its byte budget".into());
    }
    // Leave serde_json's recursion limit enabled before allocating the AST.
    let declaration: ImplDecl = serde_json::from_str(payload).map_err(|error| {
        Text::from(format!("invalid associated implementation source: {error}"))
    })?;
    crate::source_signature::validate_source_associated_implementation(&declaration)?;
    Ok(declaration)
}

impl AssociatedImplementationSource {
    /// Capture the real implementation header and only its associated type
    /// assignments. This is declaration evidence, not a conformance certificate.
    pub fn new(origin_module: Text, declaration: &ImplDecl) -> Result<Self, Text> {
        let declaration =
            crate::source_signature::source_associated_implementation(declaration)?;
        Self::from_retained(origin_module, &declaration)
    }

    pub fn new_with_items(origin_module: Text, declaration: &ImplDecl, items: &[&crate::decl::ImplItem]) -> Result<Self, Text> {
        let retained = crate::source_signature::source_associated_implementation_items(declaration, items)?;
        Self::from_retained(origin_module, &retained)
    }

    fn from_retained(origin_module: Text, declaration: &ImplDecl) -> Result<Self, Text> {
        if origin_module.is_empty() || origin_module.len() > MAX_BYTES {
            return Err("associated implementation source requires its actual module owner".into());
        }
        let payload =
            serde_json::to_string(declaration).map_err(|error| Text::from(error.to_string()))?;
        // Successful writers must produce payloads accepted by bounded readers.
        parse(&payload)?;
        Ok(Self {
            origin_module,
            declaration: payload.into(),
        })
    }

    pub fn origin_module(&self) -> &str {
        self.origin_module.as_str()
    }

    pub fn declaration(&self) -> Result<ImplDecl, Text> {
        parse(self.declaration.as_str())
    }
}

impl<'de> Deserialize<'de> for AssociatedImplementationSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Payload {
            origin_module: Text,
            declaration: Text,
        }
        let Payload {
            origin_module,
            declaration,
        } = Payload::deserialize(deserializer)?;
        let parsed = parse(declaration.as_str()).map_err(serde::de::Error::custom)?;
        // Normalize the encoding, retaining every typed field. Whitespace or key
        // ordering must not manufacture a distinct implementation identity.
        Self::from_retained(origin_module, &parsed).map_err(serde::de::Error::custom)
    }
}
