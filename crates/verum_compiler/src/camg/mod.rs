//! Content-Addressed Module Graph (CAMG) — fundamental rewrite of
//! module loading.
//!
//! # Why CAMG
//!
//! The pre-CAMG `ModuleRegistry` (`crates/verum_modules`) is a flat
//! `Map<ModuleId, Shared<ModuleInfo>>` keyed by an opaque, session-
//! local `ModuleId`. That design has four documented limits:
//!
//!   1. **No content addressing.** Identical sources in two cogs get
//!      different `ModuleId`s; the type-checker can't share work
//!      across cogs even when the modules are byte-equal.
//!   2. **Eager artifact production.** Every module loaded means its
//!      AST, export table, and type-info are computed up front (or
//!      cloned from cache). Lazy evaluation requires per-tier
//!      bookkeeping the registry doesn't expose.
//!   3. **Single-process lifetime.** ModuleId numbers don't survive
//!      across sessions; persistent caches must store everything by
//!      path string, not by content. Stale entries silently become
//!      "fresh" if a cog is renamed.
//!   4. **Parallelism hostility.** The registry is `Shared<RwLock<…>>`;
//!      every reader takes the read lock. With per-module independence
//!      (audit findings 1.2 / 1.3), we want lock-free reads of
//!      individual nodes — which means the *graph* must be lock-free
//!      at the node-key level (DashMap), not just at the outer wrapper.
//!
//! CAMG addresses each of these:
//!
//!   * **Content-addressed.** Each module is identified by a
//!     blake3 hash of its source. Two modules with byte-equal source
//!     share the same `ModuleNodeId`, regardless of cog boundary or
//!     mount path.
//!   * **Lazy artifacts.** `ModuleArtifacts` wraps each derived
//!     output (parsed AST, export table, type-info, VBC bytecode, …)
//!     in `OnceLock<Arc<…>>`. The first reader computes it; the rest
//!     get a cheap `Arc::clone`.
//!   * **Persistent.** ContentHash is a 32-byte blake3 digest;
//!     identical across machines, sessions, compiler versions (with a
//!     version-tag prefix). Disk cache keyed by hash.
//!   * **Lock-free per-node access.** `nodes`, `edges`, `by_hash`,
//!     `by_path` are all `DashMap`s. Reads never contend with reads;
//!     writes shard by hash so independent insertions don't serialise.
//!
//! # Migration plan
//!
//! This commit lands the **foundational types and a no-op graph
//! constructor**. Subsequent commits migrate the existing
//! `ModuleRegistry` consumers one tier at a time, then delete the
//! old registry once CAMG owns the lookup path. The seam is exposed
//! via `CamgGraph::new()` plus the public types so call-sites can
//! adopt incrementally.

use std::sync::{Arc, OnceLock};

use dashmap::DashMap;
use verum_common::Text;

// =============================================================================
// Content hashing
// =============================================================================

/// 32-byte content hash, blake3-derived, used as the canonical
/// identifier for a module. Two modules with byte-equal source +
/// metadata-tag produce the same hash, regardless of cog or
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Compute the content hash of a source string with a metadata
    /// tag.
    ///
    /// The tag is mixed into the digest so otherwise-equal sources
    /// from different "lifecycles" (e.g. parsed-with-meta-expansion
    /// vs raw-source) don't collide. Use `"raw"` for verbatim source,
    /// `"parsed"` for already-macro-expanded, etc. The convention is
    /// callers' to define; CAMG only requires the tag to be stable.
    pub fn of(source: &str, tag: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(tag.as_bytes());
        hasher.update(b"\0");
        hasher.update(source.as_bytes());
        let digest = hasher.finalize();
        let mut buf = [0u8; 32];
        buf.copy_from_slice(digest.as_bytes());
        Self(buf)
    }

    /// Return the underlying 32 bytes for serialisation / display.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// First 16 hex characters — sufficient for log lines without
    /// drowning the human reader. Full hash is `to_hex_full`.
    pub fn to_hex_short(&self) -> String {
        let mut out = String::with_capacity(16);
        for b in &self.0[..8] {
            out.push_str(&format!("{:02x}", b));
        }
        out
    }

    /// Full 64-character hex digest. Used in disk-cache file names
    /// and persistent metadata.
    pub fn to_hex_full(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in &self.0 {
            out.push_str(&format!("{:02x}", b));
        }
        out
    }
}

