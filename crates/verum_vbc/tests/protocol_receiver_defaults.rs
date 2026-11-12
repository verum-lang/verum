#![cfg(feature = "codegen")]
//! A protocol default belongs to its subject; explicit arguments stay lexical.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, VbcCodegen},
};

fn compile(source: &str) -> CodegenResult<VbcModule> {
    let ast = Parser::new(source)
        .parse_module()
        .expect("protocol default grammar");
    VbcCodegen::with_config(CodegenConfig::new("receiver_defaults")).compile_module(&ast)
}

fn run(module: VbcModule, expected: i64) {
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let entry = module
            .find_function_by_name("receiver_defaults.main")
            .expect("exact entry");
        let result = verum_vbc::interpreter::Interpreter::new(module.into())
            .execute_function(entry)
            .expect("selected implementation");
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

#[test]
fn selected_implementation_defaults_receive_their_own_subject() {
    for application in ["Combines", "Combines<Self>"] {
        let source = format!(
            r#"
            module receiver_defaults;
            type Combines<Rhs = Self> is protocol {{}};
            type Base is protocol {{}};
            type Granted is protocol {{ fn flag(&self) -> Int {{ 73 }} }};
            implement<T: Base + Combines> Granted for T {{}}
            type Cell is {{ value: Int }};
            implement {application} for Cell {{}}
            implement Base for Cell {{}}
            fn main() -> Int {{ Cell {{ value: 7 }}.flag() }}
        "#
        );
        run(compile(&source).unwrap(), 73);
    }
}

#[test]
fn a_bound_default_and_an_explicit_self_argument_have_distinct_owners() {
    for shape in [
        "Self",
        "(Self, Self)",
        "&Self",
        "Envelope<Self>",
        "fn<R>(Self, R) -> Self",
        "fn<T>(Self, T) -> Self",
    ] {
        let chained = shape.replace("Self", "Carrier");
        for explicit_self in [false, true] {
            for evidence_is_receiver in [false, true] {
                let bound = if explicit_self {
                    format!("Check<Self, {shape}>")
                } else {
                    "Check".into()
                };
                let carrier = if evidence_is_receiver {
                    "Wrapper<Argument>"
                } else {
                    "Argument"
                };
                let evidence = shape.replace("Self", carrier);
                let granted = explicit_self == evidence_is_receiver;
                let action = if granted {
                    "wrapper.flag()"
                } else {
                    "wrapper.value"
                };
                let source = format!(
                    r#"
                module receiver_defaults;
                type Envelope<T> is {{ value: () }};
                type Check<Carrier = Self, Rhs = {chained}> is protocol {{}};
                type Start is protocol {{}};
                type Base is protocol {{}};
                type Granted is protocol {{ fn flag(&self) -> Int {{ 73 }} }};
                implement<X: Start + Base> Granted for X {{}}
                type Argument is ();
                type Wrapper<T> is {{ value: Int }};
                implement<T: {bound}> Base for Wrapper<T> {{}}
                implement Check<{carrier}, {evidence}> for Argument {{}}
                implement Start for Wrapper<Argument> {{}}
                fn main() -> Int {{
                    let wrapper: Wrapper<Argument> = Wrapper {{ value: 7 }};
                    {action}
                }}
            "#
                );
                let module = compile(&source).unwrap_or_else(|error| panic!("shape={shape}, explicit={explicit_self}, receiver_evidence={evidence_is_receiver}: {error:?}"));
                assert_eq!(
                    module.functions.iter().any(|function| module
                        .get_string(function.name)
                        .is_some_and(|name| name.ends_with("Wrapper.flag"))),
                    granted
                );
                run(module, if granted { 73 } else { 7 });
            }
        }
    }
}

#[test]
fn actual_add_default_is_used_when_proving_a_blanket_requirement() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/base/protocols.vr"
    ))
    .unwrap();
    let parsed = Parser::new(&source).parse_module().unwrap();
    let add = parsed
        .items
        .iter()
        .find(|item| {
            matches!(&item.kind,
        verum_ast::ItemKind::Type(declaration) if declaration.name.name == "Add")
        })
        .unwrap();
    // Copy the actual declaration verbatim into a minimal dependency tree;
    // the ordinary source compiler owns all registration/default-body phases.
    // This is separate from the unchanged full-stdlib source qualification.
    // Item spans begin at `type`; retain the original visibility prefix too.
    let start = source[..add.span.start as usize]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let provider_source = format!(
        "module core.base.protocols;\n{}",
        &source[start..add.span.end as usize]
    );
    let provider = Parser::new(&provider_source).parse_module().unwrap();
    let verum_ast::ItemKind::Type(original) = &add.kind else {
        unreachable!()
    };
    let reproduced = provider
        .items
        .iter()
        .find_map(|item| match &item.kind {
            verum_ast::ItemKind::Type(declaration)
                if declaration.name.name == original.name.name =>
            {
                Some(declaration)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(reproduced.visibility, original.visibility);
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir_all(core.join("base")).unwrap();
    std::fs::write(core.join("mod.vr"), "module core;").unwrap();
    std::fs::write(core.join("base/mod.vr"), "module core.base;").unwrap();
    std::fs::write(core.join("base/protocols.vr"), provider_source).unwrap();
    for exact in [false, true] {
        let protocol = if exact { "Add" } else { "Add<Int>" };
        let rhs = if exact { "Cell" } else { "Int" };
        let action = if exact { "cell.flag()" } else { "cell.value" };
        let client = format!(
            r#"
            module receiver_defaults;
            mount core.base.protocols.Add;
            type Base is protocol {{}};
            type Granted is protocol {{ fn flag(&self) -> Int {{ 73 }} }};
            implement<T: Base + Add> Granted for T {{}}
            type Cell is {{ value: Int }};
            implement {protocol} for Cell {{
                type Output = Cell;
                fn add(self, rhs: {rhs}) -> Cell {{ self }}
            }}
            implement Base for Cell {{}}
            fn main() -> Int {{ let cell = Cell {{ value: 7 }}; {action} }}
        "#
        );
        let path = directory.path().join("probe.vr");
        std::fs::write(&path, &client).unwrap();
        let client = Parser::new(&client).parse_module().unwrap();
        let module = VbcCodegen::with_config(CodegenConfig::new("receiver_defaults"))
            .compile_module_with_mounts(&client, path.to_str().unwrap(), core.to_str().unwrap())
            .unwrap_or_else(|error| panic!("exact={exact}: {error:?}"));
        run(module, if exact { 73 } else { 7 });
    }
}
