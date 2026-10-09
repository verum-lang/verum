//! Server acknowledgement for source-cog publication.

use super::types::CogMetadata;
use crate::error::{CliError, Result};
use reqwest::StatusCode;
use reqwest::blocking::Response;
use serde::Deserialize;
use std::io::Read;
use verum_common::{List, Text};

const MAX_RECEIPT_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationReceipt {
    name: Text,
    version: Text,
    checksum: Text,
}

pub(super) fn verify_publication_receipt(
    mut response: Response,
    metadata: &CogMetadata,
) -> Result<()> {
    if !matches!(response.status(), StatusCode::OK | StatusCode::CREATED) {
        return Err(CliError::Registry(format!(
            "Publish failed: {} (no publication receipt)",
            response.status()
        )));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let mut parts = content_type.split(';');
    let is_json = parts
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"));
    let supported_parameters = parts.next().is_none_or(|parameter| {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            return false;
        };
        name.trim().eq_ignore_ascii_case("charset")
            && (value.trim().eq_ignore_ascii_case("utf-8")
                || value.trim().eq_ignore_ascii_case("\"utf-8\""))
    }) && parts.next().is_none();
    if !is_json || !supported_parameters {
        return Err(CliError::Registry(
            "Invalid publication receipt: expected application/json with optional UTF-8 charset"
                .into(),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RECEIPT_BYTES as u64)
    {
        return Err(CliError::Registry(
            "Publication receipt exceeds 65536-byte limit".into(),
        ));
    }
    let mut bytes = List::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = response.read(&mut buffer).map_err(|error| {
            CliError::Network(format!("Failed to read publication receipt: {error}"))
        })?;
        if count == 0 {
            break;
        }
        if bytes
            .len()
            .checked_add(count)
            .is_none_or(|length| length > MAX_RECEIPT_BYTES)
        {
            return Err(CliError::Registry(
                "Publication receipt exceeds 65536-byte limit".into(),
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    // A typed deserializer rejects duplicate/unknown fields and trailing JSON.
    // Parsing into a generic map first would lose duplicate-key information.
    let receipt: PublicationReceipt = serde_json::from_slice(bytes.as_slice())
        .map_err(|error| CliError::Registry(format!("Invalid publication receipt: {error}")))?;
    if receipt.name != metadata.name
        || receipt.version != metadata.version
        || receipt.checksum != metadata.checksum
    {
        return Err(CliError::Registry(
            "Publication receipt does not match requested name, version and archive checksum"
                .into(),
        ));
    }
    Ok(())
}
