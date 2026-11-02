//! A function registry with a mutation-safe index of qualified name suffixes.
//!
//! Only names are indexed. Function identities, overloads, and arities are read
//! from the live registry by the existing resolution rules.

use super::FunctionInfo;
use std::borrow::Borrow;
use std::collections::{BTreeSet, HashMap, hash_map};
use std::hash::Hash;
use std::ops::Deref;

#[derive(Clone, Debug, Default)]
pub struct FunctionRegistry {
    entries: HashMap<String, FunctionInfo>,
    // Reversing makes arbitrary suffixes contiguous without storing one copy
    // for every dot/colon component. Keys remain private: there is no mutable
    // HashMap escape that could bypass index maintenance.
    reversed_names: BTreeSet<String>,
}

fn reversed(name: &str) -> String {
    name.chars().rev().collect()
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Exactly the sorted, deduplicated keys ending in `.{suffix}` or
    /// `::{suffix}`. This includes compound suffixes and overload spellings.
    pub fn qualified_suffix_candidates(&self, suffix: &str) -> Vec<String> {
        self.qualified_suffix_candidates_with_forms(suffix, suffix)
    }

    /// As above, with independently supplied dotted and colon forms. Qualified
    /// call lookup already normalizes its dotted form before suffix matching.
    pub fn qualified_suffix_candidates_with_forms(&self, dotted: &str, colon: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        for (suffix, separator) in [(dotted, "."), (colon, "::")] {
            let prefix = format!("{}{separator}", reversed(suffix));
            for name in self.reversed_names.range(prefix.clone()..) {
                if !name.starts_with(&prefix) {
                    break;
                }
                candidates.push(reversed(name));
            }
        }
        candidates.sort();
        candidates.dedup();
        candidates
    }

    pub fn insert(&mut self, name: String, info: FunctionInfo) -> Option<FunctionInfo> {
        match self.entries.entry(name) {
            hash_map::Entry::Occupied(mut entry) => Some(entry.insert(info)),
            hash_map::Entry::Vacant(entry) => {
                self.reversed_names.insert(reversed(entry.key()));
                entry.insert(info);
                None
            }
        }
    }

    pub fn remove<Q>(&mut self, name: &Q) -> Option<FunctionInfo>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.entries.remove_entry(name).map(|(name, info)| {
            self.reversed_names.remove::<str>(&reversed(&name));
            info
        })
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.reversed_names.clear();
    }

    pub fn entry(&mut self, name: String) -> FunctionRegistryEntry<'_> {
        FunctionRegistryEntry {
            entry: self.entries.entry(name),
            reversed_names: &mut self.reversed_names,
        }
    }

    // Mutable access to values cannot change the name index. In particular,
    // bootstrap updates of param_count must be observed by live arity lookup.
    pub fn get_mut<Q>(&mut self, name: &Q) -> Option<&mut FunctionInfo>
    where
        String: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.entries.get_mut(name)
    }

    pub fn values_mut(&mut self) -> hash_map::ValuesMut<'_, String, FunctionInfo> {
        self.entries.values_mut()
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&String, &mut FunctionInfo) -> bool) {
        let names = &mut self.reversed_names;
        self.entries.retain(|name, info| {
            if keep(name, info) {
                true
            } else {
                names.remove(&reversed(name));
                false
            }
        });
    }

    pub fn as_map(&self) -> &HashMap<String, FunctionInfo> {
        &self.entries
    }
}

/// Read-only map compatibility, without a `DerefMut` key-mutation bypass.
impl Deref for FunctionRegistry {
    type Target = HashMap<String, FunctionInfo>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<'a> IntoIterator for &'a FunctionRegistry {
    type Item = (&'a String, &'a FunctionInfo);
    type IntoIter = hash_map::Iter<'a, String, FunctionInfo>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl Extend<(String, FunctionInfo)> for FunctionRegistry {
    fn extend<T: IntoIterator<Item = (String, FunctionInfo)>>(&mut self, entries: T) {
        for (name, info) in entries {
            self.insert(name, info);
        }
    }
}

impl FromIterator<(String, FunctionInfo)> for FunctionRegistry {
    fn from_iter<T: IntoIterator<Item = (String, FunctionInfo)>>(entries: T) -> Self {
        let mut registry = Self::new();
        registry.extend(entries);
        registry
    }
}

impl From<HashMap<String, FunctionInfo>> for FunctionRegistry {
    fn from(entries: HashMap<String, FunctionInfo>) -> Self {
        let reversed_names = entries.keys().map(|name| reversed(name)).collect();
        Self {
            entries,
            reversed_names,
        }
    }
}

/// Entry insertion updates the index only when a value is actually inserted.
/// An unused vacant entry or a panicking value factory leaves both unchanged.
pub struct FunctionRegistryEntry<'a> {
    entry: hash_map::Entry<'a, String, FunctionInfo>,
    reversed_names: &'a mut BTreeSet<String>,
}

impl<'a> FunctionRegistryEntry<'a> {
    pub fn key(&self) -> &String {
        self.entry.key()
    }

    pub fn or_insert(self, default: FunctionInfo) -> &'a mut FunctionInfo {
        self.or_insert_with(|| default)
    }

    pub fn or_insert_with(self, default: impl FnOnce() -> FunctionInfo) -> &'a mut FunctionInfo {
        self.or_insert_with_key(|_| default())
    }

    pub fn or_insert_with_key(
        self,
        default: impl FnOnce(&String) -> FunctionInfo,
    ) -> &'a mut FunctionInfo {
        match self.entry {
            hash_map::Entry::Occupied(entry) => entry.into_mut(),
            hash_map::Entry::Vacant(entry) => {
                let info = default(entry.key());
                self.reversed_names.insert(reversed(entry.key()));
                entry.insert(info)
            }
        }
    }

    pub fn or_default(self) -> &'a mut FunctionInfo {
        self.or_insert_with(FunctionInfo::default)
    }

    pub fn and_modify(mut self, modify: impl FnOnce(&mut FunctionInfo)) -> Self {
        if let hash_map::Entry::Occupied(entry) = &mut self.entry {
            modify(entry.get_mut());
        }
        self
    }
}
