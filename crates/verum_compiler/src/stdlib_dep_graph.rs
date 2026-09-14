//! Pre-computed `mount` dependency graph over the embedded stdlib.
//!
//! The build script (`build.rs`) scans every `core/*.vr` for `mount …;`
//! statements and emits a compact archive listing, for each stdlib
//! module, the set of other module paths it depends on. The archive is
//! embedded in the binary alongside the source archive.
//!
//! At runtime, this graph is the keystone for **on-demand stdlib
//! loading**: instead of registering all 2266 stdlib modules upfront,
//! the compiler computes the **transitive closure of modules reachable
//! from the user entry point's mount set** and loads only those.
//!
//! Reachability is conservative — a module is included whenever any
//! of these is true:
//!
//!  * It appears as a `mount path` target (or its parent does).
//!  * It is the source of a `mount path.*` glob.
//!  * It is named in a `mount path.{a, b}` nested mount.
//!  * It is a transitive dependency of an already-included module.
//!
//! This guarantees we never miss a dependency required for type-check
//! correctness, while typically pruning 90 %+ of the stdlib.
//!
//! # Performance contract
//!
//! - Decompress + parse: ~5 ms (~50 KB compressed graph).
//! - BFS reachability for a typical entry point: <1 ms.
//! - Memory: ~2 MB after deserialisation (sparse adjacency list).
//!
//! # Threading
//!
//! Singleton `OnceLock`. First reader builds the graph; subsequent
//! readers see the populated value. No mutation after build.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;

/// Compressed graph archive, embedded at build time.
static DEP_GRAPH_COMPRESSED: &[u8] = include_bytes!(env!("STDLIB_DEP_GRAPH_PATH"));

/// Lazily decompressed and indexed graph.
static DEP_GRAPH: OnceLock<Option<DepGraph>> = OnceLock::new();

/// Edge categories recorded in the on-disk archive. The runtime treats
/// `Path` and `Nested` identically for reachability; `Glob` is recorded
/// separately because callers may want different downstream behaviour
/// (e.g. enumerating all submodules under a glob source).
const EDGE_PATH: u8 = 0;
const EDGE_GLOB: u8 = 1;
const EDGE_NESTED: u8 = 2;
/// A dotted cross-module CALL. Written by `build.rs` in addition to the
/// `EDGE_PATH` copy, so `reachable_through_calls` can walk what a body
/// actually NAMES without dragging in what its mounts merely make visible.
const EDGE_CALL: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// `mount path` — direct module reference.
    Path,
    /// `mount path.*` — glob over a module's submodules.
    Glob,
    /// `mount prefix.{a, b}` — nested item / module reference.
    Nested,
    /// `super.a.b.f(…)` / `core.a.b.f(…)` — a dotted cross-module call.
    Call,
}

/// Direct mount edges from a single source module.
#[derive(Debug, Clone)]
pub struct Edges {
    /// Direct module references (`mount path`).
    pub path: Vec<String>,
    /// Glob sources (`mount path.*`).
    pub glob: Vec<String>,
    /// Items / submodules listed in a nested mount (`mount prefix.{a, b}`).
    /// The graph stores both the leaf candidate and the prefix module
    /// itself — see `extract_mounts` in build.rs.
    pub nested: Vec<String>,
    /// WHOLE dotted names called from this module's body —
    /// `core.sys.darwin.libsystem.safe_getentropy`, function segment
    /// included. `path` carries the same call under its MODULE; the two
    /// exist separately because naming a module and naming a function
    /// mean very different things to the archive loader's filter.
    pub call: Vec<String>,
}

/// Pre-computed mount adjacency over the embedded stdlib.
pub struct DepGraph {
    /// `module path` → outgoing edges.
    edges: HashMap<String, Edges>,
}

impl DepGraph {
    fn from_compressed(compressed: &[u8]) -> Option<Self> {
        if compressed.is_empty() {
            return None;
        }
        let raw = zstd::decode_all(compressed).ok()?;
        Self::parse_archive(&raw)
    }

    fn parse_archive(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        let module_count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut cursor = 4usize;
        let mut edges: HashMap<String, Edges> = HashMap::with_capacity(module_count);

        for _ in 0..module_count {
            // Module name
            let module = read_str(data, &mut cursor)?;
            // Edge count
            if cursor + 2 > data.len() {
                return None;
            }
            let edge_count = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
            cursor += 2;

            let mut e = Edges {
                path: Vec::new(),
                glob: Vec::new(),
                nested: Vec::new(),
                call: Vec::new(),
            };
            for _ in 0..edge_count {
                if cursor >= data.len() {
                    return None;
                }
                let kind = data[cursor];
                cursor += 1;
                let target = read_str(data, &mut cursor)?;
                match kind {
                    EDGE_PATH => e.path.push(target),
                    EDGE_GLOB => e.glob.push(target),
                    EDGE_NESTED => e.nested.push(target),
                    EDGE_CALL => e.call.push(target),
                    _ => return None, // unknown kind → bail
                }
            }
            edges.insert(module, e);
        }

        Some(Self { edges })
    }

