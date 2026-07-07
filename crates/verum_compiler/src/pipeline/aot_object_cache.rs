//! AOT whole-module native-object cache (AOT-STDLIB-NATIVE-CACHE-1).
//!

//! Every `verum <run|test> --aot` compile re-lowers and re-optimizes
//! the ENTIRE merged VBC module — user code plus the ~9K precompiled
//! stdlib functions that `merge_archive_function_bodies` folds into
//! the same compilation unit — even though the stdlib half is
//! byte-identical across runs.  That LLVM leg (VBC→IR lowering,
//! pass pipeline, machine codegen) is a measured multi-second fixed
//! baseline per test file (see AOT-BUDGET-1 in
//! `verum_cli::commands::test`).
//!

//! **Boundary choice: whole-module object.**  A stdlib-only object
//! split was investigated and rejected as too invasive for now:
//!
//!  * the archive is merged into the user module at the VBC level
//!  (one LLVM module), with per-compile func-id/string/const
//!  remapping;
//!  * lowering flips every defined function to INTERNAL linkage
//!  (vbc_lowering Phase 3.7) so globaldce can strip dead stdlib —
//!  a split needs the opposite (external stdlib symbols) plus
//!  canonical symbol naming (today arity collisions get
//!  compile-order-dependent `unique_name`s);
//!  * `platform_ir` emits the runtime helpers (allocator, text,
//!  main wrapper) into EVERY module, so two objects would carry
//!  duplicate symbols.
//!

//! Instead we cache the FINAL object file of the whole module,
//! keyed by the compile's deterministic INPUTS:
//!
//!  * the input source file bytes (for `verum test --aot` this is
//!  the synthesised merged test file — crate-root + test source +
//!  synthetic `fn main`, all deterministic);
//!  * the on-disk stdlib source tree (`CoreSource::auto_detect()`,
//!  path+content of every `.vr`, sorted) — disk `core/` edits
//!  feed mount resolution / typecheck without a compiler rebuild;
//!  * a compiler-binary stamp (version+len+mtime) — this also
//!  covers the EMBEDDED precompiled-stdlib archive + metadata
//!  (`include_bytes!` in `embedded_stdlib_vbc` / metadata), which
//!  change only with the binary;
//!  * a fingerprint of every lowering/target knob that can change
//!  the object (opt level, debug-info/coverage, panic strategy,
//!  codegen manifest flags, permission policy, target
//!  triple/CPU/features, the LLVM pass string).
//!
//! **Why not hash the (post-monomorph) VBC module itself?**  That
//! was the first implementation — and it NEVER hit: two compiles
//! of the same test produced serialized modules of identical
//! LENGTH (10001725 bytes measured) but different blake3 — the
//! codegen/merge/monomorph pipeline orders interned ids by HashMap
//! iteration, which is seeded per process.  The inputs, not the
//! intermediate, are the stable content.  Compiles with merged
//! project modules / external cogs (`self.project_modules`
//! non-empty) BYPASS the cache — their file set isn't hashed here.
//!
//! On a hit the entire LLVM leg is skipped and the cached object
//! is linked as-is; the only lowering-derived bit needed
//! downstream (`needs_metal`, the post-globaldce GPU-usage probe)
//! is persisted in a tiny sidecar `<key>.meta` file.
//!

//! This mainly accelerates REPEATED identical compiles — exactly
//! the shape of the meta AOT determinism gate (each file compiled
//! twice) and of re-running a test suite after edits elsewhere.
//!

//! Cache layout: `<project>/target/aot-object-cache/<blake3>.o`
//! plus `<blake3>.meta`.  Simple mtime-LRU eviction on insert
//! (caps: 50 entries / 500 MB).  Writes are temp-file + rename so
//! parallel `verum test --aot` subprocesses never observe a torn
//! object; the meta file is renamed LAST and required on fetch, so
//! meta-visible ⇒ object in place.  All cache failures degrade to
//! a normal compile (never fatal).
//!

//! Kill switch: `VERUM_NO_OBJECT_CACHE=1` disables read AND write.
//! `VERUM_TRACE_OBJECT_CACHE=1` prints key/hit/miss/timing lines to
//! stderr (used by the validation runs; the separate `vbc` hash in
//! the trace isolates VBC-serialization nondeterminism from
//! config-fingerprint drift when diagnosing unexpected misses).

