//! Scanner state is shared within its component, never with its callers.
use super::*;

#[test]
fn sqlite_scanner_source_component_keeps_its_state_encapsulated() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let mut failures = List::new();
    for owner in [
        "core.database.sqlite.native.complete.state",
        "core.database.sqlite.native.complete.scanner",
        "core.database.sqlite.native.complete.engine",
    ] {
        let module = registry.read().get_by_path(owner).unwrap().ast.clone();
        if let Err(errors) = check_source_types(
            &mut phase.configured_checker(), owner, &module, &registry,
            |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))),
        ) {
            failures.push((owner, errors));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");

    for (owner, allowed) in [
        ("core.database.sqlite.native.complete.probe", true),
        ("core.database.sqlite.native.complete_extra", false),
        ("core.database.sqlite.native.client", false),
    ] {
        for body in [
            "fn read(state:&ScanState)->Bool { state.has_content }",
            "fn write(state:&mut ScanState) { state.has_content = true; }",
            "fn construct()->ScanState { ScanState { mode: SmNormal, trigger_depth:0, saw_create:false, saw_trigger:false, saw_begin:false, saw_end:false, has_content:false, last_was_semicolon:false } }",
        ] {
            let source = format!("module {owner}; mount core.database.sqlite.native.complete.state.{{ScanState,SmNormal}}; {body}");
            let module = Parser::new(&source).parse_module().unwrap();
            let result = check_source_module(&mut phase.configured_checker(), owner, &module, &registry);
            if allowed {
                result.unwrap_or_else(|errors| panic!("{owner}: {body}: {errors:#?}"));
            } else {
                let errors = result.expect_err("outside caller gained access to scanner state");
                assert!(errors.iter().any(|error| error.code() == Some("E202")), "{errors:#?}");
            }
        }
    }
}
