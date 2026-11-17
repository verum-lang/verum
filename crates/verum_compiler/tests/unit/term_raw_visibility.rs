//! T2301: saved terminal state belongs to the raw component; dimensions are public data.
use super::*;

const RAW_OWNER: &str = "core.term.raw";
const RAW_SOURCE: &str = include_str!("../../../../core/term/raw/mod.vr");
const OUTSIDE: [&str; 4] = [
    "core.term",
    "core.term.render.probe",
    "core.term.raw_extra",
    "application.probe",
];

fn probe(owner: &str, imports: &str, body: &str) -> Module {
    Parser::new(&format!("module {owner}; {imports} {body}"))
        .parse_module().expect("visibility probe grammar")
}

fn denied(result: Result<(), List<Diagnostic>>, name: &str, context: &str) {
    let errors = result.expect_err("caller outside the component gained saved-state access");
    assert!(errors.iter().any(|error|
        error.code() == Some("E202") && error.message().contains(name)),
        "{context}: expected visibility refusal for {name}: {errors:#?}");
}

#[test]
fn saved_state_scope_survives_target_selection() {
    // Retain the actual source declaration and its field policies. Isolating
    // this primitive-only carrier avoids executing either OS backend; this
    // checks visibility, not the correctness of the platform ABI layout.
    let mut declaration = Parser::new(RAW_SOURCE).parse_module().unwrap();
    let header = declaration.items[declaration.file_header_index().expect("raw module header")].clone();
    declaration.items.retain(|item| matches!(&item.kind,
        ItemKind::Type(ty) if ty.name.name == "TermiosState"));
    assert_eq!(declaration.items.len(), 1);
    declaration.items.insert(0, header);

    for (target, bodies) in [
        ("aarch64-apple-darwin", [
            "fn create()->TermiosState { TermiosState { data:[0;60] } }",
            "fn read(state:&TermiosState)->Byte { state.data[0] }",
            "fn write(state:&mut TermiosState) { state.data[0] = 7 as Byte; }",
        ]),
        ("x86_64-unknown-linux-gnu", [
            "fn create()->TermiosState { TermiosState { data:[0;60] } }",
            "fn read(state:&TermiosState)->Byte { state.data[0] }",
            "fn write(state:&mut TermiosState) { state.data[0] = 7 as Byte; }",
        ]),
        ("x86_64-pc-windows-msvc", [
            "fn create()->TermiosState { TermiosState { input_mode:0, output_mode:0 } }",
            "fn read(state:&TermiosState)->UInt32 { state.input_mode }",
            "fn write(state:&mut TermiosState) { state.output_mode = 7; }",
        ]),
    ] {
        let target = TargetConfig::for_target(target);
        let declaration = select_source_module(RAW_OWNER, &declaration, &target)
            .unwrap().expect("raw state is available for every supported target");
        let registry = source_registry([(RAW_OWNER, "raw/mod.vr", &declaration)]).unwrap();
        let phase = crate::phases::semantic_analysis::SemanticAnalysisPhase::new()
            .with_target(target);
        let imports = "mount core.term.raw.TermiosState;";
        for owner in ["core.term.raw.termios", "core.term.raw.wincon", "core.term.raw.child.probe"] {
            for body in bodies {
                let module = probe(owner, imports, body);
                check_source_module(&mut phase.configured_checker(), owner, &module, &registry)
                    .unwrap_or_else(|errors| panic!("{owner}: {body}: {errors:#?}"));
            }
        }
        for owner in OUTSIDE {
            // An unused import must fail too; absence of a field access must
            // not turn the restricted declaration into a public capability.
            let module = probe(owner, imports, "fn probe() {}");
            denied(check_source_module(&mut phase.configured_checker(), owner, &module, &registry),
                "TermiosState", owner);
        }
    }
}

#[test]
fn real_raw_api_separates_saved_state_from_public_dimensions() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for owner in [RAW_OWNER, "core.term"] {
        let module = registry.read().get_by_path(owner).unwrap().ast.clone();
        check_source_types(&mut phase.configured_checker(), owner, &module, &registry,
            |error| error.to_diagnostic_with_span(Some(|span| session.convert_span(span))))
            .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));
    }

    for (owner, allowed) in [
        ("core.term.raw.scope_probe", true),
        ("core.term.raw.child.scope_probe", true),
        ("core.term.render.probe", false),
        ("core.term.raw_extra", false),
        ("application.probe", false),
    ] {
        for field in ["original", "fd", "is_raw", "is_alternate"] {
            for body in [
                format!("fn read(mode:&TerminalMode) {{ let value = mode.{field}; }}"),
                format!("fn write(mode:&mut TerminalMode, other:&TerminalMode) {{ mode.{field} = other.{field}; }}"),
            ] {
                let module = probe(owner, "mount core.term.raw.TerminalMode;", &body);
                let result = check_source_module(&mut phase.configured_checker(), owner, &module, &registry);
                if allowed {
                    result.unwrap_or_else(|errors| panic!("{owner}: {body}: {errors:#?}"));
                } else {
                    denied(result, field, owner);
                }
            }
        }
    }

    let owner = "core.term.raw.scope_probe";
    let module = probe(owner,
        "mount core.term.raw.{TerminalMode,TermiosState}; mount core.sys.FileDesc;",
        "fn create(original:TermiosState, fd:FileDesc)->TerminalMode { TerminalMode { original:original, fd:fd, is_raw:false, is_alternate:false } }");
    check_source_module(&mut phase.configured_checker(), owner, &module, &registry)
        .unwrap_or_else(|errors| panic!("raw mode construction: {errors:#?}"));

    let owner = "application.probe";
    for body in [
        "fn create(columns:Int, rows:Int, pixel_width:Maybe<Int>, pixel_height:Maybe<Int>)->TerminalSize { TerminalSize { columns:columns, rows:rows, pixel_width:pixel_width, pixel_height:pixel_height } }",
        "fn read(size:&TerminalSize) { let columns = size.columns; let rows = size.rows; let width = size.pixel_width; let height = size.pixel_height; }",
    ] {
        let module = probe(owner, "mount core.term.TerminalSize;", body);
        check_source_module(&mut phase.configured_checker(), owner, &module, &registry)
            .unwrap_or_else(|errors| panic!("public dimensions: {body}: {errors:#?}"));
    }
    let module = probe(owner, "mount core.term.TermiosState;", "fn probe() {}");
    let errors = check_source_module(&mut phase.configured_checker(), owner, &module, &registry)
        .expect_err("the public facade still exports saved platform state");
    assert!(errors.iter().any(|error|
        error.code() == Some("E401") && error.message().contains("TermiosState")),
        "{errors:#?}");
}
