//! Keep evaluated-origin safety compatible with the actual source library.
//! Reuse the release roster and source checker; no declarations or bodies are
//! replaced with fixture versions and every named module is checked in full.
use super::*;

#[test]
fn propagated_success_preserves_the_ownership_of_its_payload() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let check = |body: &str| {
        let source = format!("module loan_probe; mount core.{{Maybe, Result, List}}; {body}");
        let module = Parser::new(&source).parse_module().unwrap();
        check_source_module(&mut phase.configured_checker(), "loan_probe", &module, &registry)
    };
    for legal in [
        "fn keep(r: Result<&Int, Int>) -> Result<&Int, Int> { Result.Ok(r?) }",
        "fn consume() -> Result<Int, Int> { let local = 7;
            let r: Result<&Int, Int> = Result.Ok(&local); Result.Ok(*(r?)) }",
        "fn collect<T>(r: Result<T, Int>) -> Result<List<T>, Int> {
            let mut out: List<T> = List.new(); out.push(r?); Result.Ok(out) }",
        "fn collect<T>(r: Maybe<T>) -> Maybe<List<T>> {
            let mut out: List<T> = List.new(); out.push(r?); Maybe.Some(out) }",
    ] {
        check(legal).unwrap_or_else(|errors| panic!("{legal}: {errors:#?}"));
    }
    for escaping in [
        "fn leak() -> Result<&Int, Int> { let local = 7;
            let r: Result<&Int, Int> = Result.Ok(&local); Result.Ok(r?) }",
        "fn leak() -> Maybe<Maybe<&Int>> { let local = 7;
            let r: Maybe<Maybe<&Int>> = Maybe.Some(Maybe.Some(&local)); Maybe.Some(r?) }",
    ] {
        let errors = check(escaping).expect_err("a reference to expired storage escaped through ?");
        assert!(errors.iter().any(|error| error.code() == Some("E312")
            && error.message().contains("local")), "{escaping}: {errors:#?}");
    }
}

#[test]
fn actual_stdlib_value_and_callable_origins_preserve_source_admission() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let mut failures = List::new();
    for owner in [
        "core.action.monads.exception",
        "core.async.diagnostics",
        "core.async.waker",
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
        "core.metrics.registry",
        "core.net.h3.qpack.huffman",
        "core.net.h3.qpack.instructions",
        "core.net.h3.qpack.integer",
        "core.net.quic.stream_sm.send",
        "core.net.tls13.cert_compress",
        "core.net.tls13.record.plaintext",
        "core.security.aead.aes_gcm",
        "core.security.ecc.x25519",
        "core.shell.signal",
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
