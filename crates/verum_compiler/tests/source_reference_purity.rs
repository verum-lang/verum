//! Ordinary source loading must retain the selected view operation's footprint.
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

#[test]
fn complete_source_input_preserves_reference_operation_purity() {
    // Source selection belongs to the child process environment; other tests
    // never observe a temporary global change to the stdlib root.
    if std::env::var_os("VERUM_STDLIB_PATH").is_none() {
        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../core").canonicalize().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "complete_source_input_preserves_reference_operation_purity", "--nocapture"])
            .env("VERUM_STDLIB_PATH", core).status().unwrap();
        assert!(result.success(), "ordinary source checker failed: {result}");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = std::env::var_os("VERUM_PURITY_SOURCE_INPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let input = directory.path().join("consumer.vr");
            std::fs::write(&input, r#"
                pure fn consistent(text: &Text) -> Bool {
                    let bytes = text.as_bytes();
                    text.len() == bytes.len()
                }
            "#).unwrap();
            input
        });
    let input = input.canonicalize().unwrap();
    let mut session = Session::new(CompilerOptions { input: input.clone(), ..Default::default() });
    let result = CompilationPipeline::new(&mut session).run_check_only();
    assert!(result.is_ok() && !session.has_errors(),
        "{}: {result:?}\n{}", input.display(), session.format_diagnostics());
}
