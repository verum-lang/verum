//! Guardrail: every public stdlib type must have a unique simple name
//! across all of `core/`.
//!
//! Background. The VBC codegen indexes the variant-constructor table by
//! simple type name (`Type.Variant`).  Two stdlib `public type Foo is …`
//! declarations with the same simple name in different modules collide:
//! `register_type_constructors` runs with `prefer_existing_functions =
//! true` (stdlib loading mode), hits the `has_variants_for_type` first-
//! wins gate on the second module, and silently skips the second type's
//! variant registration entirely.
//!
//! Symptom: bodies that write `MyType { lock_state: Unlocked, … }`
//! compile to `[lenient] SKIP <fn>: undefined variable: Unlocked` and
//! disappear from the runtime function table.  Callers later panic with
//! `method 'X.Y' not found on value`, far from the source.
//!
//! Concrete past incident (task #160): three stdlib modules each defined
//! `public type LockKind`:
//!
//!   * `core/database/sqlite/native/l0_vfs/vfs_protocol.vr` —
//!     5-state SQLite VFS protocol (Unlocked | Shared | Reserved |
//!     Pending | Exclusive)
//!   * `core/sys/common.vr` — fcntl byte-range lock
//!     (Shared | Exclusive | Unlock)
//!   * `core/sys/locking/mod.vr` — high-level file lock
//!     (Shared | Exclusive)
//!
//! The first to register won the simple-name slot.  Whichever module
//! lost had its variants invisible to every body that referenced them
//! by simple name, including bodies inside its own module.
//!
//! Resolution: the two non-canonical types were renamed to
//! `FcntlLockKind` (sys/common) and `FileLockKind` (sys/locking).  This
//! test pins the policy: any future stdlib type whose simple name
//! collides with another existing stdlib type must pick a different
//! name (prefix-disambiguated by domain — see e.g. `LkNone`, `FxNone`,
//! `EpNone`, `AvNone`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = "core";