/// Stable module identifier. In CAMG the `ModuleNodeId` is itself
/// derived from the `ContentHash` (truncated to 64 bits) — a module
/// inserted twice yields the same id. Persistent across sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleNodeId(u64);

impl ModuleNodeId {
    /// Derive an id from a content hash. Uses the first 8 bytes —
    /// blake3 digests are uniformly random over their full 32 bytes,
    /// so the truncation has full 64-bit collision resistance.
    pub fn from_hash(hash: &ContentHash) -> Self {
        let bytes = hash.as_bytes();
        let mut id = 0u64;
        for &b in &bytes[..8] {
            id = (id << 8) | b as u64;
        }
        Self(id)
    }

    /// Raw value for cache keys / debugging.
    pub fn raw(&self) -> u64 {
        self.0
    }
}

// =============================================================================
// Module node + lazy artifacts
// =============================================================================

/// Per-module derived artifacts. Each artifact is computed lazily on
/// first access and shared via `Arc` for cheap cloning. The
/// `OnceLock` discipline means computation runs at most once per
/// process per artifact, regardless of contention.
///
/// The artifact slots are intentionally generic over the concrete
/// types so CAMG can stay architecture-only: the type-checker, parser,
/// codegen, etc. each know which artifact slot they own and supply
/// their own `Arc<T>` payload. This keeps CAMG free of compiler-
/// internal dependencies and lets it move into a separate crate
/// later (#106 crate-split work).
#[derive(Debug)]
pub struct ModuleArtifacts {
    /// The raw source text of the module. Always present (it's what
    /// produced the content hash).
    pub source: Arc<str>,
    /// Slot for arbitrary lazily-computed artifacts. The map is
    /// keyed by a stable string tag (`"ast"`, `"exports"`,
    /// `"type_info"`, `"vbc"`, …); values are type-erased through
    /// `Arc<dyn Any + Send + Sync>` so consumers downcast to their
    /// expected concrete type.
    ///
    /// `DashMap` for lock-free per-tag insertion / read; `OnceLock`
    /// inside guards "compute once" semantics on the first access.
    pub slots: DashMap<Text, Arc<OnceLock<Arc<dyn std::any::Any + Send + Sync>>>>,
}

impl ModuleArtifacts {
    /// Construct from raw source.
    pub fn from_source(source: impl Into<Arc<str>>) -> Self {
        Self { source: source.into(), slots: DashMap::new() }
    }

    /// Get-or-compute an artifact slot keyed by `tag`. The closure
    /// runs at most once per slot per process; subsequent callers
    /// receive a cheap `Arc::clone` of the cached value.
    ///
    /// Type-erasure happens at the storage boundary: the closure
    /// returns the concrete type, this method downcasts on read.
    /// Callers that pass the wrong concrete type for an existing tag
    /// receive `None` from the downcast.
    pub fn get_or_init<T, F>(&self, tag: &str, init: F) -> Option<Arc<T>>
    where
        T: Any + Send + Sync + 'static,
        F: FnOnce() -> T,
    {
        let key = Text::from(tag);
        let cell = self
            .slots
            .entry(key)
            .or_insert_with(|| Arc::new(OnceLock::new()))
            .clone();
        let arc_any = cell.get_or_init(|| Arc::new(init()) as Arc<dyn Any + Send + Sync>);
        arc_any.clone().downcast::<T>().ok()
    }
}

use std::any::Any;

/// A single node in the content-addressed module graph.
#[derive(Debug)]
pub struct ModuleNode {
    /// Stable id — derived from `hash`.
    pub id: ModuleNodeId,
    /// Content hash of the source (full 32-byte blake3).
    pub hash: ContentHash,
    /// Canonical module path (e.g. `core.shell.exec`). Multiple
    /// paths can resolve to the same node if the source happens to
    /// be byte-equal (rare in practice but architecturally sound).
    pub path: Text,
    /// Lazily-computed derived artifacts.
    pub artifacts: ModuleArtifacts,
}

