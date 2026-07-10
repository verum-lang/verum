//! Embedded precompiled stdlib `CoreMetadata`.
//!
//! T2-extended of the single-path archive-driven epic.  At build
//! time, `verum stdlib precompile` writes
//! `target/precompiled-stdlib/runtime.core_metadata` (bincode-
//! serialised [`CoreMetadata`]); `build.rs` embeds those bytes via
//! `STDLIB_RUNTIME_CORE_METADATA_PATH` env-var.  At runtime this
//! module decodes the bytes once into a process-wide
//! `Arc<CoreMetadata>` and feeds the typechecker via
//! `pipeline.set_stdlib_metadata`.
//!
//! Replaces the slow `load_stdlib_modules` path (parsed 2444 stdlib
//! ASTs from disk cache, ~9.6s) with a single bincode decode
//! (typically <50 ms for a few-MB metadata blob).

use std::sync::{Arc, OnceLock};

use verum_types::core_metadata::CoreMetadata;

/// Bincode-serialised `CoreMetadata` bytes embedded at build time.
/// Empty when the precompile sidecar wasn't available — the runtime
/// detects this and falls through to the legacy source-driven
/// typecheck stdlib registration.
static EMBEDDED_RUNTIME_METADATA: &[u8] =
    include_bytes!(env!("STDLIB_RUNTIME_CORE_METADATA_PATH"));

/// Lazily decoded metadata.  First reader pays the bincode decode
/// cost; every subsequent reader gets `Arc::clone`.
static RUNTIME_METADATA: OnceLock<Option<Arc<CoreMetadata>>> = OnceLock::new();

/// Return the embedded stdlib `CoreMetadata`, or `None` when the
/// compiler binary was built without one.  Decoded once per
/// process via `OnceLock`; subsequent calls are O(1) `Arc::clone`.
pub fn get_runtime_metadata() -> Option<Arc<CoreMetadata>> {
    RUNTIME_METADATA
        .get_or_init(|| {
            if EMBEDDED_RUNTIME_METADATA.is_empty() {
                tracing::debug!(
                    target: "embedded_stdlib_metadata",
                    "no precompiled stdlib metadata embedded — typecheck falls back to source"
                );
                return None;
            }
            match bincode::deserialize::<CoreMetadata>(EMBEDDED_RUNTIME_METADATA) {
                Ok(meta) => {
                    tracing::info!(
                        target: "embedded_stdlib_metadata",
                        "loaded precompiled stdlib metadata ({} types, {} functions, {} protocols, {:.1} KB)",
                        meta.types.len(),
                        meta.functions.len(),
                        meta.protocols.len(),
                        EMBEDDED_RUNTIME_METADATA.len() as f64 / 1024.0
                    );
                    Some(Arc::new(meta))
                }
                Err(e) => {
                    // #45: same loud-failure contract as the VBC archive
                    // (embedded_stdlib_vbc.rs). An EMBEDDED metadata blob
                    // that fails to decode is a build-system inconsistency
                    // (stale blob vs current CoreMetadata layout), and the
                    // silent source fallback runs a semantically DIFFERENT
                    // typecheck path — measured flipping suites from 7 to 14
                    // failures with zero visible signal.
                    if std::env::var("VERUM_ALLOW_STDLIB_SOURCE_FALLBACK").is_ok() {
                        tracing::warn!(
                            target: "embedded_stdlib_metadata",
                            "failed to decode embedded stdlib metadata: {} — \
                             VERUM_ALLOW_STDLIB_SOURCE_FALLBACK set, typecheck falls back to source",
                            e
                        );
                        None
                    } else {
                        panic!(
                            "FATAL: this compiler binary embeds stdlib typecheck metadata \
                             ({} KB) that failed to decode: {}\n\
                             The embedded blob was generated with an older CoreMetadata \
                             layout than this reader.\n\
                             Fix: regenerate and re-embed:\n\
                             \x20   verum stdlib precompile --out <CARGO_TARGET_DIR>/precompiled-stdlib/runtime.vbca\n\
                             \x20   touch crates/verum_compiler/build.rs && cargo build -p verum_cli\n\
                             Escape hatch (slow, typechecks stdlib from source): \
                             VERUM_ALLOW_STDLIB_SOURCE_FALLBACK=1",
                            EMBEDDED_RUNTIME_METADATA.len() / 1024,
                            e,
                        );
                    }
                }
            }
        })
        .clone()
}

/// True when this compiler binary ships a precompiled stdlib
/// `CoreMetadata` blob.  Used by callers that need to gate the
/// archive-driven typecheck on its availability before paying the
/// decode cost.
pub fn has_runtime_metadata() -> bool {
    !EMBEDDED_RUNTIME_METADATA.is_empty()
}

/// Size in bytes of the embedded metadata sidecar (bincode bytes,
/// pre-decode).  Useful for telemetry / `verum --version`-style
/// output without paying the deserialise cost.
pub fn embedded_metadata_size_bytes() -> usize {
    EMBEDDED_RUNTIME_METADATA.len()
}