    /// Look up direct edges for a module. Returns an empty `Edges`
    /// stand-in for unknown modules (treated as having no dependencies).
    pub fn edges_of(&self, module: &str) -> Option<&Edges> {
        self.edges.get(module)
    }

    /// Number of modules in the graph.
    pub fn module_count(&self) -> usize {
        self.edges.len()
    }

    /// Iterator over all modules in the graph (for diagnostics).
    pub fn modules(&self) -> impl Iterator<Item = &str> {
        self.edges.keys().map(String::as_str)
    }

    /// Compute the transitive closure of modules reachable from a seed
    /// set via mount edges. Conservative — includes every endpoint of
    /// every edge kind.
    ///
    /// `glob_expander` is invoked for every glob edge `prefix.*`; it
    /// should return all stdlib modules whose path starts with `prefix.`
    /// (typically backed by `StdlibModuleIndex::all_modules()`).
    pub fn reachable_from<F>(&self, seeds: &[String], glob_expander: F) -> HashSet<String>
    where
        F: Fn(&str) -> Vec<String>,
    {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        // Seed enqueue: also climb each seed's parent chain so a leaf
        // item path (`core.shell.exec.run`) drags in its owning module
        // (`core.shell.exec`) even when the leaf has no graph entry.
        // Stops one segment above `core` — including `core` itself
        // would re-seed via `core/mod.vr`'s blanket re-exports.
        let enqueue_with_parents =
            |s: &str, visited: &mut HashSet<String>, queue: &mut VecDeque<String>| {
                let mut current: &str = s;
                loop {
                    if visited.insert(current.to_string()) {
                        queue.push_back(current.to_string());
                    }
                    let Some(dot) = current.rfind('.') else {
                        break;
                    };
                    let parent = &current[..dot];
                    if parent == "core" {
                        break;
                    }
                    current = parent;
                }
            };

        for seed in seeds {
            enqueue_with_parents(seed, &mut visited, &mut queue);
        }

        while let Some(current) = queue.pop_front() {
            let Some(e) = self.edges.get(&current) else {
                continue;
            };

            // Direct + nested edges: enqueue the target plus its parent
            // chain (same logic as seeds — nested mounts may name items).
            for target in e.path.iter().chain(e.nested.iter()) {
                enqueue_with_parents(target, &mut visited, &mut queue);
            }

            // Glob edges: expand to all modules under the prefix.
            for prefix in &e.glob {
                for m in glob_expander(prefix) {
                    enqueue_with_parents(&m, &mut visited, &mut queue);
                }
            }
        }

        visited
    }

    /// Transitive closure over DOTTED-CALL edges, returning the WHOLE
    /// called names — `core.sys.darwin.libsystem.safe_getentropy`, not
    /// its module.
    ///
    /// `reachable_from` answers "what may this module SEE" and walks
    /// mount edges — `path` plus `nested`, each with its parent chain.
    /// Seeded at a file that mounts `core.prelude.*` that closure is
    /// most of the stdlib, which is right for a type-check closure and
    /// ruinous for a decode set.
    ///
    /// This answers the narrower question the loader asks — "whose BODY
    /// does a body of mine name" — and it answers it at FUNCTION
    /// granularity, which is the part that took two measurements to get
    /// right. Returning modules made the seven-line uuid programme
    /// answer correctly and `hello world` take over six minutes: a
    /// module in the wanted set is a WHOLESALE mount, and every function
    /// it declares gets registered. A whole name is one function.
    ///
    /// The walk itself still proceeds by module — each target's own
    /// edges are looked up under its module part — so a chain of two
    /// hops (uuid -> sys.common -> darwin.libsystem) closes.
    pub fn reachable_through_calls(&self, seeds: &[String]) -> HashSet<String> {
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut seeded: HashSet<String> = HashSet::new();
        for seed in seeds {
            if seeded.insert(seed.clone()) {
                queue.push_back(seed.clone());
            }
        }
        let mut walked: HashSet<String> = HashSet::new();
        let mut named: HashSet<String> = HashSet::new();
        while let Some(current) = queue.pop_front() {
            let Some(e) = self.edges.get(&current) else {
                continue;
            };
            for target in &e.call {
                if !named.insert(target.clone()) {
                    continue;
                }
                // Continue the walk from the target's MODULE — the whole
                // name is what the caller wants, the module is what has
                // the next set of edges.
                if let Some((module, _)) = target.rsplit_once('.')
                    && walked.insert(module.to_string())
                {
                    queue.push_back(module.to_string());
                }
            }
        }
        named
    }
}