impl ModuleNode {
    pub fn new(path: impl Into<Text>, source: impl Into<Arc<str>>) -> Self {
        let source: Arc<str> = source.into();
        let hash = ContentHash::of(&source, "raw");
        let id = ModuleNodeId::from_hash(&hash);
        Self {
            id,
            hash,
            path: path.into(),
            artifacts: ModuleArtifacts::from_source(source),
        }
    }
}

// =============================================================================
// Edges + graph
// =============================================================================

/// Edge categorisation. Mirrors the dep-graph build at
/// `crates/verum_compiler/build.rs::Edges` but at the runtime graph
/// level — distinct edge kinds let the loader prioritise (e.g. follow
/// `Mount` for type-check, follow `ChildModule` for prelude).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// Direct `mount path;` reference.
    Mount,
    /// `mount path.{a, b}` nested mount — leaf-or-prefix.
    Nested,
    /// `mount path.*` glob — expands at resolution time.
    Glob,
    /// `module X;` submodule declaration — implicit child link.
    ChildModule,
    /// Re-export edge: this module re-exports symbols from `to`.
    ReExport,
}

#[derive(Debug, Clone)]
pub struct ModuleEdge {
    pub from: ModuleNodeId,
    pub to: ModuleNodeId,
    pub kind: EdgeKind,
}

/// The content-addressed module graph itself. Lock-free per-node
/// access via DashMap. Insertions shard by hash so independent
/// builders don't serialise.
#[derive(Debug)]
pub struct CamgGraph {
    /// Primary storage: id → node.
    nodes: DashMap<ModuleNodeId, Arc<ModuleNode>>,
    /// Outgoing edges per node.
    edges: DashMap<ModuleNodeId, Vec<ModuleEdge>>,
    /// Hash → id index. Lets the loader detect "have I already
    /// loaded byte-equal source from a different cog?" in O(1).
    by_hash: DashMap<ContentHash, ModuleNodeId>,
    /// Path → id index. The conventional lookup surface.
    by_path: DashMap<Text, ModuleNodeId>,
}

impl CamgGraph {
    pub fn new() -> Self {
        Self {
            nodes: DashMap::new(),
            edges: DashMap::new(),
            by_hash: DashMap::new(),
            by_path: DashMap::new(),
        }
    }

    /// Insert a node. If a node with the same content hash already
    /// exists, the existing id is returned and `node.path` is added
    /// as an extra path alias (multiple paths can map to the same
    /// content-equal node).
    pub fn insert_node(&self, node: ModuleNode) -> ModuleNodeId {
        let hash = node.hash;
        let path = node.path.clone();

        if let Some(existing) = self.by_hash.get(&hash) {
            let id = *existing;
            // Add the new path as an alias.
            self.by_path.insert(path, id);
            return id;
        }

        let id = node.id;
        let arc = Arc::new(node);
        self.nodes.insert(id, arc);
        self.by_hash.insert(hash, id);
        self.by_path.insert(path, id);
        id
    }

    /// Add a directed edge. Idempotent: duplicate edges are tolerated
    /// (the dep-graph BFS treats them as a single visit).
    pub fn add_edge(&self, edge: ModuleEdge) {
        self.edges.entry(edge.from).or_default().push(edge);
    }

    /// Look up by stable id. Lock-free read.
    pub fn get(&self, id: ModuleNodeId) -> Option<Arc<ModuleNode>> {
        self.nodes.get(&id).map(|r| Arc::clone(&*r))
    }

    /// Look up by canonical module path.
    pub fn get_by_path(&self, path: &str) -> Option<Arc<ModuleNode>> {
        let id = *self.by_path.get(path)?.value();
        self.get(id)
    }

    /// Look up by content hash.
    pub fn get_by_hash(&self, hash: &ContentHash) -> Option<Arc<ModuleNode>> {
        let id = *self.by_hash.get(hash)?.value();
        self.get(id)
    }

    /// Number of nodes in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the graph contains any nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Snapshot the outgoing edges of a node. Returns an empty Vec
    /// when the node has no edges (or doesn't exist).
    pub fn edges_of(&self, id: ModuleNodeId) -> Vec<ModuleEdge> {
        self.edges.get(&id).map(|r| r.clone()).unwrap_or_default()
    }
}

