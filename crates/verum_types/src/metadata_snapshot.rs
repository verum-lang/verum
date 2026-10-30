//! Immutable declaration indexes tied to one exact metadata allocation.
//! Mutable checker registrations and cancellation state are never shared here.
use crate::{AnalysisControl, Result, TypeChecker, core_metadata::CoreMetadata};
use std::sync::Arc;
use verum_common::{List, Map, Set, Text};

pub(crate) type ImplementationIndex = Map<Text, List<usize>>;

pub struct CoreMetadataSnapshot {
    metadata: Arc<CoreMetadata>,
    pub(crate) implementations: Arc<ImplementationIndex>,
    pub(crate) names: Arc<MetadataNameIndex>,
}

impl CoreMetadataSnapshot {
    /// Prepare transactionally. A cancelled build publishes no partial index;
    /// the preparation control is not retained by the completed snapshot.
    pub fn prepare(metadata: Arc<CoreMetadata>, control: &AnalysisControl) -> Result<Self> {
        let implementations = implementation_index(&metadata, Some(control), &mut 0)?;
        let names = MetadataNameIndex::prepare(&metadata, Some(control))?;
        Ok(Self {
            metadata,
            implementations: Arc::new(implementations),
            names: Arc::new(names),
        })
    }

    pub fn metadata(&self) -> &Arc<CoreMetadata> {
        &self.metadata
    }
}

/// An accelerator over raw declaration keys, not a declaration authority.
/// Buckets retain every alias and private entry in metadata key order. The
/// owner keeps these indexes paired with immutable metadata, outside its wire
/// representation; equal table lengths do not establish snapshot identity.
pub(crate) struct MetadataNameIndex {
    functions: Map<Text, List<Text>>,
    types: Map<Text, List<Text>>,
}

impl MetadataNameIndex {
    pub(crate) fn prepare(
        metadata: &CoreMetadata,
        control: Option<&AnalysisControl>,
    ) -> Result<Self> {
        fn buckets<'a>(
            keys: impl Iterator<Item = &'a Text>,
            control: Option<&AnalysisControl>,
        ) -> Result<Map<Text, List<Text>>> {
            if let Some(control) = control {
                control.check()?;
            }
            let mut index: Map<Text, List<Text>> = Map::new();
            for key in keys {
                if let Some(control) = control {
                    control.check()?;
                }
                if let Some((_, leaf)) = key.as_str().rsplit_once('.') {
                    index.entry(leaf.into()).or_default().push(key.clone());
                }
            }
            Ok(index)
        }
        Ok(Self {
            functions: buckets(metadata.functions.keys(), control)?,
            types: buckets(metadata.types.keys(), control)?,
        })
    }

    fn matching_suffix<'a>(
        index: &'a Map<Text, List<Text>>,
        item: &str,
    ) -> impl Iterator<Item = &'a Text> {
        let leaf = item.rsplit('.').next().unwrap_or(item);
        let suffix = format!(".{item}");
        index
            .get(&Text::from(leaf))
            .into_iter()
            .flat_map(|keys| keys.iter())
            .filter(move |key| key.as_str().ends_with(&suffix))
    }

    pub(crate) fn function_keys_between<'a>(
        &'a self,
        prefix: &str,
        item: &str,
    ) -> impl Iterator<Item = &'a Text> {
        let head = format!("{prefix}.");
        Self::matching_suffix(&self.functions, item)
            .filter(move |key| key.as_str().starts_with(&head))
    }

    pub(crate) fn type_keys_ending_with<'a>(
        &'a self,
        item: &str,
    ) -> impl Iterator<Item = &'a Text> {
        Self::matching_suffix(&self.types, item)
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
