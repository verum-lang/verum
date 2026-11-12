//! Ordinary frontend and archive production must enforce declaration-owned fields.
use std::{fs, sync::Arc};
use verum_compiler::api::{CommonPipelineConfig, SourceFile, run_common_pipeline};
use verum_compiler::precompile::{PrecompileConfig, precompile_stdlib};
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter};

fn sources(visibility: &str, owner: &str, operation: usize) -> Vec<(&'static str, String)> {
    let body = [
        "let state = make(42); state.value",
        "let mut state = make(1); state.value = 42; read(&state)",
        "let state = State { value: 42 }; read(&state)",
    ][operation];
    vec![
        (
            "mod.vr",
            "module core; public module provider; public module consumer;".into(),
        ),
        (
            "provider.vr",
            format!(
                "module core.chamber; public type State is {{ {visibility} value: Int }}; public fn make(value: Int) -> State {{ State {{ value }} }} public fn read(state: &State) -> Int {{ state.value }}"
            ),
        ),
        (
            "consumer.vr",
            format!(
                "module {owner}; mount core.chamber.{{State, make, read}}; public fn probe() -> Int {{ {body} }}"
            ),
        ),
    ]
}
fn frontend(files: &[(&str, String)]) -> Result<(), String> {
    let files: Vec<_> = files
        .iter()
        .map(|(path, source)| SourceFile::new(*path, source.as_str()))
        .collect();
    run_common_pipeline(&files, &CommonPipelineConfig::minimal())
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}
fn archive(files: &[(&str, String)], owner: &str) -> Result<(), String> {
    archive_with_linkage(files, owner, None)
}
fn archive_with_linkage(files: &[(&str, String)], owner: &str, expected_external: Option<&str>) -> Result<(), String> {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    fs::create_dir_all(&core).unwrap();
    for (path, source) in files {
        fs::write(core.join(path), source).unwrap();
    }
    let config = PrecompileConfig {
        stdlib_path: core,
        output_path: directory.path().join("core.vbca"),
        target_triple: None,
        threads: None,
        verbose: false,
    };
    let result = match precompile_stdlib(&config) {
        Ok(result) => result,
        Err(error) => {
            assert!(!config.output_path.exists());
            return Err(format!("{error:#}"));
        }
    };
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&config.output_path).unwrap();
    let name = format!("{owner}.probe");
    let module = archive
        .module_names_ordered()
        .iter()
        .find_map(|entry| {
            let module = archive.load_module(entry).unwrap();
            module.find_function_by_name(&name).map(|_| module)
        })
        .expect("archive must contain the actual owned probe");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        if let Some(expected) = expected_external {
            let targets: Vec<_> = module.external_function_names.iter()
                .filter_map(|(_, name)| module.strings.get(*name))
                .filter(|name| name.ends_with(".make"))
                .collect();
            assert!(!targets.is_empty(), "fixture must retain the forward reference");
            assert!(targets.iter().all(|name| *name == expected), "wrong declaration identity: {targets:?}");
        }
        let id = module.find_function_by_name(&name).unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(id).unwrap().try_as_i64(), Some(42));
    }
    Ok(())
}
fn matrix(visibility: &str, owner: &str, allowed: bool, bake: bool) {
    for operation in 0..3 {
        for reverse in [false, true] {
            let mut files = sources(visibility, owner, operation);
            if reverse {
                files.reverse();
            }
            let result = if bake {
                archive(&files, owner)
            } else {
                frontend(&files)
            };
            if allowed {
                result.unwrap_or_else(|error| {
                    panic!("operation={operation}, reverse={reverse}: {error}")
                });
            } else {
                let error = result.expect_err("forbidden field access was admitted");
                assert!(
                    error.contains("value")
                        && (error.contains("private")
                            || error.contains("visibility")
                            || error.contains("accessible")),
                    "wrong refusal operation={operation}: {error}"
                );
            }
        }
    }
}
#[test]
fn ordinary_private_child_refused() {
    matrix("private", "core.chamber.child", false, false);
}
#[test]
fn ordinary_restricted_outside_refused() {
    matrix("public(in core.chamber)", "core.outside", false, false);
}
#[test]
fn ordinary_scoped_child_allowed() {
    matrix("public(in core.chamber)", "core.chamber.child", true, false);
}
#[test]
fn ordinary_public_outside_allowed() {
    matrix("public", "core.outside", true, false);
}
#[test]
fn producer_private_child_refused() {
    matrix("private", "core.chamber.child", false, true);
}
#[test]
fn producer_restricted_outside_refused() {
    matrix("public(in core.chamber)", "core.outside", false, true);
}
#[test]
fn producer_scoped_child_allowed() {
    matrix("public(in core.chamber)", "core.chamber.child", true, true);
}
#[test]
fn producer_public_outside_allowed() {
    matrix("public", "core.outside", true, true);
}

#[test]
fn unused_missing_imports_fail_in_frontend_and_producer() {
    for mount in [
        "mount absent.Token;",
        "mount core.chamber.Missing;",
        "mount absent.*;",
    ] {
        let mut files = sources("public", "core.chamber.child", 0);
        files[2].1 =
            format!("module core.chamber.child; {mount} public fn probe() -> Int {{ 42 }}");
        for result in [frontend(&files), archive(&files, "core.chamber.child")] {
            let error = result.expect_err("unused invalid mount accepted");
            assert!(
                error.contains("absent") || error.contains("Missing"),
                "wrong refusal: {error}"
            );
        }
    }
}

#[test]
fn namespace_group_alias_glob_and_reexport_imports_work() {
    for (mount, body) in [
        (
            "mount core.chamber;",
            "let state = chamber.make(42); state.value",
        ),
        (
            "mount core.chamber as cabinet;",
            "let state = cabinet.make(42); state.value",
        ),
        (
            "mount core.chamber.{make as fabricate};",
            "let state = fabricate(42); state.value",
        ),
        ("mount core.chamber.*;", "let state = make(42); state.value"),
        (
            "mount core.facade.{make};",
            "let state = make(42); state.value",
        ),
    ] {
        let mut files = sources("public", "core.chamber.child", 0);
        files.push((
            "facade.vr",
            "module core.facade; public mount core.chamber.{State, make, read};".into(),
        ));
        files[2].1 =
            format!("module core.chamber.child; {mount} public fn probe() -> Int {{ {body} }}");
        frontend(&files).unwrap_or_else(|error| panic!("{mount}: {error}"));
        archive(&files, "core.chamber.child").unwrap_or_else(|error| panic!("{mount}: {error}"));
    }
}

#[test]
fn facade_reexport_keeps_selected_same_leaf_owner_in_both_orders() {
    for reverse in [false, true] {
        let mut files = vec![
            ("mod.vr", "module core; public module first; public module second; public module facade; public module consumer;".into()),
            ("first.vr", "module core.first; public fn make() -> Int { 7 }".into()),
            ("second.vr", "module core.second; public fn make() -> Int { 42 }".into()),
            ("facade.vr", "module core.facade; public mount core.second.{make};".into()),
            ("consumer.vr", "module core.first.child; mount core.facade.{make}; public fn probe() -> Int { make() }".into()),
        ];
        if reverse {
            // Reverse the producer's file traversal too, not only frontend input.
            files[1].0 = "second.vr";
            files[2].0 = "first.vr";
            files.reverse();
        }
        frontend(&files).unwrap_or_else(|error| panic!("reverse={reverse}: {error}"));
        archive_with_linkage(&files, "core.first.child", Some("core.second.make"))
            .unwrap_or_else(|error| panic!("reverse={reverse}: {error}"));
    }
}
