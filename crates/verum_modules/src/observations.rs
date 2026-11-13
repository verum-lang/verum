//! Exact module reads, including failed lookups, for incremental consumers.
//! Observations bind to their original registry, not to a caller-supplied key.
use crate::{ModuleId, ModuleInfo, ModuleRegistry, RegistryData};
use parking_lot::Mutex;
use std::sync::Arc;
use verum_common::{Set, Shared, Text};

#[derive(Debug)]
pub(crate) struct Recorder {
    reads: Mutex<Reads>,
    parent: Option<Arc<Recorder>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Reads {
    pub ids: Set<ModuleId>,
    pub paths: Set<Text>,
    pub aliases: Set<Text>,
    pub all: bool,
    pub length: bool,
    pub empty: bool,
    pub mutated: bool,
    pub alias_table: bool,
    pub cog_name: bool,
}

#[derive(Debug)]
pub struct ModuleObservations {
    before: Arc<ModuleRegistry>,
    recorder: Arc<Recorder>,
}

/// Fixed inputs of an operation performed against one immutable registry snapshot.
#[derive(Debug)]
pub struct ModuleDependencies {
    before: Shared<RegistryData>,
    reads: Reads,
}

impl ModuleRegistry {
    /// Give one analysis an isolated registry and a receipt of its reads.
    /// Clones retain the recorder; writes make that receipt non-reusable.
    pub fn observe_reads(self: &Arc<Self>) -> (Self, ModuleObservations) {
        self.tracked(self.deep_clone())
    }

    /// Observe a declaration transaction, sharing the session's ID allocator.
    pub fn observe_snapshot(&self) -> (Self, ModuleObservations) {
        Arc::new(self.clone()).tracked(self.clone())
    }

    fn tracked(self: &Arc<Self>, mut registry: Self) -> (Self, ModuleObservations) {
        let recorder = Arc::new(Recorder {
            reads: Mutex::new(Reads::default()),
            parent: self.observations.clone(),
        });
        registry.observations = Some(recorder.clone());
        (
            registry,
            ModuleObservations {
                before: self.clone(),
                recorder,
            },
        )
    }

    pub(crate) fn observe(&self, operation: impl Fn(&mut Reads)) {
        let mut current = self.observations.as_deref();
        while let Some(recorder) = current {
            operation(&mut recorder.reads.lock());
            current = recorder.parent.as_deref();
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

    pub fn unchanged(&self, current: &ModuleRegistry) -> bool {
        let reads = self.recorder.reads.lock();
        reads.matches(&self.before.data, current)
    }

    /// Seal this operation's reads. Subsequent uses of retained registry
    /// clones cannot widen the dependencies of an already completed operation.
    pub fn finish(self) -> ModuleDependencies {
        let reads = match Arc::try_unwrap(self.recorder) {
            Ok(recorder) => recorder.reads.into_inner(),
            Err(recorder) => recorder.reads.lock().clone(),
        };
        ModuleDependencies {
            before: self.before.data.clone(),
            reads,
        }
    }
}

impl ModuleDependencies {
    /// Check the actual inputs. Mutating an observed registry prevents reuse,
    /// even when its parent still has the same immutable starting generation.
    pub fn unchanged(&self, current: &ModuleRegistry) -> bool {
        self.reads.matches(&self.before, current)
    }

    /// Reusing an earlier result must expose its dependencies to the caller,
    /// including the reads of a refusal when the queried inputs have changed.
    pub fn reuse_in(&self, current: &ModuleRegistry) -> bool {
        current.observe(|reads| reads.include(&self.reads));
        self.unchanged(current)
    }
}

impl Reads {
    fn include(&mut self, source: &Self) {
        self.ids.extend(source.ids.iter().copied());
        self.paths.extend(source.paths.iter().cloned());
        self.aliases.extend(source.aliases.iter().cloned());
        self.all |= source.all;
        self.length |= source.length;
        self.empty |= source.empty;
        self.mutated |= source.mutated;
        self.alias_table |= source.alias_table;
        self.cog_name |= source.cog_name;
    }

    fn matches(&self, before: &RegistryData, current: &ModuleRegistry) -> bool {
        if self.mutated {
            return false;
        }
        if std::ptr::eq(before, &*current.data) {
            return true;
        }
        if (self.cog_name && before.cog_name != current.data.cog_name)
            || (self.alias_table && before.path_aliases != current.data.path_aliases)
            || (self.all && before.modules != current.data.modules)
            || (self.length && before.modules.len() != current.data.modules.len())
            || (self.empty && before.modules.is_empty() != current.data.modules.is_empty())
        {
            return false;
        }
        self.ids
            .iter()
            .all(|id| same_module(before.modules.get(id), current.data.modules.get(id)))
            && self.paths.iter().all(|path| {
                same_module(
                    before.lookup_by_path(path).as_ref(),
                    current.data.lookup_by_path(path).as_ref(),
                )
            })
            && self.aliases.iter().all(|path| {
                same_module(
                    before.lookup_by_path_aliased(path).as_ref(),
                    current.data.lookup_by_path_aliased(path).as_ref(),
                )
            })
    }
}

fn same_module(before: Option<&Shared<ModuleInfo>>, current: Option<&Shared<ModuleInfo>>) -> bool {
    match (before, current) {
        (Some(before), Some(current)) => std::ptr::eq(&**before, &**current) || before == current,
        (None, None) => true,
        _ => false,
    }
}
