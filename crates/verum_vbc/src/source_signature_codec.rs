//! Bounded declaration-only AST carriers for VBC 2.30 and 2.31.

use crate::error::{VbcError, VbcResult};
use serde::{Serialize, de::DeserializeOwned};
use verum_ast::{FunctionDecl, TypeDecl};
use verum_common::Text;

pub(crate) const MAX_SOURCE_SIGNATURE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_INHERENT_IMPLEMENTATIONS: usize = 4096;

fn encode_payload<T: Serialize + DeserializeOwned>(
    declaration: &T,
    validate: impl Fn(&T) -> Result<(), Text> + Copy,
) -> VbcResult<Vec<u8>> {
    validate(declaration).map_err(|message| VbcError::Serialization(message.to_string()))?;
    let bytes = serde_json::to_vec(declaration)
        .map_err(|error| VbcError::Serialization(error.to_string()))?;
    if bytes.len() > MAX_SOURCE_SIGNATURE_BYTES {
        return Err(VbcError::Serialization(
            "source declaration exceeds byte budget".into(),
        ));
    }
    // Writers must produce a payload the bounded reader can accept.
    decode_payload::<T>(&bytes, validate)
        .map_err(|error| VbcError::Serialization(error.to_string()))?;
    Ok(bytes)
}

fn decode_payload<T: DeserializeOwned>(
    bytes: &[u8],
    validate: impl Fn(&T) -> Result<(), Text>,
) -> VbcResult<T> {
    if bytes.len() > MAX_SOURCE_SIGNATURE_BYTES {
        return Err(VbcError::Deserialization(
            "source declaration exceeds byte budget".into(),
        ));
    }
    // Keep the JSON recursion limit enabled before constructing the typed AST.
    let declaration = serde_json::from_slice(bytes)
        .map_err(|error| VbcError::Deserialization(error.to_string()))?;
    validate(&declaration).map_err(|message| VbcError::Deserialization(message.to_string()))?;
    Ok(declaration)
}

pub(crate) fn encode(signature: &FunctionDecl, is_method: bool) -> VbcResult<Vec<u8>> {
    encode_payload(signature, |signature| {
        verum_ast::source_signature::validate_source_signature(signature, is_method)
    })
}

fn check_constraint_version(
    minor: u16,
    requires_distinction: impl FnOnce() -> Result<bool, Text>,
) -> VbcResult<()> {
    if minor < crate::format::SOURCE_CONSTRAINT_CLASSIFICATION_VERSION
        && requires_distinction().map_err(|error| VbcError::Deserialization(error.to_string()))?
    {
        return Err(VbcError::Deserialization(
            "source constraint classification are ambiguous before VBC 2.64; rebuild from source"
                .into(),
        ));
    }
    Ok(())
}

pub(crate) fn decode(bytes: &[u8], is_method: bool, minor: u16) -> VbcResult<FunctionDecl> {
    let signature = decode_payload(bytes, |signature| {
        verum_ast::source_signature::validate_source_signature(signature, is_method)
    })?;
    check_constraint_version(minor, || {
        verum_ast::source_signature::signature_requires_constraint_classification(&signature)
    })?;
    Ok(signature)
}

pub(crate) fn encode_type(declaration: &TypeDecl) -> VbcResult<Vec<u8>> {
    encode_payload(
        declaration,
        verum_ast::source_signature::validate_source_type_declaration,
    )
}

pub(crate) fn decode_type(bytes: &[u8], minor: u16) -> VbcResult<TypeDecl> {
    let declaration = decode_payload(
        bytes,
        verum_ast::source_signature::validate_source_type_declaration,
    )?;
    check_constraint_version(minor, || {
        verum_ast::source_signature::type_declaration_requires_constraint_classification(
            &declaration,
        )
    })?;
    Ok(declaration)
}

/// The shared source carrier validates both syntax and bounded decoding.
pub(crate) fn encode_implementation(
    source: &verum_ast::source_implementation::AssociatedImplementationSource,
) -> VbcResult<Vec<u8>> {
    encode_payload(source, |source| source.declaration().map(|_| ()))
}
pub(crate) fn decode_implementation(
    bytes: &[u8],
    minor: u16,
) -> VbcResult<verum_ast::source_implementation::AssociatedImplementationSource> {
    let source = decode_payload(
        bytes,
        |source: &verum_ast::source_implementation::AssociatedImplementationSource| {
            source.declaration().map(|_| ())
        },
    )?;
    check_constraint_version(minor, || {
        verum_ast::source_signature::implementation_requires_constraint_classification(
            &source.declaration()?,
        )
    })?;
    Ok(source)
}
