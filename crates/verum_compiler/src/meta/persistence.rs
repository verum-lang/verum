//! Cross-run filesystem persistence for the meta evaluation cache —
//! Phase 9 V0 of the precompiled-stdlib epic.
//!
//! In-process [`MetaEvalCache`](super::cache::MetaEvalCache) is an LRU
//! cache that lives for the lifetime of one compiler run.  Every cold
//! build re-evaluates every `@meta` / `@const` / `@derive` call from
//! scratch — for stdlib-heavy programs the same Mirror-protocol-derived
//! method tables, the same field-layout `@const` blocks, the same
//! `@derive(TypedRow)` / `@derive(MysqlTypedRow)` expansions get
//! recomputed every single invocation.
//!
//! This module adds a content-addressed filesystem layer beneath the
//! in-memory cache.  Hits are validated against the source hash that
//! produced them, so source edits invalidate stale entries
//! automatically.  Compiler-version-keyed cache root means a compiler
//! upgrade nukes every entry — there's no risk of feeding stale
//! results from an older compiler into a newer one (and vice-versa).
//!
//! # Layout
//!
//! ```text
//! ~/.verum/meta-cache/<compiler-version>/<call-key-hex>.json
//! ```
//!
//! `<call-key-hex>` is the 16-hex-byte rendering of
//! `function_hash ⊕ args_hash`.  Per-entry JSON carries the call key
//! components, the source hash that produced the result, the persisted
//! [`PersistedMetaValue`], and the wall-clock timestamp.  Reads validate
//! every field before returning a hit.
//!
//! # Persistable subset
//!
//! [`MetaValue`] is the runtime representation of every compile-time
//! value, including AST fragments (`Expr`, `Type`, `Pattern`, `Item`,
//! `Items`).  Persistence covers the *primitive subset* —
//! `Unit / Bool / Int / UInt / Float / Char / Text / Bytes` plus
//! `Array / Tuple / Maybe / Map / Set` of the same.  AST-bearing variants
//! return [`PersistResult::SkippedNotPersistable`] from
//! [`PersistedMetaValue::try_from_meta`]; the in-memory cache still
//! holds them for the rest of the run.  This catches the most common
//! `@const` and `@derive` shapes without dragging the parser AST into
//! a serde-derive contract.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use verum_ast::MetaValue;
use verum_common::{Heap, List, Map, Maybe, OrderedMap, OrderedSet, Text};

/// Persistable shadow of [`MetaValue`].  Mirrors the primitive variant
/// list 1:1; AST variants (`Expr`, `Type`, `Pattern`, `Item`, `Items`)
/// have no counterpart and are skipped at conversion time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PersistedMetaValue {
    Unit,
    Bool(bool),
    Int(i128),
    UInt(u128),
    Float(f64),
    Char(char),
    Text(String),
    Bytes(Vec<u8>),
    Array(Vec<PersistedMetaValue>),
    Tuple(Vec<PersistedMetaValue>),
    Maybe(Option<Box<PersistedMetaValue>>),
    Map(Vec<(String, PersistedMetaValue)>),
    Set(Vec<String>),
}

/// Outcome of [`PersistedMetaValue::try_from_meta`].  Distinguishes
/// "persisted ok" from "this MetaValue can't be persisted" (AST
/// fragment) — the caller routes the latter back to the in-memory
/// cache only.
#[derive(Debug)]
pub enum PersistResult {
    /// Successfully converted; safe to write to disk.
    Persisted(PersistedMetaValue),
    /// Variant not in the persistable subset (`Expr` / `Type` /
    /// `Pattern` / `Item` / `Items`).  Caller skips fs writes.
    SkippedNotPersistable,
}

