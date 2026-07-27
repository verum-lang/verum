//! Ratchet: every `mount .module.name` in the stdlib must name something the
//! target module actually contains.
//!
//! An unresolvable mount entry does NOT fail the stdlib bake — the archive
//! builds with it present, and the cost lands on a caller as a confusing
//! error at the use site.  That is the same silent shape as the unregistered
//! `@vbc` intrinsics frozen by `stdlib_vbc_intrinsics_registered.rs`, and at
//! a larger scale: 266 entries here against 65 there.
//!
//! Four dispositions were established by hand while fixing the first few
//! (T0185), which is why this freezes rather than sweeps:
//!
//!   * nothing backs the name anywhere — delete it (`apply_rope`, 8dc2afe01);
//!   * it exists under a prefix — re-point with a rename, which the grammar
//!     supports (the five `.stream.*` constructors, 97de1dc9f);
//!   * it exists in a different sibling — move it to the right block
//!     (`Dim`, eb49c1925);
//!   * it is a deliberate forward declaration of another task's surface —
//!     leave it and say so (`StaticShape`/`DynShape`, handed to T0186).
//!
//! So a mechanical pass would be wrong in three of four cases.  This gate
//! only stops the set growing.
//!
//! The check is deliberately CONSERVATIVE: a name counts as present if it
//! appears anywhere in the target file, including in a comment.  It therefore
//! under-reports, and every entry below is one where the name appears nowhere
//! at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn core_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"))
}

