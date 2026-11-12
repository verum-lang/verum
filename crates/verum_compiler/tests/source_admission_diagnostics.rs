use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};

#[test]
fn independent_source_failures_are_reported_together_without_an_archive() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module first; public module second; public module valid;").unwrap();
    std::fs::write(core.join("first.vr"), "module core.first; public fn first_failure() -> Int { true }").unwrap();
    std::fs::write(core.join("second.vr"), "module core.second; public fn second_failure() -> Bool { 7 }").unwrap();
    std::fs::write(core.join("valid.vr"), "module core.valid; public fn valid() -> Int { 73 }").unwrap();
    let output = root.path().join("runtime.vbca");
    let mut serial = None;
    for num_threads in [1, 2, 4] {
        let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), num_threads, ..Default::default() });
        let error = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core().expect_err("invalid source was admitted").to_string();
        assert!(!output.exists(), "failed admission published bytecode");
        assert!(error.contains("core.first") && error.contains("core.second"), "{error}");
        let rendered = session.format_diagnostics();
        assert!(rendered.contains("first.vr") && rendered.contains("second.vr"), "{rendered}");
        assert_eq!(session.error_count(), 2, "{rendered}");
        let result = (error, rendered);
        if let Some(serial) = &serial {
            assert_eq!(&result, serial, "worker count changed diagnostics");
        } else {
            serial = Some(result);
        }
    }
}

#[test]
fn parallel_admission_preserves_lexical_owners_and_cross_module_calls() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module first; public module second; public module consumer;").unwrap();
    std::fs::write(core.join("first.vr"), "module core.first; public type Cell is { public value: Int }; public fn make() -> Cell { Cell { value: 73 } }").unwrap();
    std::fs::write(core.join("second.vr"), "module core.second; public type Cell is { public value: Bool }; public fn make() -> Cell { Cell { value: true } }").unwrap();
    std::fs::write(core.join("consumer.vr"), "module core.consumer; mount core.first.make; public fn answer() -> Int { make().value }").unwrap();
    for num_threads in [1, 2, 4] {
        let output = root.path().join(format!("runtime-{num_threads}.vbca"));
        let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), num_threads, ..Default::default() });
        let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core().unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(session.error_count(), 0);
        assert!(output.exists());
    }
}