impl PersistedMetaValue {
    /// Try to convert a [`MetaValue`] to its persistable shadow.
    /// Returns [`PersistResult::SkippedNotPersistable`] for AST-bearing
    /// variants.
    pub fn try_from_meta(value: &MetaValue) -> PersistResult {
        match value {
            MetaValue::Unit => PersistResult::Persisted(PersistedMetaValue::Unit),
            MetaValue::Bool(b) => PersistResult::Persisted(PersistedMetaValue::Bool(*b)),
            MetaValue::Int(i) => PersistResult::Persisted(PersistedMetaValue::Int(*i)),
            MetaValue::UInt(u) => PersistResult::Persisted(PersistedMetaValue::UInt(*u)),
            MetaValue::Float(f) => PersistResult::Persisted(PersistedMetaValue::Float(*f)),
            MetaValue::Char(c) => PersistResult::Persisted(PersistedMetaValue::Char(*c)),
            MetaValue::Text(t) => {
                PersistResult::Persisted(PersistedMetaValue::Text(t.as_str().to_string()))
            }
            MetaValue::Bytes(b) => PersistResult::Persisted(PersistedMetaValue::Bytes(b.clone())),
            MetaValue::Array(items) => convert_list(items, PersistedMetaValue::Array),
            MetaValue::Tuple(items) => convert_list(items, PersistedMetaValue::Tuple),
            MetaValue::Maybe(maybe_inner) => match maybe_inner {
                Maybe::None => {
                    PersistResult::Persisted(PersistedMetaValue::Maybe(None))
                }
                Maybe::Some(inner) => match Self::try_from_meta(inner.as_ref()) {
                    PersistResult::Persisted(p) => PersistResult::Persisted(
                        PersistedMetaValue::Maybe(Some(Box::new(p))),
                    ),
                    PersistResult::SkippedNotPersistable => {
                        PersistResult::SkippedNotPersistable
                    }
                },
            },
            MetaValue::Map(map) => {
                let mut entries: Vec<(String, PersistedMetaValue)> =
                    Vec::with_capacity(map.len());
                for (k, v) in map.iter() {
                    match Self::try_from_meta(v) {
                        PersistResult::Persisted(p) => {
                            entries.push((k.as_str().to_string(), p));
                        }
                        PersistResult::SkippedNotPersistable => {
                            return PersistResult::SkippedNotPersistable;
                        }
                    }
                }
                PersistResult::Persisted(PersistedMetaValue::Map(entries))
            }
            MetaValue::Set(set) => {
                let entries: Vec<String> =
                    set.iter().map(|s| s.as_str().to_string()).collect();
                PersistResult::Persisted(PersistedMetaValue::Set(entries))
            }
            // AST-bearing variants — out of scope for V0 persistence.
            MetaValue::Expr(_)
            | MetaValue::Type(_)
            | MetaValue::Pattern(_)
            | MetaValue::Item(_)
            | MetaValue::Items(_) => PersistResult::SkippedNotPersistable,
        }
    }

    /// Round-trip back into a [`MetaValue`].  Always succeeds: the
    /// primitive subset has total coverage by construction.
    pub fn into_meta(self) -> MetaValue {
        match self {
            PersistedMetaValue::Unit => MetaValue::Unit,
            PersistedMetaValue::Bool(b) => MetaValue::Bool(b),
            PersistedMetaValue::Int(i) => MetaValue::Int(i),
            PersistedMetaValue::UInt(u) => MetaValue::UInt(u),
            PersistedMetaValue::Float(f) => MetaValue::Float(f),
            PersistedMetaValue::Char(c) => MetaValue::Char(c),
            PersistedMetaValue::Text(t) => MetaValue::Text(Text::from(t)),
            PersistedMetaValue::Bytes(b) => MetaValue::Bytes(b),
            PersistedMetaValue::Array(items) => {
                let mut out: List<MetaValue> = List::new();
                for item in items {
                    out.push(item.into_meta());
                }
                MetaValue::Array(out)
            }
            PersistedMetaValue::Tuple(items) => {
                let mut out: List<MetaValue> = List::new();
                for item in items {
                    out.push(item.into_meta());
                }
                MetaValue::Tuple(out)
            }
            PersistedMetaValue::Maybe(opt) => match opt {
                None => MetaValue::Maybe(Maybe::None),
                Some(boxed) => {
                    MetaValue::Maybe(Maybe::Some(Heap::new(boxed.into_meta())))
                }
            },
            PersistedMetaValue::Map(entries) => {
                let mut out: OrderedMap<Text, MetaValue> = OrderedMap::new();
                for (k, v) in entries {
                    out.insert(Text::from(k), v.into_meta());
                }
                MetaValue::Map(out)
            }
            PersistedMetaValue::Set(entries) => {
                let mut out: OrderedSet<Text> = OrderedSet::new();
                for s in entries {
                    out.insert(Text::from(s));
                }
                MetaValue::Set(out)
            }
        }
    }
}

