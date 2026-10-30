//! Exact module reads, including failed lookups, for incremental consumers.
//! Observations bind to their original registry, not to a caller-supplied key.
use crate::{ModuleId, ModuleRegistry};
use parking_lot::Mutex;
use std::sync::Arc;
use verum_common::{Set, Text};

#[derive(Debug, Default)]
pub(crate) struct Reads {
    pub ids: Set<ModuleId>,
    pub paths: Set<Text>,
    pub aliases: Set<Text>,
    pub all: bool,
    pub length: bool,
    pub empty: bool,
    pub mutated: bool,
}

#[derive(Debug)]
pub struct ModuleObservations {
    before: Arc<ModuleRegistry>,
    reads: Arc<Mutex<Reads>>,
}

impl ModuleRegistry {
    /// Give one analysis an isolated registry and a receipt of its reads.
    /// Clones retain the recorder; writes make that receipt non-reusable.
    pub fn observe_reads(self: &Arc<Self>) -> (Self, ModuleObservations) {
        let reads = Arc::new(Mutex::new(Reads::default()));
        let mut registry = self.deep_clone();
        registry.observations = Some(reads.clone());
        (
            registry,
            ModuleObservations {
                before: self.clone(),
                reads,
            },
        )
    }

    pub(crate) fn observe(&self, operation: impl FnOnce(&mut Reads)) {
        if let Some(reads) = &self.observations {
            operation(&mut reads.lock());
        }
    }

    pub(crate) fn observe_mutation(&self) {
        self.observe(|reads| reads.mutated = true);
    }
}

impl ModuleObservations {
    /// Retain only the current generation after validating every recorded read.
    /// Failed comparisons leave the original evidence intact.
    pub fn rebase_if_unchanged(&mut self, current: Arc<ModuleRegistry>) -> bool {
        if !self.unchanged(&current) {
            return false;
        }
        self.before = current;
        true
    }

    /// Full structural equality is required for each observed result.
    /// Catalogue scans conservatively depend on the complete catalogue.
    pub fn unchanged(&self, current: &ModuleRegistry) -> bool {
        let reads = self.reads.lock();
        let before = &self.before;
        if reads.mutated
            || before.cog_name != current.cog_name
            || before.path_aliases != current.path_aliases
        {
            return false;
        }
        if reads.all && before.modules != current.modules {
            return false;
        }
        if reads.length && before.modules.len() != current.modules.len() {
            return false;
        }
        if reads.empty && before.modules.is_empty() != current.modules.is_empty() {
            return false;
        }
        reads
            .ids
            .iter()
            .all(|id| before.modules.get(id) == current.modules.get(id))
            && reads
                .paths
                .iter()
                .all(|path| before.lookup_by_path(path) == current.lookup_by_path(path))
            && reads.aliases.iter().all(|path| {
                before.lookup_by_path_aliased(path) == current.lookup_by_path_aliased(path)
            })
    }
}