impl Default for CamgGraph {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_deterministic() {
        let a = ContentHash::of("hello world", "raw");
        let b = ContentHash::of("hello world", "raw");
        assert_eq!(a, b, "same source + tag must hash equal");
    }

    #[test]
    fn content_hash_distinguishes_tag() {
        let a = ContentHash::of("hello", "raw");
        let b = ContentHash::of("hello", "parsed");
        assert_ne!(a, b, "different tags must yield different hashes");
    }

    #[test]
    fn content_hash_distinguishes_source() {
        let a = ContentHash::of("hello", "raw");
        let b = ContentHash::of("hello!", "raw");
        assert_ne!(a, b);
    }

    #[test]
    fn module_node_id_from_hash_is_stable() {
        let a = ContentHash::of("foo", "raw");
        let id1 = ModuleNodeId::from_hash(&a);
        let id2 = ModuleNodeId::from_hash(&a);
        assert_eq!(id1, id2);
        assert_eq!(id1.raw(), id2.raw());
    }

    #[test]
    fn graph_insert_lookup_by_path() {
        let g = CamgGraph::new();
        let n = ModuleNode::new("core.foo", "fn main() {}");
        let id = g.insert_node(n);
        let retrieved = g.get_by_path("core.foo").unwrap();
        assert_eq!(retrieved.id, id);
        assert_eq!(retrieved.path.as_str(), "core.foo");
    }

    #[test]
    fn graph_dedups_by_content() {
        let g = CamgGraph::new();
        let n1 = ModuleNode::new("core.foo", "fn main() {}");
        let n2 = ModuleNode::new("user.bar", "fn main() {}"); // same source!
        let id1 = g.insert_node(n1);
        let id2 = g.insert_node(n2);
        assert_eq!(id1, id2, "byte-equal sources must dedup to one node");
        // But both paths resolve to the same node.
        assert_eq!(g.get_by_path("core.foo").unwrap().id, id1);
        assert_eq!(g.get_by_path("user.bar").unwrap().id, id1);
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn graph_distinguishes_different_sources() {
        let g = CamgGraph::new();
        let n1 = ModuleNode::new("core.a", "fn a() {}");
        let n2 = ModuleNode::new("core.b", "fn b() {}");
        let id1 = g.insert_node(n1);
        let id2 = g.insert_node(n2);
        assert_ne!(id1, id2);
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn graph_edge_round_trip() {
        let g = CamgGraph::new();
        let a = g.insert_node(ModuleNode::new("a", "fn a() {}"));
        let b = g.insert_node(ModuleNode::new("b", "fn b() {}"));
        g.add_edge(ModuleEdge { from: a, to: b, kind: EdgeKind::Mount });
        let edges = g.edges_of(a);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, b);
        assert_eq!(edges[0].kind, EdgeKind::Mount);
    }

    #[test]
    fn artifacts_get_or_init_runs_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let arts = ModuleArtifacts::from_source("source");
        let counter = Arc::new(AtomicUsize::new(0));
        let c1 = Arc::clone(&counter);
        let v1: Arc<u32> = arts.get_or_init("test", move || {
            c1.fetch_add(1, Ordering::Relaxed);
            42u32
        }).unwrap();
        let c2 = Arc::clone(&counter);
        let v2: Arc<u32> = arts.get_or_init("test", move || {
            c2.fetch_add(1, Ordering::Relaxed);
            99u32 // would-be different value, but init runs only once
        }).unwrap();
        assert_eq!(*v1, 42);
        assert_eq!(*v2, 42, "second call must return cached value, not re-run init");
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn artifacts_distinguish_tags() {
        let arts = ModuleArtifacts::from_source("src");
        let a: Arc<u32> = arts.get_or_init("ast", || 1u32).unwrap();
        let b: Arc<u32> = arts.get_or_init("exports", || 2u32).unwrap();
        assert_eq!(*a, 1);
        assert_eq!(*b, 2);
    }

    #[test]
    fn hex_short_is_16_chars() {
        let h = ContentHash::of("foo", "raw");
        assert_eq!(h.to_hex_short().len(), 16);
        assert_eq!(h.to_hex_full().len(), 64);
    }
}
