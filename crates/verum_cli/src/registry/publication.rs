//! Source-cog publication framing. Spec: Cog Publication Protocol v1.

use super::types::{CogMetadata, DependencySpec};
use crate::error::{CliError, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use verum_common::{List, Map, Text};

pub const PUBLICATION_CONTENT_TYPE: &str = "application/vnd.verum.cog-publish.v1";
pub const MAX_METADATA_BYTES: usize = 1024 * 1024;
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const ENVELOPE_OVERHEAD: usize = 4 + 8;

/// Local admission limits, applied before the HTTP request is sent.
/// Private fields ensure a client cannot install an unchecked configuration.
#[derive(Debug, Clone, Copy)]
pub struct PublicationLimits {
    max_metadata_bytes: usize,
    max_archive_bytes: usize,
}

impl PublicationLimits {
    pub fn new(max_metadata_bytes: usize, max_archive_bytes: usize) -> Result<Self> {
        if !(1..=MAX_METADATA_BYTES).contains(&max_metadata_bytes)
            || !(1..=MAX_ARCHIVE_BYTES).contains(&max_archive_bytes)
        {
            return Err(CliError::InvalidArgument(format!(
                "Publication limits must be metadata 1..={MAX_METADATA_BYTES} bytes and archive 1..={MAX_ARCHIVE_BYTES} bytes"
            )));
        }
        max_metadata_bytes
            .checked_add(max_archive_bytes)
            .and_then(|length| length.checked_add(ENVELOPE_OVERHEAD))
            .ok_or_else(|| {
                CliError::InvalidArgument("Publication envelope limit overflow".into())
            })?;
        Ok(Self {
            max_metadata_bytes,
            max_archive_bytes,
        })
    }
}

impl Default for PublicationLimits {
    fn default() -> Self {
        Self {
            max_metadata_bytes: 256 * 1024,
            max_archive_bytes: MAX_ARCHIVE_BYTES,
        }
    }
}

/// Only publisher-supplied source metadata belongs in a publication request.
/// Registry timestamps, authority and verification verdicts are not inputs.
#[derive(Serialize)]
struct PublicationMetadata<'a> {
    name: &'a Text,
    version: &'a Text,
    description: &'a Option<Text>,
    authors: &'a List<Text>,
    license: &'a Option<Text>,
    repository: &'a Option<Text>,
    homepage: &'a Option<Text>,
    keywords: &'a List<Text>,
    categories: &'a List<Text>,
    readme: &'a Option<Text>,
    dependencies: &'a Map<Text, DependencySpec>,
    features: &'a Map<Text, List<Text>>,
    checksum: &'a Text,
}

/// Encode lengths and the exact archive, checking its digest before transmission.
/// JSON object ordering is deliberately not a canonicalization or signature rule.
pub(super) fn encode_publication(
    metadata: &CogMetadata,
    archive_path: &Path,
    limits: PublicationLimits,
) -> Result<List<u8>> {
    validate_metadata(metadata)?;
    let source = PublicationMetadata {
        name: &metadata.name,
        version: &metadata.version,
        description: &metadata.description,
        authors: &metadata.authors,
        license: &metadata.license,
        repository: &metadata.repository,
        homepage: &metadata.homepage,
        keywords: &metadata.keywords,
        categories: &metadata.categories,
        readme: &metadata.readme,
        dependencies: &metadata.dependencies,
        features: &metadata.features,
        checksum: &metadata.checksum,
    };
    let mut json = BoundedMetadata {
        bytes: List::new(),
        limit: limits.max_metadata_bytes,
    };
    serde_json::to_writer(&mut json, &source)
        .map_err(|error| CliError::Registry(format!("Invalid publication metadata: {error}")))?;
    let metadata_length = u32::try_from(json.bytes.len())
        .map_err(|_| CliError::Registry("Publication metadata length exceeds u32".into()))?;

    let mut archive = List::new();
    let mut file = std::fs::File::open(archive_path)?;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let length = archive
            .len()
            .checked_add(count)
            .filter(|length| *length <= limits.max_archive_bytes)
            .ok_or_else(|| {
                CliError::Registry(format!(
                    "Publication archive exceeds configured limit of {} bytes",
                    limits.max_archive_bytes
                ))
            })?;
        archive.extend_from_slice(&buffer[..count]);
        debug_assert_eq!(archive.len(), length);
    }
    if archive.is_empty() {
        return Err(CliError::Registry("Publication archive is empty".into()));
    }
    let checksum = hex::encode(Sha256::digest(archive.as_slice()));
    if checksum != metadata.checksum.as_str() {
        return Err(CliError::Registry(format!(
            "Publication archive checksum mismatch: expected {}, computed {checksum}",
            metadata.checksum
        )));
    }
    let archive_length = u64::try_from(archive.len())
        .map_err(|_| CliError::Registry("Publication archive length exceeds u64".into()))?;
    let total_length = json
        .bytes
        .len()
        .checked_add(archive.len())
        .and_then(|length| length.checked_add(ENVELOPE_OVERHEAD))
        .ok_or_else(|| CliError::Registry("Publication envelope length overflow".into()))?;
    let mut body = List::with_capacity(total_length);
    body.extend_from_slice(&metadata_length.to_le_bytes());
    body.extend_from_slice(json.bytes.as_slice());
    body.extend_from_slice(&archive_length.to_le_bytes());
    body.extend_from_slice(archive.as_slice());
    Ok(body)
}

fn validate_metadata(metadata: &CogMetadata) -> Result<()> {
    if !crate::config::is_valid_cog_name(metadata.name.as_str()) {
        return Err(CliError::Registry(
            "Invalid publication metadata: cog name".into(),
        ));
    }
    if semver::Version::parse(metadata.version.as_str()).is_err() {
        return Err(CliError::Registry(
            "Invalid publication metadata: version".into(),
        ));
    }
    if metadata.checksum.len() != 64
        || !metadata
            .checksum
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(CliError::Registry(
            "Invalid publication metadata: checksum must be lowercase SHA-256 hex".into(),
        ));
    }
    if metadata.signature.is_some()
        || metadata.proofs.is_some()
        || metadata.cbgr_profiles.is_some()
        || metadata.ipfs_hash.is_some()
        || metadata.artifacts.tier0.is_some()
        || metadata.artifacts.tier1.is_some()
        || metadata.artifacts.tier2.is_some()
        || metadata.artifacts.tier3.is_some()
    {
        return Err(CliError::Registry(
            "Publication v1 accepts source metadata only; signature, proof, profile and derived artifact claims are unsupported".into(),
        ));
    }
    Ok(())
}

/// Stop serialization at the configured bound instead of allocating an
/// unbounded JSON value and checking its size afterwards.
struct BoundedMetadata {
    bytes: List<u8>,
    limit: usize,
}

impl Write for BoundedMetadata {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > self.limit)
        {
            return Err(std::io::Error::other("metadata exceeds configured limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
