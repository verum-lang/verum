//! Bounded declaration-only AST carrier for VBC 2.30.

use crate::error::{VbcError, VbcResult};
use verum_ast::FunctionDecl;

pub(crate) const MAX_SOURCE_SIGNATURE_BYTES: usize = 1024 * 1024;

pub(crate) fn encode(signature: &FunctionDecl, is_method: bool) -> VbcResult<Vec<u8>> {
    verum_ast::source_signature::validate_source_signature(signature, is_method)
        .map_err(|message| VbcError::Serialization(message.to_string()))?;
    let bytes = serde_json::to_vec(signature)
        .map_err(|error| VbcError::Serialization(error.to_string()))?;
    if bytes.len() > MAX_SOURCE_SIGNATURE_BYTES {
        return Err(VbcError::Serialization(
            "source signature exceeds byte budget".into(),
        ));
    }
    decode(&bytes, is_method).map_err(|error| VbcError::Serialization(error.to_string()))?;
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8], is_method: bool) -> VbcResult<FunctionDecl> {
    if bytes.len() > MAX_SOURCE_SIGNATURE_BYTES {
        return Err(VbcError::Deserialization(
            "source signature exceeds byte budget".into(),
        ));
    }
    // Keep serde_json's recursion limit enabled: validation after a recursive
    // unbounded binary decode would be too late to protect the reader's stack.
    let signature: FunctionDecl = serde_json::from_slice(bytes)
        .map_err(|error| VbcError::Deserialization(error.to_string()))?;
    verum_ast::source_signature::validate_source_signature(&signature, is_method)
        .map_err(|message| VbcError::Deserialization(message.to_string()))?;
    Ok(signature)
}
