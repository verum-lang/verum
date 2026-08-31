//! The script cache must notice when a MOUNTED module changed (T1003).
//!
//! The cache key digests the ENTRY FILE'S BYTES and nothing else, so a
//! script that mounts a sibling module keys identically before and after
//! that module is edited. Measured on the registry showcase: editing
//! `src/showcase/sizes.vr` and re-running `src/showcase/main.vr` printed
//! the PRE-EDIT text, and only an edit to `main.vr` itself picked the
//! change up. The entry's own bytes were already re-checked on lookup
//! (`CacheMeta::source_len`); the closure was not checked at all, so the
//! gate around that showcase had to append a throwaway comment to the
//! entry on every run to force a miss.
//!
//! Folding the closure INTO the key is not available: the key is needed
//! at lookup time, and the closure is not known until after the compile
//! that the lookup exists to avoid. So the closure is RECORDED on store
//! and VERIFIED on lookup.
//!
//! Both directions are pinned below. The negative one is not decoration:
//! a verification that never matches turns the cache into a permanent
//! miss, which behaves correctly, costs a full compile every run, and
//! looks exactly like a working cache from the outside.

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use verum_cli::script::cache::{CacheDep, ScriptCache, dep_hash};
use verum_cli::script::context::{ScriptContext, ScriptContextOptions};

const ENTRY_SRC: &str = "mount helper.{answer};\n\nfn main() {\n    print(answer());\n}\n";
const DEP_SRC: &str = "module helper;\n\npublic fn answer() -> Int {\n    1\n}\n";

/// Snapshot one dependency exactly as the compile path does: canonical
/// path, byte length, blake3 of the bytes.
fn dep_of(path: &Path) -> CacheDep {
    let bytes = fs::read(path).expect("read dependency");
    CacheDep {
        path: fs::canonicalize(path)
            .expect("canonicalise dependency")
            .display()
            .to_string(),
        len: bytes.len() as u64,
        hash: dep_hash(&bytes),
    }
}

struct Fixture {
    _tmp: TempDir,
    entry: PathBuf,
    dep: PathBuf,
    cache: ScriptCache,
}

impl Fixture {
    /// An entry that mounts one sibling module, with a stored cache
    /// entry whose closure names that sibling.
    fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let entry = tmp.path().join("main.vr");
        let dep = tmp.path().join("helper.vr");
        fs::write(&entry, ENTRY_SRC).expect("write entry");
        fs::write(&dep, DEP_SRC).expect("write dependency");
        let cache = ScriptCache::at(tmp.path().join("script-cache")).expect("cache root");

        let f = Fixture {
            _tmp: tmp,
            entry,
            dep,
            cache,
        };
        f.store(vec![dep_of(&f.dep)]);
        f
    }

    fn ctx(&self) -> ScriptContext {
        ScriptContext::from_path(&self.entry, &ScriptContextOptions::default())
            .expect("script context")
    }

    fn store(&self, deps: Vec<CacheDep>) {
        self.ctx()
            .cache_store(&self.cache, b"PRETEND-BYTECODE", deps)
            .expect("store");
    }

    fn hits(&self) -> bool {
        self.ctx()
            .cache_lookup(&self.cache)
            .expect("lookup")
            .is_some()
    }
}

#[test]
fn an_unchanged_closure_still_hits() {
    let f = Fixture::new();
    assert!(
        f.hits(),
        "nothing changed, so the entry must still be served — a closure check that \
         never matches is a permanent miss wearing a cache's clothes"
    );
}

#[test]
fn a_changed_mounted_module_is_a_miss() {
    let f = Fixture::new();
    assert!(f.hits(), "precondition: the fresh entry hits");

    // The edit that T1003 was invisible to. The ENTRY is untouched, so
    // the key is unchanged and the old behaviour was a hit.
    fs::write(&f.dep, DEP_SRC.replace("    1\n", "    2\n")).expect("edit dependency");

    assert!(
        !f.hits(),
        "a mounted module changed, so the stored bytecode no longer corresponds to \
         the sources and must not be served"
    );
}

#[test]
fn a_same_length_edit_is_still_a_miss() {
    let f = Fixture::new();
    // Length alone cannot decide this: `source_len` is the entry's own
    // pre-check, and a one-character swap keeps every length identical.
    // Only the digest separates these two files.
    let edited = DEP_SRC.replace("    1\n", "    7\n");
    assert_eq!(edited.len(), DEP_SRC.len(), "the edit must not change length");
    fs::write(&f.dep, edited).expect("edit dependency");

    assert!(!f.hits(), "a same-length edit to a mounted module must still miss");
}

#[test]
fn a_deleted_mounted_module_is_a_miss() {
    let f = Fixture::new();
    fs::remove_file(&f.dep).expect("delete dependency");

    assert!(
        !f.hits(),
        "a closure that cannot be re-verified is not trustworthy; recompiling \
         re-diagnoses the missing module instead of running yesterday's bytecode"
    );
}

#[test]
fn an_entry_with_no_mounts_is_unaffected() {
    let f = Fixture::new();
    // A single-file script records an empty closure. It must keep
    // hitting even while a neighbouring file churns — the check is
    // scoped to what the compile actually read, not to the directory.
    f.store(Vec::new());
    fs::write(&f.dep, "module helper;\n\npublic fn answer() -> Int {\n    99\n}\n")
        .expect("edit unrelated file");

    assert!(
        f.hits(),
        "an empty closure means the compile read nothing but the entry, so an \
         unrelated file's change must not invalidate it"
    );
}
