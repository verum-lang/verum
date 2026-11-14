//! Inferred source results survive the ordinary compiler and archive pipeline.
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter};

fn execute(provider: &str, consumer: &str) -> Result<i64, Text> {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    for (name, source) in [
        (
            "mod.vr",
            "module core; public module provider; public module consumer;".into(),
        ),
        ("provider.vr", format!("module core.provider; {provider}")),
        ("consumer.vr", format!("module core.consumer; {consumer}")),
    ] {
        std::fs::write(core.join(name), source).unwrap();
    }
    let output = directory.path().join("results.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core()
        .map_err(|error| Text::from(format!("{error}\n{}", session.format_diagnostics())))?;
    let archive = read_archive_from_file(&output).expect("archive reload");
    let module = archive.load_module("core").unwrap();
    let entry = module.find_function_by_name("core.consumer.probe").unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    vm.run_global_ctors().unwrap();
    Ok(vm
        .execute_function(entry)
        .unwrap()
        .try_as_i64()
        .expect("Int result"))
}

#[test]
fn inferred_scalar_update_preserves_payloads_through_the_archive() {
    for result in ["", "->Unit"] {
        let provider = format!(
            "public type Cell is {{public pointer:&Int,public value:Int}}; public fn update(cell:&mut Cell, unrelated:&Int){result} {{cell.value=73;}}"
        );
        let value = execute(&provider, "mount core.provider.{Cell,update}; fn preserve(mut cell:Cell)->Cell {let unrelated=4;update(&mut cell,&unrelated);cell} public fn probe()->Int {let input=9;let result=preserve(Cell{pointer:&input,value:0});result.value}")
            .unwrap_or_else(|error| panic!("{result}: {error}"));
        assert_eq!(value, 73);
    }
}

#[test]
fn inferred_generic_results_execute_after_archive_reload() {
    let value = execute(
        "public fn identity<T>(value:T) {value}",
        "mount core.provider.identity as selected; public fn probe()->Int {if selected(true) {selected(73)} else {0}}",
    ).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(value, 73);
}

#[test]
fn a_wrong_consumer_result_is_rejected_by_source_admission() {
    let error = execute(
        "public fn identity(value:Int) {value}",
        "mount core.provider.identity; fn invalid()->Bool {identity(73)} public fn probe()->Int {73}",
    ).expect_err("the caller cannot select a different result type");
    assert!(
        (error.contains("expected Bool") || error.contains("expected 'Bool'")) && error.contains("Int"),
        "{error}"
    );
}