use std::path::{Path, PathBuf};
use std::time::Instant;

use tracing::{debug, info};

/// Bump when the cached artifact contract changes (object contents,
/// meta format, key composition).  Old entries then miss and age out
/// via LRU.  v2: input-content key (source bytes + stdlib tree)
/// replacing the never-hitting serialized-VBC key of v1.
const CACHE_FORMAT_VERSION: u32 = 2;

/// LRU caps — enforced on insert, oldest-mtime first.
const MAX_ENTRIES: usize = 50;
const MAX_TOTAL_BYTES: u64 = 500 * 1024 * 1024;

/// Age after which an orphaned `*.tmp.<pid>` file (crashed writer)
/// is swept during eviction.
const STALE_TMP_SECS: u64 = 3600;

fn trace_enabled() -> bool {
    std::env::var_os("VERUM_TRACE_OBJECT_CACHE").is_some()
}

/// `true` when the object cache must not be consulted at all for
/// this compile.  LTO needs the live LLVM module for the bitcode
/// sidecar; the IR-dump paths exist to inspect a REAL compile.
pub(super) fn bypassed(lto: bool, emit_ir: bool) -> bool {
    std::env::var_os("VERUM_NO_OBJECT_CACHE").is_some()
        || lto
        || emit_ir
        || std::env::var_os("VERUM_DUMP_IR").is_some()
        || std::env::var_os("VERUM_DUMP_PRE_PASS").is_some()
}

/// Identity of the compiler binary producing objects.  Any rebuild
/// of `verum` (new LLVM, new lowering code) changes the stamp and
/// therefore every key; stale entries age out via LRU.
pub(super) fn compiler_stamp() -> String {
    let (len, mtime_nanos) = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(&p).ok())
        .map(|md| {
            let mtime = md
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            (md.len(), mtime)
        })
        .unwrap_or((0, 0));
    format!("{}|{}|{}", env!("CARGO_PKG_VERSION"), len, mtime_nanos)
}

/// A prepared cache handle: directory + fully-computed content key.
pub(super) struct AotObjectCache {
    dir: PathBuf,
    key: String,
}

/// Payload recovered on a cache hit — the lowering-derived facts the
/// post-object pipeline still needs.
pub(super) struct CacheHit {
    /// Post-globaldce `verum_metal_ensure_init` body probe (#100) —
    /// drives the Metal/Foundation/objc framework link decision.
    pub needs_metal: bool,
}

impl AotObjectCache {
    /// Derive the cache key from the compile's deterministic
    /// inputs.  Returns `None` (cache disabled for this compile)
    /// when the input file can't be read — never fatal.
    ///

    /// The key hashes, in order: the cache format version, the
    /// caller-built config fingerprint (compiler stamp + every
    /// out-of-module lowering knob), the input source bytes, and
    /// the resolved on-disk stdlib source tree (path + content of
    /// every file, sorted by path).  The embedded stdlib archive /
    /// metadata are covered by the compiler stamp inside the
    /// fingerprint (`include_bytes!` — they change only with the
    /// binary).
    ///

