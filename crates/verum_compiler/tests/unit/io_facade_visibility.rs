//! T2834: lexical imports cannot hide the actual public buffered I/O routes.
use super::*;

#[test]
fn real_io_facade_retains_buffer_declarations_and_private_fields() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let facade = registry.read().get_by_path("core.io").unwrap().ast.clone();
    check_source_types(&mut phase.configured_checker(), "core.io", &facade, &registry,
        |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))))
        .unwrap_or_else(|errors| panic!("complete I/O facade: {errors:#?}"));

    let imports = r#"
        mount core.io.{Read, Write, BufReader, BufWriter};
        mount core.io.file.{BufReader as FileReader, BufWriter as FileWriter};
        mount core.io.buffer.{BufReader as DirectReader, BufWriter as DirectWriter};
    "#;
    let client = Parser::new(&format!(r#"
        module application.io_client;
        {imports}
        fn reader<R:Read>(value:BufReader<R>)->DirectReader<R> {{ value }}
        fn file_reader<R:Read>(value:FileReader<R>)->DirectReader<R> {{ value }}
        fn writer<W:Write>(value:BufWriter<W>)->DirectWriter<W> {{ value }}
        fn file_writer<W:Write>(value:FileWriter<W>)->DirectWriter<W> {{ value }}
    "#)).parse_module().unwrap();
    check_source_module(&mut phase.configured_checker(), "application.io_client", &client, &registry)
        .unwrap_or_else(|errors| panic!("public buffered I/O declaration identity: {errors:#?}"));

    for (owner, name) in [
        ("core.io", "BufReader"), ("core.io.file", "BufReader"),
        ("core.io", "BufWriter"), ("core.io.file", "BufWriter"),
    ] {
        let (_, source) = phase.configured_checker().resolve_public_source_type(
            owner, name, &registry.read()).expect("public archive source declaration");
        assert_eq!(source.as_str(), "core.io.buffer", "{owner}.{name}");
    }

    for body in [
        "fn reader<R:Read>(value:&BufReader<R>) { let hidden = value.inner; }",
        "fn writer<W:Write>(value:&FileWriter<W>) { let hidden = value.inner; }",
    ] {
        let client = Parser::new(&format!("module application.io_client; {imports} {body}"))
            .parse_module().unwrap();
        let errors = check_source_module(&mut phase.configured_checker(), "application.io_client", &client, &registry)
            .expect_err("a public route exposed private buffer storage");
        assert!(errors.iter().any(|error|
            error.code() == Some("E202") && error.message().contains("inner")),
            "{body}: {errors:#?}");
    }
}