/// Entries that do not resolve today, as (mod.vr path, target module, name).
/// Fixing one means deleting its line; the staleness test below fails if a
/// listed entry starts resolving, so the list can only shrink deliberately.
const KNOWN_DANGLING: &[(&str, &str, &str)] = &[
    ("core/async/mod.vr", "broadcast", "RecvError"),
    ("core/async/mod.vr", "broadcast", "SendError"),
    ("core/async/mod.vr", "stream", "Iter"),
    ("core/async/mod.vr", "timer", "Interval"),
    ("core/base/mod.vr", "iterator", "above"),
    ("core/base/mod.vr", "iterator", "constructors"),
    ("core/base/mod.vr", "iterator", "discipline"),
    ("core/base/mod.vr", "iterator", "every"),
    ("core/base/mod.vr", "iterator", "expected"),
    ("core/base/mod.vr", "iterator", "exported"),
    ("core/base/mod.vr", "iterator", "falls"),
    ("core/base/mod.vr", "iterator", "glob"),
    ("core/base/mod.vr", "iterator", "misses"),
    ("core/base/mod.vr", "iterator", "param"),
    ("core/base/mod.vr", "iterator", "parent"),
    ("core/base/mod.vr", "iterator", "prefix"),
    ("core/base/mod.vr", "iterator", "prelude"),
    ("core/base/mod.vr", "iterator", "re"),
    ("core/base/mod.vr", "iterator", "rigid"),
    ("core/base/mod.vr", "iterator", "try_resolve_variant_constructor"),
    ("core/base/mod.vr", "iterator", "typechecker"),
    ("core/base/mod.vr", "iterator", "wildcard"),
    ("core/base/mod.vr", "iterator", "write"),
    ("core/base/mod.vr", "memory", "Allocator"),
    ("core/base/mod.vr", "memory", "List"),
    ("core/base/mod.vr", "memory", "OOM"),
    ("core/base/mod.vr", "memory", "Raw"),
    ("core/base/mod.vr", "memory", "Set"),
    ("core/base/mod.vr", "memory", "Used"),
    ("core/base/mod.vr", "memory", "family"),
    ("core/base/mod.vr", "memory", "try_with_capacity"),
    ("core/base/mod.vr", "ops", "Note"),
    ("core/base/mod.vr", "ops", "accessed"),
    ("core/base/mod.vr", "ops", "variants"),
    ("core/base/mod.vr", "ops", "via"),
    ("core/database/postgres/mod.vr", "connection", "SimpleQueryResult"),
    ("core/database/postgres/mod.vr", "row", "Row"),
    ("core/database/sqlite/native/l1_pager/mod.vr", "actor", "CheckpointMode"),
    ("core/database/sqlite/native/l1_pager/mod.vr", "db_header", "TextEncoding"),
    ("core/database/sqlite/native/l1_pager/mod.vr", "savepoint", "SavepointStack"),
    ("core/database/sqlite/native/l1_pager/mod.vr", "wal_writer", "integration"),
    ("core/database/sqlite/native/l2_record/mod.vr", "type_coercion", "SqliteApiValue"),
    ("core/database/sqlite/native/l3_btree/mod.vr", "comparator", "ordering_name"),
    ("core/database/sqlite/native/l3_btree/mod.vr", "comparator", "reverse_ordering"),
    ("core/database/sqlite/native/l3_btree/mod.vr", "integrity", "IntegrityReport"),
    ("core/database/sqlite/native/l5_sql/mod.vr", "lexer", "LexError"),
    ("core/database/sqlite/native/l5_sql/mod.vr", "lexer", "Token"),
    ("core/encoding/mod.vr", "der", "Time"),
    ("core/math/mod.vr", "agent", "ParseError"),
    ("core/math/mod.vr", "agent", "Request"),
    ("core/math/mod.vr", "agent", "Tokenizer"),
    ("core/math/mod.vr", "agent", "framework"),
    ("core/math/mod.vr", "agent", "pattern"),
    ("core/math/mod.vr", "autodiff", "Contexts"),
    ("core/math/mod.vr", "autodiff", "modes"),
    ("core/math/mod.vr", "autodiff", "protocols"),
    ("core/math/mod.vr", "autodiff", "results"),
    ("core/math/mod.vr", "distributed", "DType"),
    ("core/math/mod.vr", "distributed", "Tensor"),
    ("core/math/mod.vr", "distributed", "parallelism"),
    ("core/math/mod.vr", "gpu", "Event"),
    ("core/math/mod.vr", "gpu", "Streams"),
    ("core/math/mod.vr", "gpu", "graphs"),
    ("core/math/mod.vr", "gpu", "identification"),
    ("core/math/mod.vr", "gpu", "spaces"),
    ("core/math/mod.vr", "guardrails", "Additional"),
    ("core/math/mod.vr", "guardrails", "filters"),
    ("core/math/mod.vr", "guardrails", "types"),
    ("core/math/mod.vr", "hott", "Path"),
    ("core/math/mod.vr", "linalg", "Types"),
    ("core/math/mod.vr", "nn", "Activations"),
    ("core/math/mod.vr", "nn", "Convolutions"),
    ("core/math/mod.vr", "nn", "functions"),
    ("core/math/mod.vr", "nn", "scaled_dot_product_attention"),
    ("core/math/mod.vr", "rag", "Chunking"),
    ("core/math/mod.vr", "rag", "types"),
    ("core/math/mod.vr", "random", "Generators"),
    ("core/math/mod.vr", "sdg", "Connection"),
    ("core/math/mod.vr", "tensor", "Concatenation"),
    ("core/math/mod.vr", "tensor", "Constructors"),
    ("core/math/mod.vr", "tensor", "Core"),
    ("core/math/mod.vr", "tensor", "DynShape"),
    ("core/math/mod.vr", "tensor", "DynTensorView"),
    ("core/math/mod.vr", "tensor", "DynTensorViewMut"),
    ("core/math/mod.vr", "tensor", "Logical"),
    ("core/math/mod.vr", "tensor", "StaticShape"),
    ("core/math/mod.vr", "tensor", "Strides"),
    ("core/math/mod.vr", "tensor", "TensorIndex"),
    ("core/math/mod.vr", "tensor", "TensorViewMut"),
    ("core/math/mod.vr", "tensor", "from_fn"),
    ("core/math/mod.vr", "tensor", "logical_and"),
    ("core/math/mod.vr", "tensor", "logical_not"),
    ("core/math/mod.vr", "tensor", "logical_or"),
    ("core/math/mod.vr", "tensor", "types"),
    ("core/math/mod.vr", "tensor", "where_cond"),
    ("core/mem/mod.vr", "allocator", "StackAllocator"),
    ("core/mem/mod.vr", "allocator", "configurations"),
    ("core/mem/mod.vr", "allocator", "integration"),
    ("core/mem/mod.vr", "allocator", "protocols"),
    ("core/mem/mod.vr", "cap_audit", "record_attenuate"),
    ("core/mem/mod.vr", "cap_audit", "record_epoch_advance"),
    ("core/mem/mod.vr", "cap_audit", "record_gen_bump"),
    ("core/mem/mod.vr", "cap_audit", "record_ref_decr"),
    ("core/mem/mod.vr", "cap_audit", "record_ref_incr"),
    ("core/mem/mod.vr", "cap_audit", "record_revoke"),
    ("core/mem/mod.vr", "capability", "Containment"),
    ("core/mem/mod.vr", "capability", "Packed"),
    ("core/mem/mod.vr", "capability", "Predicates"),
    ("core/mem/mod.vr", "capability", "Preset"),
    ("core/mem/mod.vr", "capability", "Single"),
    ("core/mem/mod.vr", "capability", "Sub"),
    ("core/mem/mod.vr", "capability", "constructors"),
    ("core/mem/mod.vr", "capability", "consumed"),
    ("core/mem/mod.vr", "capability", "creation"),
    ("core/mem/mod.vr", "capability", "encoding"),
    ("core/mem/mod.vr", "capability", "masks"),
    ("core/mem/mod.vr", "capability", "over"),
    ("core/mem/mod.vr", "capability", "paths"),
    ("core/mem/mod.vr", "capability", "predicates"),
    ("core/mem/mod.vr", "capability", "sets"),
    ("core/mem/mod.vr", "epoch", "deferral"),
    ("core/mem/mod.vr", "heap", "exported"),
    ("core/mem/mod.vr", "heap", "re"),
    ("core/mesh/k8s/mod.vr", "gateway", "Listener"),
    ("core/mesh/xds/mod.vr", "client", "Subscription"),
    ("core/mesh/xds/mod.vr", "resources", "Listener"),
    ("core/mesh/xds/mod.vr", "types", "Node"),
    ("core/meta/mod.vr", "contexts", "ParseError"),
    ("core/net/h3/qpack/mod.vr", "static_table", "StaticEntry"),
    ("core/net/mod.vr", "http", "Url"),
    ("core/net/mod.vr", "tls", "TlsError"),
    ("core/net/quic/api/mod.vr", "stream", "QuicStream"),
    ("core/net/quic/mod.vr", "address_token", "TokenKind"),
    ("core/net/quic/mod.vr", "address_token", "VerifyOptions"),
    ("core/net/quic/stream_sm/mod.vr", "recv", "RecvError"),
    ("core/net/quic/stream_sm/mod.vr", "send", "SendError"),
    ("core/net/tls13/handshake/mod.vr", "client_sm", "ClientConfig"),
    ("core/protobuf/mod.vr", "wire", "Cursor"),
    ("core/runtime/mod.vr", "config", "implementations"),
    ("core/runtime/mod.vr", "env", "IsolationLevel"),
    ("core/security/x509/mod.vr", "spki", "RsaPublicKey"),
    ("core/security/zk/halo2/mod.vr", "circuit", "ColumnType"),
    ("core/security/zk/halo2/mod.vr", "circuit", "Constraint"),
    ("core/security/zk/halo2/mod.vr", "circuit", "Helpers"),
    ("core/security/zk/halo2/mod.vr", "circuit", "container"),
    ("core/security/zk/halo2/mod.vr", "circuit", "generic"),
    ("core/security/zk/halo2/mod.vr", "circuit", "primitives"),
    ("core/security/zk/halo2/mod.vr", "circuit", "shapes"),
    ("core/sys/darwin/mod.vr", "errno", "Common"),
    ("core/sys/darwin/mod.vr", "errno", "Non"),
    ("core/sys/darwin/mod.vr", "errno", "errors"),
    ("core/sys/darwin/mod.vr", "errno", "functions"),
    ("core/sys/darwin/mod.vr", "io", "Functions"),
    ("core/sys/darwin/mod.vr", "io", "IoCqe"),
    ("core/sys/darwin/mod.vr", "io", "IoDriver"),
    ("core/sys/darwin/mod.vr", "io", "IoOp"),
    ("core/sys/darwin/mod.vr", "io", "IoOpKind"),
    ("core/sys/darwin/mod.vr", "io", "IoToken"),
    ("core/sys/darwin/mod.vr", "libsystem", "SockaddrIn6"),
    ("core/sys/darwin/mod.vr", "libsystem", "Stat"),
    ("core/sys/darwin/mod.vr", "libsystem", "Timespec"),
    ("core/sys/darwin/mod.vr", "libsystem", "Timeval"),
    ("core/sys/darwin/mod.vr", "mach", "policies"),
    ("core/sys/darwin/mod.vr", "mach", "returns"),
    ("core/sys/darwin/mod.vr", "mach", "states"),
    ("core/sys/darwin/mod.vr", "thread", "Condvar"),
    ("core/sys/darwin/mod.vr", "thread", "Constants"),
    ("core/sys/darwin/mod.vr", "thread", "Functions"),
    ("core/sys/darwin/mod.vr", "thread", "Mutex"),
    ("core/sys/darwin/mod.vr", "thread", "Once"),
    ("core/sys/darwin/mod.vr", "thread", "SpinLock"),
    ("core/sys/darwin/mod.vr", "thread", "Thread"),
    ("core/sys/darwin/mod.vr", "thread", "ThreadError"),
    ("core/sys/darwin/mod.vr", "thread", "ThreadFn"),
    ("core/sys/darwin/mod.vr", "thread", "Types"),
    ("core/sys/darwin/mod.vr", "time", "DeadlineTimer"),
    ("core/sys/darwin/mod.vr", "time", "Duration"),
    ("core/sys/darwin/mod.vr", "time", "Instant"),
    ("core/sys/darwin/mod.vr", "time", "PerfCounter"),
    ("core/sys/darwin/mod.vr", "time", "Stopwatch"),
    ("core/sys/darwin/mod.vr", "time", "The"),
    ("core/sys/darwin/mod.vr", "time", "Types"),
    ("core/sys/darwin/mod.vr", "time", "Utilities"),
    ("core/sys/darwin/mod.vr", "time", "bare"),
    ("core/sys/darwin/mod.vr", "time", "below"),
    ("core/sys/darwin/mod.vr", "time", "concrete"),
    ("core/sys/darwin/mod.vr", "time", "cross"),
    ("core/sys/darwin/mod.vr", "time", "declaration"),
    ("core/sys/darwin/mod.vr", "time", "declared"),
    ("core/sys/darwin/mod.vr", "time", "exported"),
    ("core/sys/darwin/mod.vr", "time", "functions"),
    ("core/sys/darwin/mod.vr", "time", "have"),
    ("core/sys/darwin/mod.vr", "time", "items"),
    ("core/sys/darwin/mod.vr", "time", "names"),
    ("core/sys/darwin/mod.vr", "time", "per"),
    ("core/sys/darwin/mod.vr", "time", "platform"),
    ("core/sys/darwin/mod.vr", "time", "rather"),
    ("core/sys/darwin/mod.vr", "time", "re"),
    ("core/sys/darwin/mod.vr", "time", "their"),
    ("core/sys/darwin/mod.vr", "time", "they"),
    ("core/sys/darwin/mod.vr", "time", "types"),
    ("core/sys/darwin/mod.vr", "time", "under"),
    ("core/sys/darwin/mod.vr", "tls", "ContextEntry"),
    ("core/sys/darwin/mod.vr", "tls", "ContextSlots"),
    ("core/sys/darwin/mod.vr", "tls", "ThreadControlBlock"),
    ("core/sys/darwin/mod.vr", "tls", "TlsError"),
    ("core/sys/darwin/mod.vr", "tls", "Types"),
    ("core/sys/darwin/mod.vr", "tls", "Utilities"),
    ("core/sys/linux/mod.vr", "auxv", "Convenience"),
    ("core/sys/linux/mod.vr", "auxv", "constants"),
    ("core/sys/linux/mod.vr", "auxv", "functions"),
    ("core/sys/linux/mod.vr", "errno", "Common"),
    ("core/sys/linux/mod.vr", "errno", "errors"),
    ("core/sys/linux/mod.vr", "errno", "functions"),
    ("core/sys/linux/mod.vr", "io", "IoDriver"),
    ("core/sys/linux/mod.vr", "io", "constants"),
    ("core/sys/linux/mod.vr", "mem", "constants"),
    ("core/sys/linux/mod.vr", "mem", "functions"),
    ("core/sys/linux/mod.vr", "mem", "utilities"),
    ("core/sys/linux/mod.vr", "syscall", "Basic"),
    ("core/sys/linux/mod.vr", "syscall", "Intrinsic"),
    ("core/sys/linux/mod.vr", "syscall", "Sleep"),
    ("core/sys/linux/mod.vr", "syscall", "advanced"),
    ("core/sys/linux/mod.vr", "syscall", "futex"),
    ("core/sys/linux/mod.vr", "syscall", "info"),
    ("core/sys/linux/mod.vr", "syscall", "mapping"),
    ("core/sys/linux/mod.vr", "thread", "Condvar"),
    ("core/sys/linux/mod.vr", "thread", "Extended"),
    ("core/sys/linux/mod.vr", "thread", "Mutex"),
    ("core/sys/linux/mod.vr", "thread", "SpinLock"),
    ("core/sys/linux/mod.vr", "thread", "Thread"),
    ("core/sys/linux/mod.vr", "thread", "ThreadError"),
    ("core/sys/linux/mod.vr", "thread", "ThreadFn"),
    ("core/sys/linux/mod.vr", "time", "Duration"),
    ("core/sys/linux/mod.vr", "time", "Instant"),
    ("core/sys/linux/mod.vr", "time", "Stopwatch"),
    ("core/sys/linux/mod.vr", "time", "Timespec"),
    ("core/sys/linux/mod.vr", "time", "Timeval"),
    ("core/sys/linux/mod.vr", "time", "Types"),
    ("core/sys/linux/mod.vr", "time", "Utilities"),
    ("core/sys/linux/mod.vr", "time", "bare"),
    ("core/sys/linux/mod.vr", "time", "concrete"),
    ("core/sys/linux/mod.vr", "time", "cross"),
    ("core/sys/linux/mod.vr", "time", "declaration"),
    ("core/sys/linux/mod.vr", "time", "export"),
    ("core/sys/linux/mod.vr", "time", "no"),
    ("core/sys/linux/mod.vr", "time", "platform"),
    ("core/sys/linux/mod.vr", "time", "re"),
    ("core/sys/linux/mod.vr", "time", "there"),
    ("core/sys/linux/mod.vr", "time", "timer"),
    ("core/sys/linux/mod.vr", "tls", "Types"),
    ("core/sys/mod.vr", "common", "MemoryOrdering"),
    ("core/sys/mod.vr", "init", "Error"),
    ("core/sys/mod.vr", "init", "functions"),
    ("core/sys/mod.vr", "init", "handling"),
    ("core/sys/mod.vr", "io_engine", "Duration"),
    ("core/sys/mod.vr", "mmio", "Register"),
    ("core/term/raw/mod.vr", "capabilities", "ColorProfile"),
    ("core/term/render/mod.vr", "cell", "Cell"),
    ("core/term/render/mod.vr", "viewport", "Viewport"),
    ("core/term/widget/mod.vr", "gauge", "Gauge"),
    ("core/term/widget/mod.vr", "paragraph", "Span"),
    ("core/term/widget/mod.vr", "split", "Split"),
    ("core/term/widget/mod.vr", "table", "Row"),
    ("core/text/mod.vr", "format", "Alignment"),
    ("core/theory_interop/mod.vr", "core", "Registry"),
];