fn convert_list(
    items: &List<MetaValue>,
    wrap: fn(Vec<PersistedMetaValue>) -> PersistedMetaValue,
) -> PersistResult {
    let mut out: Vec<PersistedMetaValue> = Vec::with_capacity(items.len());
    for item in items.iter() {
        match PersistedMetaValue::try_from_meta(item) {
            PersistResult::Persisted(p) => out.push(p),
            PersistResult::SkippedNotPersistable => {
                return PersistResult::SkippedNotPersistable;
            }
        }
    }
    PersistResult::Persisted(wrap(out))
}

/// On-disk record for one cached meta-call.  Compiler-version-keyed
/// directory means upgrades nuke every entry implicitly; the
/// `source_hash` field validates that the entry still corresponds to
/// the current source state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaCacheEntry {
    /// Compiler version that produced this entry.  Matches
    /// `env!("CARGO_PKG_VERSION")` at write time.  Reads check it for
    /// strict equality.
    pub compiler_version: String,
    /// Function name (rendered for diagnostics; not consulted on
    /// lookup — the call key carries the authoritative hash).
    pub function_name: String,
    /// `function_hash` from [`MetaCallKey`](super::cache::MetaCallKey).
    pub function_hash: u64,
    /// `args_hash` from [`MetaCallKey`](super::cache::MetaCallKey).
    pub args_hash: u64,
    /// Source hash that produced this result.  Mismatch on read =
    /// stale entry; treat as miss.
    pub source_hash: u64,
    /// Persisted result.  Always in the primitive subset (AST
    /// variants are skipped before reaching this path).
    pub value: PersistedMetaValue,
    /// Unix timestamp (seconds) at write time.  Diagnostic only —
    /// reads do not honour TTL.  TTL is enforced by the in-memory
    /// LRU; persisted entries are valid until source/compiler
    /// invalidates them.
    pub computed_at_seconds: u64,
}

/// Filesystem-backed persistence layer.  Owns one cache root; every
/// `(function_hash, args_hash)` pair gets its own JSON file.  Reads and
/// writes are best-effort — IO errors degrade to "miss" / "no
/// persistence" rather than killing the build.
#[derive(Debug, Clone)]
pub struct MetaCachePersistence {
    cache_root: PathBuf,
    compiler_version: String,
}

impl MetaCachePersistence {
    /// Construct with an explicit cache root.  Caller is responsible
    /// for creating the directory; the persistence layer will mkdir
    /// on first write but tolerates missing directories on read.
    pub fn new(cache_root: PathBuf, compiler_version: String) -> Self {
        Self {
            cache_root,
            compiler_version,
        }
    }

    /// Build a default cache root rooted at `home_dir`:
    /// `<home_dir>/.verum/meta-cache/<compiler-version>/`.
    ///
    /// Caller supplies `home_dir` (typically via `dirs::home_dir()`
    /// at the CLI entry point) so this crate stays free of the
    /// `dirs` dependency.  Test code passes a tempdir.
    pub fn default_root(home_dir: &Path, compiler_version: &str) -> PathBuf {
        home_dir
            .join(".verum")
            .join("meta-cache")
            .join(compiler_version)
    }

    /// Per-entry filename under [`Self::cache_root`].  Format:
    /// `<function_hash:016x>-<args_hash:016x>.json`.
    fn entry_path(&self, function_hash: u64, args_hash: u64) -> PathBuf {
        self.cache_root.join(format!(
            "{:016x}-{:016x}.json",
            function_hash, args_hash
        ))
    }

    /// Look up a persisted entry.  Returns `None` for cache miss,
    /// IO error, decode error, compiler-version mismatch, or
    /// source-hash mismatch.
    pub fn get(
        &self,
        function_hash: u64,
        args_hash: u64,
        source_hash: u64,
    ) -> Option<MetaValue> {
        let path = self.entry_path(function_hash, args_hash);
        let bytes = std::fs::read(&path).ok()?;
        let entry: MetaCacheEntry = serde_json::from_slice(&bytes).ok()?;
        if entry.compiler_version != self.compiler_version {
            return None;
        }
        if entry.function_hash != function_hash || entry.args_hash != args_hash {
            return None;
        }
        if entry.source_hash != source_hash {
            return None;
        }
        Some(entry.value.into_meta())
    }