fn read_str(data: &[u8], cursor: &mut usize) -> Option<String> {
    if *cursor + 2 > data.len() {
        return None;
    }
    let len = u16::from_le_bytes([data[*cursor], data[*cursor + 1]]) as usize;
    *cursor += 2;
    if *cursor + len > data.len() {
        return None;
    }
    let s = String::from_utf8(data[*cursor..*cursor + len].to_vec()).ok()?;
    *cursor += len;
    Some(s)
}

/// Get the global dep graph. Builds on first call; later calls are
/// HashMap reads. Returns `None` if the embedded graph is unavailable
/// (minimal builds without `core/`).
pub fn get_dep_graph() -> Option<&'static DepGraph> {
    DEP_GRAPH
        .get_or_init(|| DepGraph::from_compressed(DEP_GRAPH_COMPRESSED))
        .as_ref()
}

/// Whether the embedded dep graph is available.
pub fn has_dep_graph() -> bool {
    !DEP_GRAPH_COMPRESSED.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_loads_from_embedded_archive() {
        let Some(g) = get_dep_graph() else {
            return;
        };
        assert!(
            g.module_count() > 0,
            "embedded graph should contain >0 modules"
        );
    }

    /// The call walk reaches `safe_getentropy` ITSELF — not its module —
    /// and stays orders of magnitude smaller than the mount walk from
    /// the same seeds (T1481).
    ///
    /// Three claims, and each one is a measurement that cost a build:
    /// the chain `uuid -> sys.common -> darwin.libsystem` must close
    /// (two hops, so a one-hop walk fails here); what comes back must be
    /// the WHOLE name, because a module in the wanted set is a wholesale
    /// mount and that version took `hello world` from 1.7 seconds to
    /// over six minutes; and the result must stay far under the mount
    /// walk, which returns ~3900 names from these same seeds.
    #[test]
    fn the_call_walk_reaches_the_entropy_function_not_its_module() {
        let Some(g) = get_dep_graph() else {
            return;
        };
        let seeds = vec!["core.prelude".to_string(), "core.id.uuid".to_string()];
        let narrow = g.reachable_through_calls(&seeds);
        let sorted = || {
            let mut v: Vec<&str> = narrow.iter().map(String::as_str).collect();
            v.sort_unstable();
            v
        };
        assert!(
            narrow.contains("core.sys.darwin.libsystem.safe_getentropy"),
            "the call chain uuid -> sys.common -> darwin.libsystem must be walked \
             to the FUNCTION; got {:?}",
            sorted()
        );
        assert!(
            !narrow.contains("core.sys.darwin.libsystem"),
            "a bare module name would be a WHOLESALE mount downstream; got {:?}",
            sorted()
        );
        let wide = g.reachable_from(&seeds, |_| Vec::new());
        assert!(
            narrow.len() * 10 < wide.len(),
            "call walk {} must stay far under the mount walk {}",
            narrow.len(),
            wide.len()
        );
    }

    #[test]
    fn shell_reachability_is_a_subset() {
        let Some(g) = get_dep_graph() else {
            return;
        };
        // `core.shell.exec` should be reachable from itself.
        let seeds = vec!["core.shell.exec".to_string()];
        let reachable = g.reachable_from(&seeds, |_| Vec::new());
        assert!(reachable.contains("core.shell.exec"));

        // The BFS visits parent-chain paths and nested-leaf candidates
        // that don't correspond to real modules (e.g. `core.shell.exec.run`
        // is the seed but the leaf `run` is an item, not a module).
        // For the "subset" claim we count only the entries that are
        // actually present in the embedded graph (real modules).
        let real_count = reachable.iter().filter(|m| g.edges_of(m).is_some()).count();
        assert!(real_count > 0, "should reach at least one real module");
        assert!(
            real_count < g.module_count(),
            "real reachable {} should be < total {}",
            real_count,
            g.module_count()
        );
    }

    #[test]
    fn reachability_is_conservative_through_parents() {
        let Some(g) = get_dep_graph() else {
            return;
        };
        // Importing a leaf item (e.g. `mount core.shell.exec.{run}`) is
        // recorded as a nested edge to `core.shell.exec.run`. The walker
        // must climb to the parent module `core.shell.exec`.
        let seeds = vec!["core.shell.exec.run".to_string()];
        let reachable = g.reachable_from(&seeds, |_| Vec::new());
        assert!(
            reachable.contains("core.shell.exec"),
            "parent module should be reached via nested-leaf seed"
        );
    }
}