/// Every `mount .<module>.<name>` and `mount .<module>.{a, b, …}` in `text`.
///
/// Scanned by hand rather than with a regex to avoid a dev-dependency, and
/// `//` comments are stripped first: prose naming a mount is not a mount.
fn mounts_in(text: &str) -> Vec<(String, String)> {
    let code: String = text
        .lines()
        .map(|l| l.split_once("//").map_or(l, |(before, _)| before))
        .collect::<Vec<_>>()
        .join("\n");

    let mut out = Vec::new();
    let mut rest = code.as_str();
    while let Some(at) = rest.find("mount .") {
        rest = &rest[at + "mount .".len()..];
        let module: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if module.is_empty() {
            continue;
        }
        rest = &rest[module.len()..];
        if !rest.starts_with('.') {
            continue;
        }
        rest = &rest[1..];
        if let Some(stripped) = rest.strip_prefix('{') {
            let Some(end) = stripped.find('}') else { continue };
            for item in stripped[..end].split(',') {
                let name: String = item
                    .trim()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    out.push((module.clone(), name));
                }
            }
            rest = &stripped[end..];
        } else {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() {
                out.push((module.clone(), name));
            }
        }
    }
    out
}

/// Walk `core/` collecting every mount that names a sibling module which
/// exists, paired with whether the name appears in it.
fn scan(dir: &Path, out: &mut Vec<(String, String, String, bool)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, out);
        } else if path.file_name().is_some_and(|n| n == "mod.vr") {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let rel = path
                .strip_prefix(core_dir().parent().unwrap())
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            for (module, name) in mounts_in(&text) {
                let target = path.with_file_name(format!("{module}.vr"));
                let Ok(target_text) = std::fs::read_to_string(&target) else { continue };
                let present = target_text
                    .match_indices(&name)
                    .any(|(i, _)| {
                        let before = target_text[..i].chars().next_back();
                        let after = target_text[i + name.len()..].chars().next();
                        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                        !word(before) && !word(after)
                    });
                out.push((rel.clone(), module, name, present));
            }
        }
    }
}

