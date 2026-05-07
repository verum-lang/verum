//! Embedded precompiled stdlib VBC archive.
//!
//! Phase 5 of the precompiled-stdlib archive epic. Embeds the
//! `.vbca` artefact produced by Phase 4 (`verum stdlib precompile`)
//! into the compiler binary at build time. Phase 6 consumes this
//! through [`get_runtime_archive`] in `compile_ast_to_vbc` to skip
//! source-driven stdlib codegen.
//!
//! When the embedded bytes are empty (build script reports archive
//! missing), [`get_runtime_archive`] returns `None` and callers fall
//! back to the legacy source-archive path.

use std::sync::OnceLock;

use verum_vbc::archive::{VbcArchive, read_archive_from_file};
use verum_vbc::error::VbcResult;

/// Compressed archive bytes embedded at build time. Sourced from
/// `target/precompiled-stdlib/runtime.vbca`. Empty when no archive
/// was available at build time.
static EMBEDDED_RUNTIME_VBC: &[u8] = include_bytes!(env!("STDLIB_RUNTIME_VBC_PATH"));

/// Lazily decoded archive. First reader pays the zstd-decompress + bincode-
/// deserialise cost (~5-50 ms for ~10 MB archive); every subsequent reader
/// gets a `&'static VbcArchive`.
static RUNTIME_ARCHIVE: OnceLock<Option<VbcArchive>> = OnceLock::new();

/// Returns the embedded runtime VBC archive, or `None` when the
/// compiler binary was built without one (`target/precompiled-stdlib/
/// runtime.vbca` missing at build time).
///
/// Decoded once per process via `OnceLock`. Subsequent calls are
/// O(1) pointer reads.
pub fn get_runtime_archive() -> Option<&'static VbcArchive> {
    RUNTIME_ARCHIVE
        .get_or_init(|| {
            if EMBEDDED_RUNTIME_VBC.is_empty() {
                tracing::debug!(
                    target: "embedded_stdlib_vbc",
                    "no precompiled stdlib archive embedded — falling back to source compile"
                );
                return None;
            }
            let t0 = std::time::Instant::now();
            match decode_archive(EMBEDDED_RUNTIME_VBC) {
                Ok(archive) => {
                    let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
                    if std::env::var("VERUM_TRACE_CODEGEN_PATH").is_ok() {
                        eprintln!(
                            "[embedded_stdlib_vbc] decode_archive: {:.2}ms ({} modules, {} KB compressed)",
                            elapsed,
                            archive.module_count(),
                            EMBEDDED_RUNTIME_VBC.len() / 1024,
                        );
                    }
                    tracing::info!(
                        target: "embedded_stdlib_vbc",
                        "loaded precompiled stdlib archive ({} modules, {:.1} KB compressed) in {:.2}ms",
                        archive.module_count(),
                        EMBEDDED_RUNTIME_VBC.len() as f64 / 1024.0,
                        elapsed,
                    );
                    Some(archive)
                }
                Err(e) => {
                    tracing::warn!(
                        target: "embedded_stdlib_vbc",
                        "failed to decode embedded stdlib archive: {} — falling back to source compile",
                        e
                    );
                    None
                }
            }
        })
        .as_ref()
}

/// True when this compiler binary ships a precompiled stdlib
/// archive. Used by callers that need to choose between hot-path
/// (load embedded VBC) and slow-path (re-compile sources) at
/// process startup.
pub fn has_runtime_archive() -> bool {
    !EMBEDDED_RUNTIME_VBC.is_empty()
}

/// Returns the size in bytes of the embedded archive (compressed,
/// pre-decode). Useful for telemetry / `verum --version`-style
/// output without paying the deserialise cost.
pub fn embedded_size_bytes() -> usize {
    EMBEDDED_RUNTIME_VBC.len()
}

fn decode_archive(bytes: &[u8]) -> VbcResult<VbcArchive> {
    // Reuse `read_archive_from_file` by writing to a tempfile? No —
    // there's an in-memory variant. The existing `read_archive` takes
    // a `Read`; wrap the byte slice in a `Cursor`.
    let cursor = std::io::Cursor::new(bytes);
    verum_vbc::archive::read_archive(cursor)
        .map_err(|e| verum_vbc::error::VbcError::ArchiveError(e.to_string()))
}

// Suppress dead-code warnings on a helper that is only kept for
// callers that prefer the file-based loader (e.g. tests that want
// to swap in a fixture archive).
#[doc(hidden)]
pub fn read_archive_from_path(path: &std::path::Path) -> VbcResult<VbcArchive> {
    read_archive_from_file(path).map_err(|e| {
        verum_vbc::error::VbcError::ArchiveError(format!(
            "read archive {}: {}",
            path.display(),
            e
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_or_empty() {
        // Either the build script embedded a real archive (and
        // get_runtime_archive returns Some), or it embedded an empty
        // placeholder (returns None). Both are valid outcomes.
        let archive = get_runtime_archive();
        if archive.is_some() {
            assert!(has_runtime_archive());
            assert!(embedded_size_bytes() > 0);
        } else {
            assert!(!has_runtime_archive() || embedded_size_bytes() == 0);
        }
    }

    /// End-to-end Phase 6b smoke test: deserialise the embedded
    /// stdlib archive, hand it to a fresh `VbcLinker`, and verify
    /// the merge succeeds without panicking. Skipped when the
    /// compiler binary was built without a precompiled archive
    /// (early bootstrap, minimal-features build).
    #[test]
    fn linker_round_trip_through_embedded_archive() {
        use verum_vbc::linker::VbcLinker;

        let archive = match get_runtime_archive() {
            Some(a) => a,
            None => return, // archive missing — minimal-features build
        };

        let mut linker = VbcLinker::new("aarch64-apple-darwin");
        let added = linker.add_archive(archive).expect("add_archive");
        assert!(added > 0, "expected at least one module merged from archive");

        let merged = linker.finalize();
        // Merged module should carry the union of every contained
        // module's strings/types/functions. Lower bound: at least
        // the count from archive.
        assert!(
            merged.strings.len() > 0,
            "merged module has no strings — likely an integration regression"
        );
        assert!(
            merged.functions.len() > 0,
            "merged module has no functions — archive contained only empty modules?"
        );
    }
}