    /// Deliberately NOT keyed on the VBC module: its serialization
    /// is per-process-HashMap-order nondeterministic (measured:
    /// identical length, different bytes on every run), so a VBC
    /// key never hits.  The caller must BYPASS the cache when
    /// additional un-hashed sources are merged into the compile
    /// (project modules / external cogs).
    pub(super) fn prepare(
        target_dir: &Path,
        input_path: &Path,
        fingerprint: &str,
    ) -> Option<Self> {
        let t0 = Instant::now();
        let input_bytes = match std::fs::read(input_path) {
            Ok(b) => b,
            Err(e) => {
                debug!(
                    "aot-object-cache: cannot read input {} ({}) — cache disabled for this compile",
                    input_path.display(),
                    e
                );
                return None;
            }
        };

        // On-disk stdlib source stamp.  `auto_detect` resolves the
        // same root chain the pipeline's stdlib loading uses
        // (VERUM_CORE_PATH → ./core → <exe>/../core → ~/.verum/core
        // → ./stdlib); hashing path+content makes local stdlib
        // edits (which feed mount resolution and typecheck without
        // a compiler rebuild) miss the cache instead of reusing a
        // stale object.
        let mut stdlib_files = crate::core_source::CoreSource::auto_detect().load_all_source_files();
        stdlib_files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
        let mut stdlib_hasher = blake3::Hasher::new();
        stdlib_hasher.update(&(stdlib_files.len() as u64).to_le_bytes());
        for f in &stdlib_files {
            stdlib_hasher.update(&(f.path.as_str().len() as u64).to_le_bytes());
            stdlib_hasher.update(f.path.as_str().as_bytes());
            stdlib_hasher.update(&(f.content.as_str().len() as u64).to_le_bytes());
            stdlib_hasher.update(f.content.as_str().as_bytes());
        }
        let stdlib_hash = stdlib_hasher.finalize();

        let input_hash = blake3::hash(&input_bytes);
        let mut hasher = blake3::Hasher::new();
        hasher.update(&CACHE_FORMAT_VERSION.to_le_bytes());
        hasher.update(fingerprint.as_bytes());
        hasher.update(&(input_bytes.len() as u64).to_le_bytes());
        hasher.update(input_hash.as_bytes());
        hasher.update(stdlib_hash.as_bytes());
        let key = hasher.finalize().to_hex().to_string();
        if trace_enabled() {
            eprintln!(
                "[aot-object-cache] key={} input_bytes={} input_hash={} stdlib_files={} stdlib_hash={} fp_bytes={} prepare={:?}",
                key,
                input_bytes.len(),
                input_hash.to_hex(),
                stdlib_files.len(),
                stdlib_hash.to_hex(),
                fingerprint.len(),
                t0.elapsed(),
            );
        }
        Some(Self {
            dir: target_dir.join("aot-object-cache"),
            key,
        })
    }

    fn obj_path(&self) -> PathBuf {
        self.dir.join(format!("{}.o", self.key))
    }

    fn meta_path(&self) -> PathBuf {
        self.dir.join(format!("{}.meta", self.key))
    }

    /// Look the key up; on a hit, copy the cached object to
    /// `dest_obj` (the pipeline's regular `<module>.o` path) and
    /// return the sidecar facts.  Any inconsistency (missing /
    /// unreadable / version-mismatched meta, vanished object) is a
    /// clean miss.
    pub(super) fn try_fetch(&self, dest_obj: &Path) -> Option<CacheHit> {
        let t0 = Instant::now();
        // Meta is renamed last on store, so meta-present ⇒ the
        // object rename already happened.  (The object may still be
        // EVICTED concurrently — the failed copy below degrades to
        // a miss.)
        let meta = match std::fs::read_to_string(self.meta_path()) {
            Ok(m) => m,
            Err(_) => {
                if trace_enabled() {
                    eprintln!("[aot-object-cache] MISS key={}", self.key);
                }
                return None;
            }
        };
        let needs_metal = match parse_meta(&meta) {
            Some(nm) => nm,
            None => {
                if trace_enabled() {
                    eprintln!(
                        "[aot-object-cache] MISS (meta unparsable/version-mismatch) key={}",
                        self.key
                    );
                }
                return None;
            }
        };
        if let Err(e) = std::fs::copy(self.obj_path(), dest_obj) {
            if trace_enabled() {
                eprintln!(
                    "[aot-object-cache] MISS (object copy failed: {}) key={}",
                    e, self.key
                );
            }
            return None;
        }
        // LRU touch — mtime is the eviction ordering.
        touch(&self.obj_path());
        touch(&self.meta_path());
        info!(
            "  AOT object cache HIT ({}…) — reusing native object, skipping LLVM lowering/optimization/codegen",
            &self.key[..12.min(self.key.len())]
        );
        if trace_enabled() {
            eprintln!(
                "[aot-object-cache] HIT key={} fetch={:?}",
                self.key,
                t0.elapsed()
            );
        }
        Some(CacheHit { needs_metal })
    }

