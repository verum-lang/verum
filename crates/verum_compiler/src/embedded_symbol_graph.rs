//! The baked call-graph index, embedded next to the archive it
//! describes (T0753).
//!
//! `verum stdlib precompile` writes
//! `target/precompiled-stdlib/runtime.symbol_graph` beside
//! `runtime.vbca`; `build.rs` embeds those bytes through
//! `STDLIB_SYMBOL_GRAPH_PATH`.  The reader here hands them out as a
//! `&'static [u8]` — no decode, no allocation, and the pages are
//! faulted in by the OS only as the walk touches them.
//!
//! Empty when the sidecar was missing at build time.  The caller then
//! scans the archive instead, which is what every build did before
//! this sidecar existed: slower start-up, identical answers.

/// Bytes of the baked symbol graph, or empty when this compiler was
/// built without one.
static EMBEDDED_SYMBOL_GRAPH: &[u8] = include_bytes!(env!("STDLIB_SYMBOL_GRAPH_PATH"));

/// The embedded graph bytes, or `None` when this build has no sidecar.
pub fn embedded_bytes() -> Option<&'static [u8]> {
    if EMBEDDED_SYMBOL_GRAPH.is_empty() {
        None
    } else {
        Some(EMBEDDED_SYMBOL_GRAPH)
    }
}

/// Size of the embedded sidecar in bytes — telemetry without touching
/// the contents.
pub fn embedded_size_bytes() -> usize {
    EMBEDDED_SYMBOL_GRAPH.len()
}
