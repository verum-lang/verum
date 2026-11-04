//! The versioned source index is a complete, bounded package-version set.
//! Validate it before callers choose versions or change a project lockfile.

use super::types::{IndexEntry, VersionEntry};
use crate::error::{CliError, Result};
use reqwest::blocking::Response;
use reqwest::{StatusCode, Url};
use serde::de::value::MapAccessDeserializer;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::io::Read;
use std::marker::PhantomData;
use verum_common::{List, Map, Set, Text};

const MAX_INDEX_BYTES: usize = 1024 * 1024;
const MAX_INDEX_VERSIONS: usize = 1024;
const MAX_COORDINATE_BYTES: usize = 4096;

/// Match source-publication names, then encode exactly one URL path segment.
/// Appending segments retains a configured registry's complete path prefix.
pub(super) fn index_url(base: &str, name: &str) -> Result<Url> {
    if name.len() > MAX_COORDINATE_BYTES || !crate::config::is_valid_cog_name(name) {
        return Err(CliError::Registry("Invalid package index name".into()));
    }
    let mut url = Url::parse(base)
        .map_err(|error| CliError::Registry(format!("Invalid registry base URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(CliError::Registry(
            "Registry base URL must have an HTTP or HTTPS origin".into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(CliError::Registry(
            "Registry base URL must not contain a query or fragment".into(),
        ));
    }
    url.path_segments_mut()
        .map_err(|_| CliError::Registry("Registry base URL cannot contain path segments".into()))?
        .pop_if_empty()
        .extend(["api", "v1", "cogs", name, "index"]);
    Ok(url)
}

/// Cap bytes read from reqwest's response body, including chunked/close-delimited
/// responses and any content decoding performed by the configured HTTP client.
/// Content-Length is not an authority for either completeness or the byte limit.
pub(super) fn read_index(response: Response, name: &str) -> Result<IndexEntry> {
    // 206 is a successful HTTP status, but a partial version set can resolve a
    // constraint incorrectly. The v1 index contract has exactly one success code.
    if response.status() != StatusCode::OK {
        return Err(CliError::Registry(format!(
            "Package index request failed: {}",
            response.status()
        )));
    }
    let mut bytes = List::new();
    std::io::copy(&mut response.take((MAX_INDEX_BYTES + 1) as u64), &mut bytes)
        .map_err(|error| CliError::Network(format!("Failed to read package index: {error}")))?;
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(CliError::Registry("Package index exceeds 1 MiB".into()));
    }
    let JsonObject(wire): JsonObject<WireIndex> = serde_json::from_slice(&bytes)
        .map_err(|error| CliError::Registry(format!("Invalid package index: {error}")))?;
    if wire.name.as_str() != name {
        return Err(CliError::Registry(
            "Package index does not match the requested name".into(),
        ));
    }
    if wire.versions.is_empty() || wire.versions.len() > MAX_INDEX_VERSIONS {
        return Err(CliError::Registry(
            "Package index must contain between 1 and 1024 versions".into(),
        ));
    }
    {
        // Release coordinates retain build metadata. SemVer precedence equality
        // is not identity: 1.0.0+one and 1.0.0+two are distinct stored releases.
        let mut seen = Set::new();
        for JsonObject(version) in wire.versions.iter() {
            if version.version.len() > MAX_COORDINATE_BYTES
                || semver::Version::parse(version.version.as_str()).is_err()
            {
                return Err(CliError::Registry(
                    "Package index contains an invalid exact version".into(),
                ));
            }
            if !seen.insert(version.version.as_str()) {
                return Err(CliError::Registry(
                    "Package index contains duplicate versions".into(),
                ));
            }
            if version.checksum.len() != 64
                || !version
                    .checksum
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(CliError::Registry(
                    "Package index contains an invalid SHA-256 checksum".into(),
                ));
            }
        }
    }
    Ok(IndexEntry {
        name: wire.name,
        versions: wire
            .versions
            .into_iter()
            .map(|JsonObject(version)| VersionEntry {
                version: version.version,
                checksum: version.checksum,
                yanked: version.yanked,
                features: version.features,
            })
            .collect(),
    })
}

// Derived struct decoding rejects duplicate fields. Unknown fields are refused
// by this v1 wire contract, so they cannot hide ignored duplicate JSON objects.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIndex {
    name: Text,
    versions: List<JsonObject<WireVersion>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireVersion {
    version: Text,
    checksum: Text,
    yanked: bool,
    #[serde(deserialize_with = "features")]
    features: Map<Text, List<Text>>,
}

// Serde structs also accept a sequence representation. Require a JSON object
// before their derived visitors run; MapAccessDeserializer preserves duplicate
// fields for the derived decoder instead of first collapsing them into a map.
struct JsonObject<T>(T);

impl<'de, T> Deserialize<'de> for JsonObject<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Object<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for Object<T>
        where
            T: Deserialize<'de>,
        {
            type Value = JsonObject<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a package index object")
            }

            fn visit_map<A>(self, entries: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                T::deserialize(MapAccessDeserializer::new(entries)).map(JsonObject)
            }
        }

        deserializer.deserialize_map(Object::<T>(PhantomData))
    }
}

fn features<'de, D>(deserializer: D) -> std::result::Result<Map<Text, List<Text>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Features;

    impl<'de> Visitor<'de> for Features {
        type Value = Map<Text, List<Text>>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object mapping distinct feature names to string arrays")
        }

        fn visit_map<A>(self, mut entries: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut features = Map::new();
            while let Some(name) = entries.next_key::<Text>()? {
                if features.contains_key(&name) {
                    return Err(de::Error::custom("duplicate package index feature key"));
                }
                // Array order and duplicate member strings are source data.
                features.insert(name, entries.next_value::<List<Text>>()?);
            }
            Ok(features)
        }
    }

    deserializer.deserialize_map(Features)
}
