//! Immutable release metadata shared by every language-service transport.
use anyhow::{Context, Result, ensure};
use bincode::Options;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, sync::Arc};
use verum_common::Text;
use verum_types::core_metadata::CoreMetadata;

const ARTIFACT_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct LibraryIdentity {
    pub sha256: Text,
    pub format_version: u32,
}

pub struct LibraryMetadata {
    pub(crate) snapshot: Arc<verum_types::CoreMetadataSnapshot>,
    pub(crate) registry: verum_modules::ModuleRegistry,
    pub identity: LibraryIdentity,
}

impl LibraryMetadata {
    /// Decode the same sidecar embedded in release compilers. Invalid or
    /// incompatible artifacts are errors, never a source-fallback trigger.
    pub fn decode(bytes: &[u8]) -> Result<Arc<Self>> {
        ensure!(!bytes.is_empty(), "Library metadata is empty");
        ensure!(
            bytes.len() as u64 <= ARTIFACT_LIMIT,
            "Library metadata exceeds size limit"
        );
        let core: CoreMetadata = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(ARTIFACT_LIMIT)
            .reject_trailing_bytes()
            .deserialize(bytes)
            .context("Invalid or incompatible library metadata")?;
        let mut registry = verum_modules::ModuleRegistry::new();
        for module in verum_types::module_projection::project_modules(&core, None, |_| {
            registry.allocate_id()
        })? {
            registry.register(module);
        }
        let core = Arc::new(core);
        let snapshot = Arc::new(verum_types::CoreMetadataSnapshot::prepare(
            core.clone(), &verum_types::AnalysisControl::default(),
        )?);
        Ok(Arc::new(Self {
            registry,
            identity: LibraryIdentity {
                sha256: hex::encode(Sha256::digest(bytes)).into(),
                format_version: core.version,
            },
            snapshot,
        }))
    }

    pub fn load(path: &Path) -> Result<Arc<Self>> {
        let file = std::fs::File::open(path).context("Open library metadata")?;
        let mut bytes = Vec::new();
        file.take(ARTIFACT_LIMIT + 1).read_to_end(&mut bytes)?;
        Self::decode(&bytes)
    }

    pub fn metadata(&self) -> &CoreMetadata {
        self.snapshot.metadata()
    }
}