    /// Insert the freshly-emitted object + sidecar meta under this
    /// key, then run LRU eviction.  Best-effort: failures are logged
    /// and swallowed.
    pub(super) fn store(&self, obj_path: &Path, needs_metal: bool) {
        let t0 = Instant::now();
        if let Err(e) = self.store_inner(obj_path, needs_metal) {
            debug!("aot-object-cache: store failed (non-fatal): {}", e);
            if trace_enabled() {
                eprintln!(
                    "[aot-object-cache] STORE-FAILED key={} err={}",
                    self.key, e
                );
            }
            return;
        }
        if trace_enabled() {
            eprintln!(
                "[aot-object-cache] STORE key={} store={:?}",
                self.key,
                t0.elapsed()
            );
        }
        self.evict();
    }

    fn store_inner(&self, obj_path: &Path, needs_metal: bool) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let pid = std::process::id();
        // Object first, meta LAST — readers require the meta file,
        // so meta-visible ⇒ object already renamed into place.
        // Renames are atomic within the same directory.
        let obj_tmp = self.dir.join(format!("{}.o.tmp.{}", self.key, pid));
        std::fs::copy(obj_path, &obj_tmp)?;
        std::fs::rename(&obj_tmp, self.obj_path())?;

        let meta_tmp = self.dir.join(format!("{}.meta.tmp.{}", self.key, pid));
        std::fs::write(
            &meta_tmp,
            format!(
                "version={}\nneeds_metal={}\n",
                CACHE_FORMAT_VERSION,
                if needs_metal { 1 } else { 0 }
            ),
        )?;
        std::fs::rename(&meta_tmp, self.meta_path())?;
        Ok(())
    }

    /// mtime-LRU eviction: delete oldest entries until both caps
    /// hold.  Never evicts the entry just inserted.  Races with
    /// concurrent evictors/writers are tolerated (all removals are
    /// best-effort; readers treat a vanished object as a miss).
    fn evict(&self) {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        struct Entry {
            key: String,
            mtime: std::time::SystemTime,
            bytes: u64,
        }
        let mut objs: Vec<Entry> = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name.contains(".tmp.") {
                // Sweep orphaned temp files from crashed writers.
                let stale = e
                    .metadata()
                    .ok()
                    .and_then(|md| md.modified().ok())
                    .and_then(|m| m.elapsed().ok())
                    .map(|d| d.as_secs() > STALE_TMP_SECS)
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            if let Some(key) = name.strip_suffix(".o") {
                if let Ok(md) = e.metadata() {
                    objs.push(Entry {
                        key: key.to_string(),
                        mtime: md.modified().unwrap_or(std::time::UNIX_EPOCH),
                        bytes: md.len(),
                    });
                }
            }
        }
        let mut total: u64 = objs.iter().map(|o| o.bytes).sum();
        let mut count = objs.len();
        if count <= MAX_ENTRIES && total <= MAX_TOTAL_BYTES {
            return;
        }
        objs.sort_by_key(|o| o.mtime); // oldest first
        for o in &objs {
            if count <= MAX_ENTRIES && total <= MAX_TOTAL_BYTES {
                break;
            }
            if o.key == self.key {
                continue;
            }
            // Meta first so concurrent readers miss cleanly, then
            // the object.
            let _ = std::fs::remove_file(self.dir.join(format!("{}.meta", o.key)));
            let _ = std::fs::remove_file(self.dir.join(format!("{}.o", o.key)));
            count -= 1;
            total = total.saturating_sub(o.bytes);
            if trace_enabled() {
                eprintln!("[aot-object-cache] EVICT key={}", o.key);
            }
        }
    }
}

/// Parse the sidecar meta.  `None` on version mismatch or missing
/// fields (treated as a miss by the caller).
fn parse_meta(meta: &str) -> Option<bool> {
    let mut version_ok = false;
    let mut needs_metal = None;
    for line in meta.lines() {
        if let Some(v) = line.strip_prefix("version=") {
            version_ok = v.trim() == CACHE_FORMAT_VERSION.to_string();
        } else if let Some(v) = line.strip_prefix("needs_metal=") {
            needs_metal = match v.trim() {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            };
        }
    }
    if version_ok { needs_metal } else { None }
}

/// Best-effort mtime bump for LRU ordering.
fn touch(path: &Path) {
    let now = std::time::SystemTime::now();
    if let Ok(f) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = f.set_times(
            std::fs::FileTimes::new()
                .set_accessed(now)
                .set_modified(now),
        );
    }
}