#[test]
fn no_new_dangling_mounts_in_the_stdlib() {
    let mut found = Vec::new();
    scan(&core_dir(), &mut found);
    assert!(
        found.len() > 1000,
        "scanned only {} mount entries — the walk did not run, and an empty \
         walk would pass this gate vacuously",
        found.len()
    );

    let known: BTreeSet<(&str, &str, &str)> = KNOWN_DANGLING.iter().copied().collect();
    let mut unexpected: Vec<String> = found
        .iter()
        .filter(|(_, _, _, present)| !present)
        .filter(|(f, m, n, _)| !known.contains(&(f.as_str(), m.as_str(), n.as_str())))
        .map(|(f, m, n, _)| format!("{f}: mount .{m}.{n}"))
        .collect();
    unexpected.sort();
    unexpected.dedup();

    assert!(
        unexpected.is_empty(),
        "{} mount entry/entries name something their target module does not \
         contain:\n  {}\n\nThe bake will NOT catch this — an unresolvable \
         mount builds fine and fails at the caller. Point it at what exists \
         (a rename is allowed: `mount .m.real as public_name`), or add it to \
         KNOWN_DANGLING with the reason.",
        unexpected.len(),
        unexpected.join("\n  ")
    );
}

#[test]
fn the_known_dangling_list_has_no_stale_entries() {
    let mut found = Vec::new();
    scan(&core_dir(), &mut found);
    assert!(!found.is_empty(), "the walk did not run");

    let resolving: BTreeSet<(&str, &str, &str)> = found
        .iter()
        .filter(|(_, _, _, present)| *present)
        .map(|(f, m, n, _)| (f.as_str(), m.as_str(), n.as_str()))
        .collect();

    let fixed: Vec<String> = KNOWN_DANGLING
        .iter()
        .filter(|e| resolving.contains(&(e.0, e.1, e.2)))
        .map(|e| format!("{}: mount .{}.{}", e.0, e.1, e.2))
        .collect();

    assert!(
        fixed.is_empty(),
        "{} listed entry/entries now resolve and must be deleted from \
         KNOWN_DANGLING so the ratchet keeps tightening:\n  {}",
        fixed.len(),
        fixed.join("\n  ")
    );
}
