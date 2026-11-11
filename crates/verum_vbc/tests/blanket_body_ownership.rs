#![cfg(feature = "codegen")]
//! A blanket body belongs to its declaration until a receiver is selected.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    VbcCodegen::with_config(CodegenConfig::new("template"))
        .compile_module(&ast)
        .expect("source compilation")
}

fn replay(module: VbcModule, expected: i64) {
    let encoded = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&encoded).unwrap();
    for module in [module, decoded] {
        let entry = module.find_function_by_name("template.main").unwrap();
        let result = Interpreter::new(module.into())
            .execute_function(entry)
            .expect("actual VM execution");
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

#[test]
fn an_unselected_blanket_has_no_executable_free_receiver_body() {
    let module = compile(
        "module template; \
         type Base is protocol {}; \
         type Derived is protocol { fn score(&self) -> Int; }; \
         implement<R: Base> Derived for R { fn score(&self) -> Int { 41 } } \
         fn main() -> Int { 7 }",
    );
    assert!(
        module.functions.iter().all(|function| {
            let name = module.get_string(function.name).unwrap();
            name != "R.score" && !name.ends_with(".R.score")
        }),
        "a generic receiver name must not publish an executable method"
    );
    replay(module, 7);
}

#[test]
fn a_selected_blanket_executes_only_through_its_actual_receiver() {
    let module = compile(
        "module template; \
         type Base is protocol { fn value(&self) -> Int; }; \
         type Derived is protocol { fn score(&self) -> Int; }; \
         implement<Subject: Base> Derived for Subject { \
             fn score(&self) -> Int { self.value() + 1 } \
         } \
         type Cell is { value: Int }; \
         implement Base for Cell { fn value(&self) -> Int { self.value } } \
         fn main() -> Int { let cell = Cell { value: 41 }; cell.score() }",
    );
    assert!(module.functions.iter().all(|function| {
        let name = module.get_string(function.name).unwrap();
        name != "Subject.score" && !name.ends_with(".Subject.score")
    }));
    replay(module, 42);
}

#[test]
fn a_concrete_receiver_is_not_classified_by_its_spelling() {
    let module = compile(
        "module template; \
         type Derived is protocol { fn score(&self) -> Int; }; \
         type R is { value: Int }; \
         implement Derived for R { fn score(&self) -> Int { self.value } } \
         fn main() -> Int { let receiver = R { value: 43 }; receiver.score() }",
    );
    replay(module, 43);
}

#[test]
fn selected_bodies_cannot_disappear_when_lowering_fails() {
    for implementation in [
        "type Base is protocol {}; \
         type Derived is protocol { fn score(&self) -> Int; }; \
         implement<R: Base> Derived for R { fn score(&self) -> Int { missing_body() } } \
         implement Base for Cell {}",
        "type Derived is protocol { fn score(&self) -> Int { missing_body() } }; \
         implement Derived for Cell {}",
    ] {
        let source = format!(
            "module template; type Cell is {{ value: Int }}; {implementation} \
             fn main() -> Int {{ let cell = Cell {{ value: 1 }}; cell.score() }}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        for entry in ["single", "additional", "mounted", "phases"] {
            let mut codegen = VbcCodegen::with_config(CodegenConfig::new("template"));
            let result = match entry {
                "single" => codegen.compile_module(&ast),
                "additional" => codegen.compile_additional_module(&ast),
                "phases" => codegen
                    .collect_all_declarations(&ast)
                    .and_then(|()| codegen.compile_pending_default_methods())
                    .and_then(|()| codegen.finalize_module()),
                "mounted" => {
                    let directory = tempfile::tempdir().unwrap();
                    let source_path = directory.path().join("main.vr");
                    std::fs::write(&source_path, &source).unwrap();
                    codegen.compile_module_with_mounts(
                        &ast,
                        source_path.to_str().unwrap(),
                        directory.path().to_str().unwrap(),
                    )
                }
                _ => unreachable!(),
            };
            let error = result.expect_err("a selected body must either lower or report its error");
            assert!(
                error.to_string().contains("missing_body"),
                "{entry}: {error}"
            );
        }
    }
}
