//! VTYP emission through the compiler's bootstrap checking pipeline.
//!
//! These self-contained modules use primitive types and no mounts. This tests
//! the supported exporter surface, not embedded stdlib or imported identities.
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};

fn bootstrap_pipeline(session: &mut Session) -> CompilationPipeline<'_> {
    let root = session.options().input.parent().unwrap().to_path_buf();
    CompilationPipeline::new_core(session, CoreConfig::new(root))
}

#[test]
fn unsupported_export_is_a_compilation_error_without_a_snapshot() {
    let directory = tempfile::TempDir::new().unwrap();
    let source = directory.path().join("sample.vr");
    std::fs::write(
        &source,
        "module sample; public fn retain<T>(value: T) -> T { value }",
    )
    .unwrap();
    let options = CompilerOptions {
        input: source.clone(),
        emit_types: true,
        ..Default::default()
    };
    let mut session = Session::new(options);
    let error = bootstrap_pipeline(&mut session)
        .run_check_only()
        .expect_err("unsupported contract must reach the caller");
    assert!(
        error.to_string().contains("VTYP export: function contracts"),
        "{error:#}"
    );
    assert!(!source.with_extension("vtyp").exists());
}

#[test]
fn an_inferred_result_cannot_be_published_as_unit() {
    let directory = tempfile::TempDir::new().unwrap();
    let source = directory.path().join("inferred.vr");
    std::fs::write(
        &source,
        "module inferred; public fn value() { 41 } public fn probe() -> Int { value() }",
    )
    .unwrap();
    let mut session = Session::new(CompilerOptions {
        input: source.clone(),
        emit_types: true,
        ..Default::default()
    });
    let error = bootstrap_pipeline(&mut session)
        .run_check_only()
        .expect_err("source-only export must refuse an inferred result");
    assert!(
        error.to_string().contains("VTYP export: inferred function result"),
        "{error:#}"
    );
    assert!(!source.with_extension("vtyp").exists());
}

#[test]
fn supported_export_and_write_failure_use_the_same_compiler_path() {
    let directory = tempfile::TempDir::new().unwrap();
    let source = directory.path().join("sample.vr");
    std::fs::write(
        &source,
        "module sample; public fn retain(value: Int) -> Int { value } public fn finish() -> () {}",
    )
    .unwrap();
    let options = CompilerOptions {
        input: source.clone(),
        emit_types: true,
        ..Default::default()
    };
    let mut session = Session::new(options.clone());
    bootstrap_pipeline(&mut session).run_check_only().unwrap();
    assert!(!session.has_errors());
    let path = source.with_extension("vtyp");
    let bytes = std::fs::read(&path).unwrap();
    let restored = verum_types::type_exporter::deserialize_module_exports(&bytes).unwrap();
    assert_eq!(restored.path.as_str(), "sample");
    assert_eq!(restored.functions.len(), 2);
    assert_eq!(restored.functions[0].name.as_str(), "retain");
    assert_eq!(restored.functions[1].name.as_str(), "finish");
    assert_eq!(restored.functions[1].return_type, verum_types::Type::Unit);

    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let mut session = Session::new(options);
    let error = bootstrap_pipeline(&mut session)
        .run_check_only()
        .unwrap_err();
    assert!(
        error.to_string().contains("cannot write type metadata"),
        "{error:#}"
    );
}