    /// Persist a result.  Best-effort: directory creation, json
    /// encoding, and file write all degrade silently to "no
    /// persistence" on failure.  Returns `true` when the entry made
    /// it to disk, `false` otherwise.
    ///
    /// Returns `false` (without writing) for AST-bearing
    /// `MetaValue` variants; the in-memory cache still holds them.
    pub fn put(
        &self,
        function_name: &str,
        function_hash: u64,
        args_hash: u64,
        source_hash: u64,
        value: &MetaValue,
    ) -> bool {
        let persisted = match PersistedMetaValue::try_from_meta(value) {
            PersistResult::Persisted(p) => p,
            PersistResult::SkippedNotPersistable => return false,
        };

        if std::fs::create_dir_all(&self.cache_root).is_err() {
            return false;
        }

        let entry = MetaCacheEntry {
            compiler_version: self.compiler_version.clone(),
            function_name: function_name.to_string(),
            function_hash,
            args_hash,
            source_hash,
            value: persisted,
            computed_at_seconds: now_seconds(),
        };

        let path = self.entry_path(function_hash, args_hash);
        match serde_json::to_vec(&entry) {
            Ok(bytes) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&path, bytes).is_ok()
            }
            Err(_) => false,
        }
    }

    /// Wipe every entry under the cache root.  Used by
    /// `verum cache clean` and similar tooling.  Returns the number
    /// of files removed; errors degrade to 0.
    pub fn clear(&self) -> usize {
        let entries = match std::fs::read_dir(&self.cache_root) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        let mut removed: usize = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }

    /// Path to the cache root.  Diagnostic / inspection use only.
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }
}

