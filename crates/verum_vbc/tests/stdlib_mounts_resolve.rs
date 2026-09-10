//! Ratchet: every `mount .module.name` in the stdlib must name something the
//! target module actually contains.
//!
//! An unresolvable mount entry does NOT fail the stdlib bake — the archive
//! builds with it present, and the cost lands on a caller as a confusing
//! error at the use site.  That is the same silent shape as the unregistered
//! `@vbc` intrinsics frozen by `stdlib_vbc_intrinsics_registered.rs`.
//!
//! The roster's size is NOT restated here.  It said "266 entries" while the
//! array held 251, of which 191 no longer existed — a number in a comment is
//! the memory of a measurement, and this one was wrong in both directions at
//! once.  `KNOWN_DANGLING.len()` is the number; read it there.
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
    ("core/async/mod.vr", "stream", "Iter"),
    ("core/database/postgres/mod.vr", "connection", "SimpleQueryResult"),
    ("core/database/postgres/mod.vr", "row", "Row"),
    ("core/database/sqlite/native/l1_pager/mod.vr", "actor", "CheckpointMode"),
    ("core/database/sqlite/native/l1_pager/mod.vr", "db_header", "TextEncoding"),
    ("core/database/sqlite/native/l1_pager/mod.vr", "savepoint", "SavepointStack"),
    ("core/database/sqlite/native/l2_record/mod.vr", "type_coercion", "SqliteApiValue"),
    ("core/database/sqlite/native/l3_btree/mod.vr", "comparator", "ordering_name"),
    ("core/database/sqlite/native/l3_btree/mod.vr", "comparator", "reverse_ordering"),
    ("core/database/sqlite/native/l3_btree/mod.vr", "integrity", "IntegrityReport"),
    ("core/encoding/mod.vr", "der", "Time"),
    ("core/mem/mod.vr", "cap_audit", "record_attenuate"),
    ("core/mem/mod.vr", "cap_audit", "record_epoch_advance"),
    ("core/mem/mod.vr", "cap_audit", "record_gen_bump"),
    ("core/mem/mod.vr", "cap_audit", "record_ref_decr"),
    ("core/mem/mod.vr", "cap_audit", "record_ref_incr"),
    ("core/mem/mod.vr", "cap_audit", "record_revoke"),
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
    ("core/net/tls13/handshake/mod.vr", "client_sm", "ClientConfig"),
    ("core/protobuf/mod.vr", "wire", "Cursor"),
    ("core/runtime/mod.vr", "env", "IsolationLevel"),
    ("core/security/x509/mod.vr", "spki", "RsaPublicKey"),
    ("core/security/zk/halo2/mod.vr", "circuit", "ColumnType"),
    ("core/security/zk/halo2/mod.vr", "circuit", "Constraint"),
    ("core/sys/darwin/mod.vr", "io", "IoCqe"),
    ("core/sys/darwin/mod.vr", "io", "IoDriver"),
    ("core/sys/darwin/mod.vr", "io", "IoOp"),
    ("core/sys/darwin/mod.vr", "io", "IoOpKind"),
    ("core/sys/darwin/mod.vr", "io", "IoToken"),
    ("core/sys/darwin/mod.vr", "time", "Duration"),
    ("core/sys/darwin/mod.vr", "time", "Instant"),
    ("core/sys/darwin/mod.vr", "tls", "ContextEntry"),
    ("core/sys/darwin/mod.vr", "tls", "ContextSlots"),
    ("core/sys/darwin/mod.vr", "tls", "ThreadControlBlock"),
    ("core/sys/darwin/mod.vr", "tls", "TlsError"),
    ("core/sys/linux/mod.vr", "io", "IoDriver"),
    ("core/sys/linux/mod.vr", "time", "Duration"),
    ("core/sys/linux/mod.vr", "time", "Instant"),
    ("core/sys/linux/mod.vr", "time", "Timespec"),
    ("core/sys/linux/mod.vr", "time", "Timeval"),
    ("core/sys/mod.vr", "common", "MemoryOrdering"),
    ("core/sys/mod.vr", "io_engine", "Duration"),
    ("core/sys/mod.vr", "mmio", "Register"),
    ("core/term/raw/mod.vr", "capabilities", "ColorProfile"),
    ("core/term/render/mod.vr", "cell", "Cell"),
    ("core/term/render/mod.vr", "viewport", "Viewport"),
    ("core/term/widget/mod.vr", "gauge", "Gauge"),
    ("core/term/widget/mod.vr", "paragraph", "Span"),
    ("core/term/widget/mod.vr", "split", "Split"),
    ("core/term/widget/mod.vr", "table", "Row"),
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

/// The MIRROR of the check below, and the reason it exists is measured:
/// on 2026-09-10 the roster held 251 entries and **191 of them were no
/// longer extracted at all** — mount lines deleted, or produced by an
/// older `mounts_in` that did not strip comments and so froze English
/// words from the prose inside a multi-line mount block
/// (`core/sys/darwin/mod.vr:346` supplied twenty: `below`, `they`,
/// `types`, `per`, `rather`, …).
///
/// None of them could reach `resolving`, so the check below stayed
/// green and the roster decayed in silence. A ratchet that can only
/// shrink deliberately cannot shrink at all when 76% of its entries
/// are unfixable and nothing reports them.
///
/// The two directions are different questions and neither implies the
/// other: "this entry started resolving" is about the STDLIB changing,
/// "this entry is no longer extracted" is about the ROSTER or the
/// EXTRACTOR changing.
#[test]
fn every_known_dangling_entry_is_still_extracted() {
    let mut found = Vec::new();
    scan(&core_dir(), &mut found);
    assert!(!found.is_empty(), "the walk did not run");

    let extracted: BTreeSet<(&str, &str, &str)> = found
        .iter()
        .map(|(f, m, n, _)| (f.as_str(), m.as_str(), n.as_str()))
        .collect();

    let gone: Vec<String> = KNOWN_DANGLING
        .iter()
        .filter(|e| !extracted.contains(&(e.0, e.1, e.2)))
        .map(|e| format!("{}: mount .{}.{}", e.0, e.1, e.2))
        .collect();

    assert!(
        gone.is_empty(),
        "{} listed entry/entries are no longer produced by the scan at all \
         — the mount line is gone, or the extractor no longer yields that \
         name. Delete their lines from KNOWN_DANGLING; leaving them makes \
         the roster grow stale without any test noticing:\n  {}",
        gone.len(),
        gone.join("\n  ")
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