/// Ratchet baseline: a snapshot of the *unique* type names that
/// currently appear in two or more stdlib modules.  The codegen
/// variant-constructor table is keyed by simple type name, so every
/// duplicate is a latent collision — the second declaration's variants
/// either silently lose to first-wins or get clobbered, depending on
/// which mode `register_type_constructors` runs in.
///
/// **Policy.** This list MAY shrink (rename a duplicate to a unique
/// disambiguated name) but it MUST NOT grow.  Every NEW `public type X`
/// added to `core/` must pick a name that doesn't already appear
/// anywhere else under `core/`.  See task #160 in the developer log
/// for the underlying class of bugs (silent variant skip, surfaces as
/// runtime `method 'X.Y' not found on value`).
///
/// **How to remove an entry.** Pick a domain-prefixed disambiguated
/// name following the existing stdlib convention (`LkNone` / `FxNone`
/// / `AvNone` / `EpNone` / `FmNone` for sqlite catalogue variants;
/// `FcntlLockKind` / `FileLockKind` / `VfsLockKind` for the LockKind
/// triplet).  Update every site that imports the old name. Then drop
/// the entry from this baseline and run the test to confirm the
/// duplicate is gone.
const BASELINE_DUPLICATES: &[&str] = &[
    "AccessMode", "Affinity", "AllocHandle",
    "ApplyOutcome", "Args",
    "Block",
    "CacheMode", "Category",
    "CheckpointMode", "ChildSpecOpaque",
    "CircuitBreaker", "CircuitBreakerConfig", "CircuitBreakerOpaque",
    "CircuitState",
    "Condvar", "Config",
    "Connection",
    "ConstraintOp", "ContextEntry", "ContextError", "ContextSlots",
    "Counter", "CpuContext", "CpuFeatures",
    "Cursor", "DbError", "DeadlineTimer",
    "Duration", "Edge",
    "ElementType", "Empty", "EnginePhase", "Event",
    "ExceptionFrame", "ExecutionEnv", "ExecutionEnvOpaque",
    "ExecutorHandle", "ExportResult", "Expr", "Fault",
    "FkAction",
    "Formatter", "Frame", "FrameHeader",
    "FutureHandle", "Gauge", "Generator", "H3Frame", "HeaderField",
    "Histogram", "HistogramSnapshot",
    "InlineCircuitBreaker", "InlineRetryPolicy", "Instant",
    "IntegrityReport", "IoCqe", "IoDriver", "IoDriverError",
    "IODriverHandle", "IoError", "IoOp", "IoOpKind", "IoToken", "IpMreq",
    "IsolationLevel", "JoinHandle", "JoinHandleOpaque",
    "JournalMode", "Json", "KanDirection",
    "KanExtensionResult", "Keyword", "Layout",
    "Lifetime", "Listener", "Literal",
    "LocalHeap", "LockRegion", "LogLevel", "LogRecord",
    "MemoryOrdering",
    "Message", "Metadata", "Method", "MethodSet",
    "MiddlewareChainOpaque", "Mode", "Modifier", "Mutex", "Node",
    "ObstructionData", "ObstructionPoint", "Once", "OnConflict", "Op",
    "Opcode", "OpenMode", "OpStream",
    "OtlpExporter", "Outcome", "PageHeader",
    "PanicInfo",
    "ParseError", "ParseState", "Path",
    "PerfCounter", "Phase", "PipelineReport", "PlanError",
    "PlatformIOEngine", "PorterTokenizer",
    "Prop",
    "Reading", "ReadyFuture",
    "RecoveryContextOpaque", "RecoveryError", "RecoveryStrategy",
    "RecvError", "Register", "Registration", "Request",
    "Resource", "RestartPolicy", "RestartStrategy",
    "RetryConfig", "Role", "RowShape",
    "RTree", "Runtime", "SavepointStack", "SchemaError",
    "SchemaKind", "Segment", "SendError",
    "Session", "SetOutcome",
    "SharedRegistryOpaque", "Signal", "SimpleTokenizer",
    "SingleThreadExecutorOpaque", "Snapshot",
    "SockaddrIn", "SockaddrIn6",
    "Span", "SpawnConfig",
    "SpinLock", "Split", "SqliteValue", "StackAllocator",
    "Stat", "State", "StaticEntry", "StatusCode",
    "StepOutcome", "StepResult", "StmtKind", "Stopwatch",
    "Subscription", "SupervisionStrategy",
    "SupervisorHandleOpaque", "SystemTime",
    "TaskHandle", "TaskId", "Tensor",
    "Thread", "ThreadControlBlock", "ThreadError",
    "ThreadFn", "ThreadId", "ThreadPool", "Timeout",
    "TimeoutError", "Timespec", "Timeval", "TlsError", "Token",
    "TriggerEvent",
    "TriggerTiming", "TrigramTokenizer", "TxnState", "Unicode61Tokenizer",
    "UniqueConstraint", "UpdateOp", "Url", "ValidationError",
    "Value", "VarintVector", "Verdict",
    "YieldNow",
];

#[test]
fn stdlib_public_type_names_ratchet() {
    let root = workspace_root().join(ROOT);
    assert!(
        root.is_dir(),
        "expected stdlib root at {} but it does not exist",
        root.display()
    );

    // name -> List<(module-path, file-path, line)>
    let mut definitions: BTreeMap<String, Vec<(String, PathBuf, usize)>> = BTreeMap::new();
    walk_dir(&root, &root, &mut definitions);

    let baseline: std::collections::HashSet<&str> =
        BASELINE_DUPLICATES.iter().copied().collect();
    let mut current_duplicates: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    let mut new_violations: Vec<String> = Vec::new();
    for (name, sites) in &definitions {
        if sites.len() < 2 {
            continue;
        }
        current_duplicates.insert(name.clone());
        if baseline.contains(name.as_str()) {
            continue;
        }
        let mut entry = format!(
            "NEW duplicate: type `{}` is declared in {} stdlib modules:\n",
            name,
            sites.len()
        );
        for (modpath, file, line) in sites {
            entry.push_str(&format!("    - {} ({}:{})\n", modpath, file.display(), line));
        }
        new_violations.push(entry);
    }

    if !new_violations.is_empty() {
        panic!(
            "{} new duplicated public type name(s) introduced.  Each \
             public stdlib type must have a unique simple name across \
             all of `core/`, because the VBC variant-constructor table \
             is keyed by simple name.  Two `public type Foo is …` \
             declarations collide and the second's variants are silently \
             skipped, surfacing later as runtime \
             `method 'X.Y' not found on value` panics.\n\n\
             Pick a domain-prefixed disambiguated name following the \
             existing stdlib convention (e.g. `FcntlLockKind`, \
             `FileLockKind`, `LkNone`, `FxNone`).\n\n\
             {}",
            new_violations.len(),
            new_violations.join("\n"),
        );
    }

    // Stale-baseline check: catch entries listed in BASELINE_DUPLICATES
    // that are no longer duplicates.  Forces the baseline to shrink as
    // renames happen instead of silently tracking dead names.
    let stale: Vec<&&str> = BASELINE_DUPLICATES
        .iter()
        .filter(|n| !current_duplicates.contains(**n))
        .collect();
    if !stale.is_empty() {
        let names: Vec<String> = stale.iter().map(|s| (**s).to_string()).collect();
        panic!(
            "{} entries in `BASELINE_DUPLICATES` are no longer duplicated \
             and must be removed from the baseline:\n  {}\n\nRemove them \
             from the array in `crates/verum_compiler/tests/\
             stdlib_unique_type_names.rs` so the ratchet shrinks.",
            names.len(),
            names.join("\n  "),
        );
    }
}

