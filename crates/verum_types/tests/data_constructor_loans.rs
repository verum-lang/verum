//! Data constructors carry payloads; only evaluated calls can mutate captures.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn errors(source: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (owner, body) in providers.iter().chain(std::iter::once(&("probe", source))) {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let ast = VerumParser::new().parse_module_str(body, file).expect("source grammar");
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        let mut info = ModuleInfo::new(id, path, ast, file, (*body).into());
        info.exports = exports;
        registry.register(info);
    }
    let module = registry.get_by_path("probe").unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_module_registry_direct(registry.clone());
    checker.set_current_module_path("probe");
    let mut errors = checker.prepare_module_declarations(&module.ast);
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    errors.iter().chain(checker.diagnostic_sources()).map(|e| e.to_string().into()).collect()
}

const CHOICE: &str = "type Choice<T> is Empty | Hold(T);";
const CELL: &str = "type Cell is { public pointer:&Int, public value:Int };";

#[test]
fn diagnostic_environment_cannot_authorize_missing_record_fields() {
    const CHILD: &str = "VERUM_RECORD_ADMISSION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Separate processes avoid mutating the environment of parallel tests.
        for trace in [false, true] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "diagnostic_environment_cannot_authorize_missing_record_fields", "--nocapture"])
                .env(CHILD, "1").env_remove("VERUM_TRACE_MISSING_FIELD");
            if trace { command.env("VERUM_TRACE_MISSING_FIELD", "1"); }
            let output = command.output().expect("record admission child");
            assert!(output.status.success(), "trace={trace}: stdout={:?}; stderr={:?}",
                std::str::from_utf8(&output.stdout), std::str::from_utf8(&output.stderr));
        }
        return;
    }

    let record = "type Pair is { public first:Int, public second:Int };";
    for body in [
        "fn probe() { let value:Pair = Pair { first:1 }; }",
        "fn probe()->Pair { Pair { first:1 } }",
        "implement Pair { fn probe()->Self { Self { first:1 } } }",
    ] {
        let source = format!("{record} {body}");
        let found = errors(&source, &[]);
        assert!(found.iter().any(|error| error.contains("Missing required field 'second'")),
            "incomplete record admitted or misdiagnosed: {source}: {found:?}");
    }
    for body in [
        "fn probe()->Pair { Pair { first:1, second:2 } }",
        "implement Pair { fn probe()->Self { Self { first:1, second:2 } } }",
        "fn probe(original:Pair)->Pair { Pair { first:1, ..original } }",
    ] {
        let source = format!("{record} {body}");
        let found = errors(&source, &[]);
        assert!(found.is_empty(), "valid record rejected: {source}: {found:?}");
    }
}

#[test]
fn qualified_and_bare_constructors_do_not_invoke_callable_payloads() {
    for constructor in ["Choice.Hold", "Hold"] {
        let source = format!("{CHOICE} fn wrap(callback:fn()->Int)->Choice<fn()->Int> {{let value={constructor}(callback);value}}");
        let found = errors(&source, &[]);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
}

#[test]
fn generic_constructor_payloads_do_not_become_unknown_calls() {
    let source = format!("{CHOICE} fn wrap<T>(payload:T)->Choice<T> {{let value=Choice.Hold(payload);value}}");
    let found = errors(&source, &[]);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn constructor_assignment_preserves_owned_payload_storage() {
    let source = format!("{CHOICE} type Slot is {{value:Choice<fn()->Int>}}; implement Slot {{fn replace(&mut self, callback:fn()->Int) {{self.value=Choice.Hold(callback);}}}}");
    let found = errors(&source, &[]);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn imported_and_renamed_constructors_keep_the_selected_owner() {
    for mount in ["mount package.Choice;", "mount package.Choice as Renamed;"] {
        let constructor = if mount.contains("Renamed") { "Renamed" } else { "Choice" };
        let source = format!("{mount} fn wrap(callback:fn()->Int)->{constructor}<fn()->Int> {{let value={constructor}.Hold(callback);value}}");
        let found = errors(&source, &[("package", "public type Choice<T> is Empty | Hold(T);")]);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
}

#[test]
fn same_shaped_function_calls_still_invalidate_unknown_captures() {
    for declaration in [
        "fn invoke(callback:fn()->Int)->Choice<fn()->Int>;",
        "fn invoke(callback:fn()->Int)->Choice<fn()->Int> {callback();Choice.Hold(callback)}",
    ] {
        let source = format!("{CHOICE} {declaration} fn wrap(callback:fn()->Int)->Choice<fn()->Int> {{let value=invoke(callback);value}}");
        let found = errors(&source, &[]);
        assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{source}: {found:?}");
    }
}

#[test]
fn a_shadowed_constructor_name_does_not_grant_data_construction_authority() {
    let source = format!("{CHOICE} fn invoke(callback:fn()->Int)->Choice<fn()->Int>; fn wrap(callback:fn()->Int)->Choice<fn()->Int> {{let Hold=invoke;let value=Hold(callback);value}}");
    let found = errors(&source, &[]);
    assert!(found.iter().any(|e| e.contains("cannot establish the lifetime")), "{found:?}");
}

#[test]
fn data_construction_does_not_erase_argument_call_writes() {
    let source = format!("{CHOICE} {CELL} fn replace(target:&mut Cell, source:&Int)->Int {{target.pointer=source;73}} fn invalid(input:&Int)->Cell {{let mut cell=Cell{{pointer:input,value:0}};{{let local=4;let ignored=Choice.Hold(replace(&mut cell,&local));}}cell}}");
    let found = errors(&source, &[]);
    assert!(found.iter().any(|e| e.contains("local") && e.contains("does not live long enough")), "{found:?}");
}

#[test]
fn borrowed_constructor_payloads_keep_their_real_lifetimes() {
    let source = format!("{CHOICE} fn invalid()->Choice<&Int> {{let local=4;let value=Choice.Hold(&local);value}}");
    let found = errors(&source, &[]);
    assert!(found.iter().any(|e| e.contains("local") && e.contains("does not live long enough")), "{found:?}");
}

#[test]
fn constructor_payloads_reject_overlapping_mutable_loans() {
    for body in ["let value=Pair.Join(&mut local,&mut local);", "let value:Pair=Pair.Join(&mut local,&mut local);"] {
        let source = format!("type Pair is Join(&mut Int,&mut Int); fn invalid() {{let mut local=4;{body}}}");
        let found = errors(&source, &[]);
        assert!(found.iter().any(|e| e.contains("borrow")), "{source}: {found:?}");
    }
}

#[test]
fn constructor_payloads_still_consume_affine_values() {
    let source = format!("{CHOICE} type affine Token is {{value:Int}}; fn invalid(token:Token) {{let first=Choice.Hold(token);let second=Choice.Hold(token);}}");
    let found = errors(&source, &[]);
    assert!(found.iter().any(|e| e.contains("moved") || e.contains("consumed")), "{found:?}");
}

#[test]
fn one_affine_payload_can_be_constructed() {
    for constructor in ["Choice.Hold", "Hold"] {
        let source = format!("{CHOICE} type affine Token is {{value:Int}}; fn wrap(token:Token)->Choice<Token> {{let value={constructor}(token);value}}");
        let found = errors(&source, &[]);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
}