fn now_seconds() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::span::Span;

    /// Smallest constructible Expr — empty tuple `()`. Used to
    /// build a `MetaValue::Expr` for AST-skip tests.
    fn dummy_expr() -> Expr {
        Expr::new(ExprKind::Tuple(List::new()), Span::default())
    }

    fn primitive(v: i128) -> MetaValue {
        MetaValue::Int(v)
    }

    #[test]
    fn round_trip_primitives() {
        let cases: Vec<MetaValue> = vec![
            MetaValue::Unit,
            MetaValue::Bool(true),
            MetaValue::Int(-42),
            MetaValue::UInt(7),
            MetaValue::Float(3.14),
            MetaValue::Char('λ'),
            MetaValue::Text(Text::from("hello")),
            MetaValue::Bytes(vec![1, 2, 3]),
        ];
        for input in cases {
            let p = match PersistedMetaValue::try_from_meta(&input) {
                PersistResult::Persisted(p) => p,
                PersistResult::SkippedNotPersistable => panic!("primitive rejected"),
            };
            let back = p.into_meta();
            assert_eq!(format!("{back:?}"), format!("{input:?}"));
        }
    }

    #[test]
    fn round_trip_array() {
        let mut items: List<MetaValue> = List::new();
        items.push(MetaValue::Int(1));
        items.push(MetaValue::Int(2));
        items.push(MetaValue::Int(3));
        let input = MetaValue::Array(items);
        let p = match PersistedMetaValue::try_from_meta(&input) {
            PersistResult::Persisted(p) => p,
            _ => panic!(),
        };
        let back = p.into_meta();
        match back {
            MetaValue::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(format!("{:?}", items[0]), format!("{:?}", MetaValue::Int(1)));
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_maybe_some() {
        let inner = Heap::new(MetaValue::Int(99));
        let input = MetaValue::Maybe(Maybe::Some(inner));
        let p = match PersistedMetaValue::try_from_meta(&input) {
            PersistResult::Persisted(p) => p,
            _ => panic!(),
        };
        let back = p.into_meta();
        match back {
            MetaValue::Maybe(Maybe::Some(inner)) => match inner.as_ref() {
                MetaValue::Int(99) => {}
                other => panic!("expected Int(99), got {other:?}"),
            },
            other => panic!("expected Maybe::Some, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_maybe_none() {
        let input = MetaValue::Maybe(Maybe::None);
        let p = match PersistedMetaValue::try_from_meta(&input) {
            PersistResult::Persisted(p) => p,
            _ => panic!(),
        };
        let back = p.into_meta();
        match back {
            MetaValue::Maybe(Maybe::None) => {}
            other => panic!("expected Maybe::None, got {other:?}"),
        }
    }

    #[test]
    fn ast_variants_skipped() {
        let input = MetaValue::Expr(dummy_expr());
        match PersistedMetaValue::try_from_meta(&input) {
            PersistResult::SkippedNotPersistable => {}
            PersistResult::Persisted(_) => panic!("Expr should not be persistable"),
        }
    }

    #[test]
    fn fs_persistence_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "test-compiler-1.0.0".to_string(),
        );

        let value = primitive(123);
        let function_hash: u64 = 0x1111_2222_3333_4444;
        let args_hash: u64 = 0x5555_6666_7777_8888;
        let source_hash: u64 = 0x9999_aaaa_bbbb_cccc;

        // Initial miss.
        assert!(
            cache.get(function_hash, args_hash, source_hash).is_none(),
            "should miss on empty cache"
        );

        // Put.
        let written = cache.put(
            "test_fn",
            function_hash,
            args_hash,
            source_hash,
            &value,
        );
        assert!(written, "put should succeed for primitive");

        // Hit.
        let got = cache
            .get(function_hash, args_hash, source_hash)
            .expect("post-put get should hit");
        match got {
            MetaValue::Int(123) => {}
            other => panic!("expected Int(123), got {other:?}"),
        }
    }

    #[test]
    fn source_hash_mismatch_misses() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "test-compiler-1.0.0".to_string(),
        );
        let function_hash: u64 = 1;
        let args_hash: u64 = 2;
        let source_hash: u64 = 3;
        cache.put(
            "test_fn",
            function_hash,
            args_hash,
            source_hash,
            &primitive(1),
        );

        // Different source hash = stale entry = miss.
        assert!(
            cache.get(function_hash, args_hash, 99).is_none(),
            "source hash mismatch must miss"
        );
        // Original source hash still hits.
        assert!(
            cache.get(function_hash, args_hash, source_hash).is_some(),
            "original source hash must still hit"
        );
    }

    #[test]
    fn compiler_version_mismatch_misses() {
        let dir = tempfile::tempdir().unwrap();
        let writer = MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "compiler-1.0.0".to_string(),
        );
        writer.put("f", 1, 2, 3, &primitive(7));

        // Different compiler version reading the same dir =
        // upgrade path = miss.
        let reader = MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "compiler-1.1.0".to_string(),
        );
        assert!(
            reader.get(1, 2, 3).is_none(),
            "compiler version mismatch must miss"
        );
    }

    #[test]
    fn ast_value_skips_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "test-compiler".to_string(),
        );
        let written = cache.put("f", 1, 2, 3, &MetaValue::Expr(dummy_expr()));
        assert!(!written, "Expr-bearing put must skip persistence");
    }

    #[test]
    fn clear_removes_all_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetaCachePersistence::new(
            dir.path().to_path_buf(),
            "test-compiler".to_string(),
        );
        cache.put("f", 1, 1, 1, &primitive(1));
        cache.put("g", 2, 2, 2, &primitive(2));
        cache.put("h", 3, 3, 3, &primitive(3));

        let removed = cache.clear();
        assert_eq!(removed, 3);
        assert!(cache.get(1, 1, 1).is_none());
        assert!(cache.get(2, 2, 2).is_none());
    }

    #[test]
    fn default_root_includes_compiler_version() {
        let dir = tempfile::tempdir().unwrap();
        let root = MetaCachePersistence::default_root(dir.path(), "1.2.3");
        assert!(
            root.to_string_lossy().contains("1.2.3"),
            "default_root must embed compiler version: {}",
            root.display()
        );
        assert!(
            root.to_string_lossy().contains("meta-cache"),
            "default_root must include meta-cache segment"
        );
    }
}