/// Walk a directory recursively, collecting `public type Name is …`
/// declarations.  Skips test-helper paths under `vcs/`, generated
/// `target/` directories, and anything under `.git/`.
fn walk_dir(
    repo_core_root: &Path,
    dir: &Path,
    sink: &mut BTreeMap<String, Vec<(String, PathBuf, usize)>>,
) {
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(repo_core_root, &path, sink);
        } else if path.extension().and_then(|s| s.to_str()) == Some("vr") {
            scan_file(repo_core_root, &path, sink);
        }
    }
}

/// Scan a single `.vr` file for `public type Name is …` declarations.
/// Records each match under its simple name in `sink`.
///
/// Heuristic-only — the test is intentionally lenient about formatting
/// (whitespace, generics, body kind).  Excludes `protocol` types since
/// they cannot collide via the variant-constructor mechanism (no
/// constructors).
fn scan_file(
    repo_core_root: &Path,
    file: &Path,
    sink: &mut BTreeMap<String, Vec<(String, PathBuf, usize)>>,
) {
    let contents = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let module_path = file_to_module_path(repo_core_root, file);
    for (lineno, line) in contents.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("public type ") {
            continue;
        }
        let after_kw = &trimmed["public type ".len()..];
        // Strip leading optional `affine`/`unique` qualifiers.
        let after_kw = after_kw
            .strip_prefix("affine ")
            .or_else(|| after_kw.strip_prefix("unique "))
            .unwrap_or(after_kw);
        // Take the type name up to first non-identifier character.
        let mut end = 0;
        for (i, c) in after_kw.char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end = i + c.len_utf8();
            } else {
                break;
            }
        }
        if end == 0 {
            continue;
        }
        let name = &after_kw[..end];
        // Skip protocol types — `public type Foo is protocol { … }` —
        // they have no variant constructors so cannot trip this bug.
        let rest = after_kw[end..].trim_start();
        let rest = rest.strip_prefix('<').map_or(rest, |r| {
            // Skip generic params `<…>`.
            let mut depth = 1;
            let mut cut = 0;
            for (i, c) in r.char_indices() {
                match c {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            cut = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            &r[cut..]
        });
        let rest = rest.trim_start().strip_prefix("is").unwrap_or(rest).trim_start();
        if rest.starts_with("protocol") {
            continue;
        }
        sink.entry(name.to_string()).or_default().push((
            module_path.clone(),
            file.to_path_buf(),
            lineno + 1,
        ));
    }
}

/// Convert a file path under `core/` into its dotted module path:
///
///   core/sys/common.vr           -> core.sys.common
///   core/sys/locking/mod.vr      -> core.sys.locking
fn file_to_module_path(repo_core_root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(repo_core_root).unwrap_or(file);
    let mut parts: Vec<String> = vec!["core".to_string()];
    for component in rel.components() {
        if let std::path::Component::Normal(s) = component
            && let Some(s) = s.to_str()
        {
            let s = s.strip_suffix(".vr").unwrap_or(s);
            parts.push(s.to_string());
        }
    }
    let joined = parts.join(".");
    joined.trim_end_matches(".mod").to_string()
}

fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in crate_dir.ancestors() {
        if ancestor.join("Cargo.lock").is_file() && ancestor.join(ROOT).is_dir() {
            return ancestor.to_path_buf();
        }
    }
    panic!(
        "workspace root with Cargo.lock and {ROOT}/ not found from {}",
        crate_dir.display()
    );
}
