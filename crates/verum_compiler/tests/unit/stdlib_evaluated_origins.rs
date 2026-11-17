//! Keep evaluated-origin safety compatible with the actual source library.
//! Reuse the release roster and source checker; no declarations or bodies are
//! replaced with fixture versions and every named module is checked in full.
use super::*;

#[test]
fn actual_stdlib_value_and_callable_origins_preserve_source_admission() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let mut failures = List::new();
    for owner in [
        "core.async.diagnostics",
        "core.base.glob",
        "core.base.ordering",
        "core.database.sqlite.native.concurrency.pool",
        "core.database.sqlite.native.integrity.error",
        "core.database.sqlite.native.iter.cursor",
        "core.database.sqlite.native.opcode_catalog.error_codes",
        "core.encoding.base64",
        "core.encoding.der",
        "core.encoding.hex",
        "core.encoding.varint",
        "core.math.bits",
        "core.math.checked",
        "core.mem.cap_audit",
        "core.net.h3.qpack.huffman",
        "core.net.h3.qpack.instructions",
        "core.net.h3.qpack.integer",
        "core.net.quic.stream_sm.send",
        "core.net.tls13.cert_compress",
        "core.net.tls13.record.plaintext",
        "core.security.aead.aes_gcm",
        "core.security.ecc.x25519",
        "core.sys.init",
        // Preserve the source-admission improvements from the same campaign.
        "core.action.monads.pure",
        "core.action.monads.quantum",
        "core.collections.consistent_hash",
        "core.intrinsics.bitwise",
        "core.runtime.config",
        "core.security.labels",
    ] {
        let module = registry.read().get_by_path(owner)
            .unwrap_or_else(|| panic!("missing actual source module {owner}"))
            .ast.clone();
        if let Err(errors) = check_source_types(
            &mut phase.configured_checker(), owner, &module, &registry,
            |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))),
        ) {
            failures.push((Text::from(owner), errors));
        }
    }
    assert!(failures.is_empty(), "actual source admission: {failures:#?}");
}
