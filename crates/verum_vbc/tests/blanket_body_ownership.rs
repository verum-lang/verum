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

#[test]
fn qualified_calls_use_the_proven_blanket_for_each_receiver() {
    let source = r#"
        module template;
        type Parent is protocol { fn score(&self) -> Int; };
        type Ready is protocol {};
        type Other is protocol { fn value(&self) -> Int; };
        type Derived is protocol { fn value(&self) -> Int; };
        implement<T: Parent + Ready> Derived for T {
            fn value(&self) -> Int { self.score() }
        }
        type Left is { points: Int };
        type Right is { points: Int };
        type Direct is { points: Int };
        type Missing is { points: Int };
        implement Parent for Left { fn score(&self) -> Int { self.points } }
        implement Ready for Left {}
        implement Other for Left { fn value(&self) -> Int { 17 } }
        implement Ready for Right {}
        implement Parent for Right { fn score(&self) -> Int { self.points + 1 } }
        implement Parent for Direct { fn score(&self) -> Int { 0 } }
        implement Ready for Direct {}
        implement Derived for Direct { fn value(&self) -> Int { self.points + 2 } }
        implement Parent for Missing { fn score(&self) -> Int { self.points } }
        implement Missing { fn value(&self) -> Int { 91 } }
        fn bounded<T: Derived>(item: T) -> Int { item.value() }
        fn other<T: Other>(item: T) -> Int { item.value() }
        fn main() -> Int {
            bounded(Left { points: 1 }) + bounded(Right { points: 10 }) + bounded(Direct { points: 100 }) + other(Left { points: 0 })
        }
    "#;
    let module = compile(source);
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    for image in [&module, &decoded] {
        let descriptor = |name: &str| {
            image
                .types
                .iter()
                .find(|ty| image.get_string(ty.name) == Some(name))
                .unwrap()
        };
        let protocol = descriptor("Derived").id;
        let rows = |name| {
            descriptor(name)
                .protocols
                .iter()
                .filter(|row| row.protocol.0 == protocol.0)
                .collect::<Vec<_>>()
        };
        let left = rows("Left");
        let right = rows("Right");
        let direct = rows("Direct");
        assert_eq!((left.len(), right.len(), direct.len()), (1, 1, 1));
        assert_eq!(
            left[0].source_implementation,
            right[0].source_implementation
        );
        assert_ne!(left[0].methods, right[0].methods);
        let blanket = left[0].source_implementation.as_ref().unwrap();
        assert_eq!(blanket.origin_module(), "template");
        assert_eq!(blanket.declaration().unwrap().generics.len(), 1);
        assert!(
            direct[0]
                .source_implementation
                .as_ref()
                .unwrap()
                .declaration()
                .unwrap()
                .generics
                .is_empty()
        );
        assert!(rows("Missing").is_empty());
        let selector = image.strings.lookup("dyn:template.Derived.value").unwrap();
        let receiver = |name| verum_vbc::types::TypeRef::Concrete(descriptor(name).id);
        let exact = image
            .try_bound_method(&receiver("Left"), selector, 0)
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(exact.0, left[0].methods[0]);
        let missing = image
            .try_bound_method(&receiver("Missing"), selector, 0)
            .unwrap_err();
        assert!(missing.contains("template.Derived.value"), "{missing}");
    }
    replay(module, 131);
}

#[test]
fn a_blanket_carries_its_arguments_associated_type_and_declared_default() {
    let module = compile(
        r#"
        module template;
        type Parent is protocol { fn detail(&self) -> Int { 17 } };
        type Derived<Arg> is protocol { type Payload; fn detail(&self) -> Int { 23 } };
        implement<T: Parent> Derived<Int> for T { type Payload = T; }
        type Cell is { points: Int };
        implement Parent for Cell {}
        fn parent<T: Parent>(item: T) -> Int { item.detail() }
        fn derived<T: Derived<Int>>(item: T) -> Int { item.detail() }
        fn main() -> Int {
            let cell = Cell { points: 0 };
            parent(cell) * 100 + derived(cell)
        }
    "#,
    );
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    for image in [&module, &decoded] {
        let descriptor = |name: &str| {
            image
                .types
                .iter()
                .find(|ty| image.get_string(ty.name) == Some(name))
                .unwrap()
        };
        let receiver = descriptor("Cell");
        let row = receiver
            .protocols
            .iter()
            .find(|row| row.protocol.0 == descriptor("Derived").id.0)
            .unwrap();
        assert_eq!(
            row.protocol_args_text
                .iter()
                .map(|&id| image.get_string(id))
                .collect::<Vec<_>>(),
            [Some("Int")]
        );
        let payload = image.strings.lookup("Payload").unwrap();
        assert_eq!(
            row.associated_types,
            [(payload, verum_vbc::types::TypeRef::Concrete(receiver.id))]
        );
    }
    replay(module, 1723);
}
