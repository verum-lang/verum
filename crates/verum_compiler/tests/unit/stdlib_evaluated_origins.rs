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
        "fn leak() -> Result<&Int, Int> { let local = 7;
            let r: Result<&Int, Int> = Result.Ok(&local); Result.Ok(((r?))) }",
        "fn leak() -> Maybe<Maybe<&Int>> { let local = 7;
            let r: Maybe<Maybe<&Int>> = Maybe.Some(Maybe.Some(&local)); Maybe.Some(r?) }",
    ] {
        let errors = check(escaping).expect_err(escaping);
        assert!(errors.iter().any(|error| error.code() == Some("E312")
            && error.message().contains("local")), "{escaping}: {errors:#?}");
    }
}

#[test]
fn propagated_residual_keeps_loans_until_its_actual_destination() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for (escapes, body) in [
        (false, "fn keep(source: &Int) -> Result<Int, &Int> { let r: Result<Int, &Int> = Result.Err(source); Result.Ok(r?) }"),
        (false, "fn consume() -> Int { let local = 7; let r: Result<Int, &Int> = Result.Err(&local);
            let held: Result<Int, &Int> = try { r? }; match held { Result.Ok(v) => v, Result.Err(e) => *e } }"),
        (false, "fn consume() -> Int { let local = 7; let r: Result<Int, &Int> = Result.Err(&local);
            try { r? } recover |e| { *e } }"),
        (false, "fn keep(source: &Int) -> &Int { let local = 7; let r: Result<Int, &Int> = Result.Err(&local);
            try { r?; source } recover |e| { let value = *e; source } }"),
        (false, "fn keep(source: &Int, r: Result<Int, Int>) -> &Int {
            try { r?; source } recover { ref e => { let value = *e; source } } }"),
        (true, "fn leak(source: &Int, r: Result<Int, Int>) -> &Int {
            try { r?; source } recover { ref e => e } }"),
        (true, "fn leak() -> Result<Int, &Int> { let local = 7; let r: Result<Int, &Int> = Result.Err(&local); Result.Ok(r?) }"),
        (true, "fn leak() -> Result<Int, &Int> { let local = 7; let r: Result<Int, &Int> = Result.Err(&local);
            let held: Result<Int, &Int> = try { r? }; held }"),
        (true, "fn leak() -> Result<Int, &Int> { let caught: Result<Int, &Int> = try {
            let local = 7; let r: Result<Int, &Int> = Result.Err(&local); r? }; caught }"),
        (true, "fn leak(source: &Int) -> &Int { let local = 7; let r: Result<Int, &Int> = Result.Err(&local);
            try { r?; source } recover |e| { e } }"),
    ] {
        let source = format!("module loan_probe; mount core.base.result.Result; {body}");
        let module = Parser::new(&source).parse_module().unwrap();
        let result = check_source_module(&mut phase.configured_checker(), "loan_probe", &module, &registry);
        if escapes {
            let errors = result.expect_err("propagated reference outlived its storage");
            assert!(errors.iter().any(|error| error.code() == Some("E312")
                && (error.message().contains("local") || error.message().contains("temporary match subject"))), "{body}: {errors:#?}");
        } else {
            result.unwrap_or_else(|errors| panic!("{body}: {errors:#?}"));
        }
    }
}

#[test]
fn checked_static_protocol_calls_preserve_owned_inputs_and_refuse_borrowed_results() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let header = "module collector; mount core.{Result, List, Text}; mount core.encoding.json.JsonValue;";
    for (accepted, source) in [
        (true, "type Factory<T> is protocol { fn read(value: JsonValue, path: Text) -> Result<T, Int>; };
            fn collect<T: Factory<T>>(v: JsonValue) -> Result<List<T>, Int> {
                let mut out: List<T> = List.new(); out.push(T.read(v, \"\")?); Result.Ok(out) }"),
        (true, "type Factory<T> is protocol { fn make(value: Int) -> T; };
            fn collect<T: Factory<T>>() -> List<T> { let mut out: List<T> = List.new(); out.push(T.make(7)); out }"),
        (false, "type Factory<T> is protocol { fn make(value: &Int) -> T; };
            fn leak<T: Factory<T>>() -> List<T> { let local=7; let mut out: List<T> =List.new(); out.push(T.make(&local)); out }"),
        (false, "type Factory<T> is protocol { fn make(value: List<&Int>) -> T; };
            fn leak<T: Factory<T>>() -> List<T> { let local=7; let mut refs: List<&Int> =List.new(); refs.push(&local);
                let mut out: List<T> =List.new(); out.push(T.make(refs)); out }"),
        (false, "context Hidden { fn get() -> Int; }
            type Factory<T> is protocol { fn make() -> T using [Hidden]; };
            fn try_collect<T: Factory<T>>() -> List<T> using [Hidden] {
                let mut out: List<T> = List.new(); out.push(T.make()); out }"),
    ] {
        let module = Parser::new(&format!("{header} {source}")).parse_module().unwrap();
        let result = check_source_module(&mut phase.configured_checker(), "collector", &module, &registry);
        if accepted {
            result.unwrap_or_else(|errors| panic!("{source}: {errors:#?}"));
        } else {
            let errors = result.expect_err(source);
            assert!(errors.iter().any(|error| error.code() == Some("E312")), "{source}: {errors:#?}");
        }
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

#[test]
fn formatter_writes_preserve_the_output_buffer_loan() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let fixture = include_str!("../../../../core-tests/text/format/unit_test.vr");
    let mut module = Parser::new(fixture).parse_module().unwrap();
    let mut selected = 0;
    module.items.retain(|item| match &item.kind {
        verum_ast::ItemKind::Function(f) => {
            let keep = f.name.as_str() == "test_radix_prefix_survives_zero_and_signed_minimum";
            if keep { selected += 1; }
            keep
        }
        _ => true,
    });
    assert_eq!(selected, 1);
    check_source_module(&mut phase.configured_checker(), "format_unit_tests", &module, &registry)
        .unwrap_or_else(|errors| panic!("format fixture: {errors:#?}"));
    let source = r#"module formatter_loan; mount core.base.protocols.{Formatter,FormatSpec};
        fn render()->Text {
            let mut text=Text.from("λ:");
            { let mut f=Formatter.with_spec(&mut text,FormatSpec.new());
              let _=f.write_int(0); let _=f.width(); }
            text
        }"#;
    let module = Parser::new(source).parse_module().unwrap();
    check_source_module(&mut phase.configured_checker(), "formatter_loan", &module, &registry)
        .unwrap_or_else(|errors| panic!("formatter read after write: {errors:#?}"));
}
