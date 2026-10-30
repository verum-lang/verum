//! Immutable declaration indexes tied to one exact metadata allocation.
//! Mutable checker registrations and cancellation state are never shared here.
use crate::{AnalysisControl, Result, TypeChecker, core_metadata::CoreMetadata};
use std::sync::Arc;
use verum_common::{List, Map, Set, Text};

pub(crate) type ImplementationIndex = Map<Text, List<usize>>;

pub struct CoreMetadataSnapshot {
    metadata: Arc<CoreMetadata>,
    pub(crate) implementations: Arc<ImplementationIndex>,
}

impl CoreMetadataSnapshot {
    /// Prepare transactionally. A cancelled build publishes no partial index;
    /// the preparation control is not retained by the completed snapshot.
    pub fn prepare(metadata: Arc<CoreMetadata>, control: &AnalysisControl) -> Result<Self> {
        let implementations = implementation_index(&metadata, Some(control), &mut 0)?;
        Ok(Self {
            metadata,
            implementations: Arc::new(implementations),
        })
    }

    pub fn metadata(&self) -> &Arc<CoreMetadata> {
        &self.metadata
    }
}

pub(crate) fn implementation_index(
    metadata: &CoreMetadata,
    control: Option<&AnalysisControl>,
    scanned: &mut usize,
) -> Result<ImplementationIndex> {
    if let Some(control) = control {
        control.check()?;
    }
    let mut index: ImplementationIndex = Map::new();
    let mut declarations = Set::new();
    for (position, implementation) in metadata.implementations.iter().enumerate() {
        if let Some(control) = control {
            control.check()?;
        }
        *scanned += 1;
        if !declarations.insert(implementation) {
            continue;
        }
        let owner = metadata
            .types
            .get(&implementation.target_type)
            .map(TypeChecker::metadata_declaring_key)
            .unwrap_or_else(|| implementation.target_type.clone());
        index.entry(owner).or_default().push(position);
    }
    Ok(index)
}
