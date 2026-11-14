#![cfg(feature = "codegen")]
//! Conditions are checked from source declarations, before bytecode publication.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, VbcCodegen},
};

fn compile(declarations: &str, call: bool) -> CodegenResult<VbcModule> {
    let action = if call { "cell.flag()" } else { "cell.value" };
    let source = format!(
        r#"
        module obligations;
        type A is protocol {{}};
        type Granted is protocol {{ fn flag(&self) -> Int; }};
        type Cell is {{ value: Int }};
        implement A for Cell {{}}
        {declarations}
        fn main() -> Int {{ let cell = Cell {{ value: 7 }}; {action} }}
    "#
    );
    let ast = Parser::new(&source)
        .parse_module()
        .expect("implementation constraint grammar");
    VbcCodegen::with_config(CodegenConfig::new("obligations")).compile_module(&ast)
}

fn run(module: &VbcModule, expected: i64) {
    let wire = verum_vbc::serialize::serialize_module(module).unwrap();
    for module in [
        module.clone(),
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let main = module.find_function_by_name("obligations.main").unwrap();
        let result = verum_vbc::interpreter::Interpreter::new(module.into())
            .execute_function(main)
            .expect("execute selected source body");
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

fn absent(declarations: &str) {
    let module = compile(declarations, false).expect("unrelated code remains valid");
    assert!(
        !module.functions.iter().any(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name.ends_with("Cell.flag"))
        }),
        "unsatisfied body must not be published"
    );
    run(&module, 7);
    let error = compile(declarations, true).expect_err("absent method must refuse dispatch");
    assert!(error.to_string().contains("flag"), "{error}");
}

#[test]
fn where_constraints_are_conjunctive() {
    let declarations = r#"
        type B is protocol {};
        implement<X: A> Granted for X where X: B { fn flag(&self) -> Int { 59 } }
    "#;
    absent(declarations);
    run(
        &compile(&format!("{declarations}\nimplement B for Cell {{}}"), true).unwrap(),
        59,
    );
}

#[test]
fn protocol_arguments_are_part_of_the_obligation() {
    let declarations = r#"
        type B<T> is protocol {};
        implement<X: A + B<Int>> Granted for X { fn flag(&self) -> Int { 59 } }
    "#;
    absent(&format!("{declarations}\nimplement B<Bool> for Cell {{}}"));
    run(
        &compile(
            &format!("{declarations}\nimplement B<Int> for Cell {{}}"),
            true,
        )
        .unwrap(),
        59,
    );
}

#[test]
fn conditional_implementation_heads_keep_their_own_premises() {
    let declarations = r#"
        type B<T> is protocol {};
        type Mark is protocol {};
        type Argument is ();
        implement<T: Mark> B<T> for Cell {}
        implement<X: A + B<Argument>> Granted for X { fn flag(&self) -> Int { 59 } }
    "#;
    absent(declarations);
    run(
        &compile(
            &format!("{declarations}\nimplement Mark for Argument {{}}"),
            true,
        )
        .unwrap(),
        59,
    );
}

#[test]
fn unfounded_cycles_do_not_discharge_obligations() {
    absent(
        r#"
        type B is protocol {};
        type C is protocol {};
        implement<X: B> C for X {}
        implement<X: C> B for X {}
        implement<X: A + B> Granted for X { fn flag(&self) -> Int { 59 } }
    "#,
    );
}

#[test]
fn finite_derived_evidence_satisfies_the_whole_conjunction() {
    let module = compile(
        r#"
        type B is protocol {};
        type C is protocol {};
        implement<X: A> B for X {}
        implement<X: B> C for X {}
        implement<X: A + C> Granted for X { fn flag(&self) -> Int { 59 } }
    "#,
        true,
    )
    .unwrap();
    run(&module, 59);
}

#[test]
fn membership_is_not_an_associated_equality() {
    let declarations = r#"
        type B is protocol { type Item; };
        implement<X: A + B<Item = Int>> Granted for X { fn flag(&self) -> Int { 59 } }
    "#;
    absent(&format!(
        "{declarations}\nimplement B for Cell {{ type Item = Bool; }}"
    ));
    run(
        &compile(
            &format!("{declarations}\nimplement B for Cell {{ type Item = Int; }}"),
            true,
        )
        .unwrap(),
        59,
    );
}

#[test]
fn inherited_bounds_keep_their_substituted_arguments() {
    let declarations = r#"
        type Parent<T> is protocol {};
        type Child<T> is protocol extends Parent<T> {};
        implement<X: A + Parent<Int>> Granted for X { fn flag(&self) -> Int { 59 } }
    "#;
    absent(&format!(
        "{declarations}\nimplement Child<Bool> for Cell {{}}"
    ));
    run(
        &compile(
            &format!("{declarations}\nimplement Child<Int> for Cell {{}}"),
            true,
        )
        .unwrap(),
        59,
    );
}

#[test]
fn overlapping_blanket_bodies_cannot_be_selected_by_source_order() {
    for reverse in [false, true] {
        let first = "implement<X: A> Granted for X { fn flag(&self) -> Int { 59 } }";
        let second = "implement<X: A> Granted for X { fn flag(&self) -> Int { 73 } }";
        let declarations = if reverse {
            format!("{second}\n{first}")
        } else {
            format!("{first}\n{second}")
        };
        let error =
            compile(&declarations, true).expect_err("two applicable source bodies are ambiguous");
        assert!(error.to_string().contains("ambiguous"), "{error}");
    }
}

#[test]
fn growing_obligations_are_rejected_on_an_ordinary_thread_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let error = compile(
                r#"
            type Wrap<T> is { value: T };
            type B<T> is protocol {};
            implement<T> B<T> for Cell where Cell: B<Wrap<T>> {}
            implement<X: A + B<Int>> Granted for X { fn flag(&self) -> Int { 59 } }
        "#,
                true,
            )
            .expect_err("expanding obligations require a bounded diagnostic");
            assert!(
                error.to_string().contains("obligation type budget"),
                "{error}"
            );
            run(
                &compile(
                    "implement<X: A> Granted for X { fn flag(&self) -> Int { 59 } }",
                    true,
                )
                .unwrap(),
                59,
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn a_repeated_query_observes_a_later_source_implementation() {
    let declarations = Parser::new(
        r#"
        module obligations;
        type A is protocol {};
        type B<T> is protocol {};
        type Granted is protocol { fn flag(&self)->Int; };
        type Cell is {value:Int};
        implement<X: A + B<Int>> Granted for X {fn flag(&self)->Int {59}}
        implement A for Cell {}
    "#,
    )
    .parse_module()
    .unwrap();
    let additional = Parser::new(
        r#"
        module obligations;
        implement B<Int> for Cell {}
        implement A for Cell {}
        fn main()->Int {let cell=Cell {value:7}; cell.flag()}
    "#,
    )
    .parse_module()
    .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("obligations"));
    codegen.initialize();
    let first = codegen.compile_additional_module(&declarations).unwrap();
    assert!(!first.functions.iter().any(|function| {
        first
            .get_string(function.name)
            .is_some_and(|name| name.ends_with("Cell.flag"))
    }));
    let completed = codegen.compile_additional_module(&additional).unwrap();
    run(&completed, 59);
}

#[test]
fn repeated_proofs_keep_different_protocol_arguments_independent() {
    let declarations = r#"
        type B<T> is protocol {};
        type Denied is protocol {fn forbidden(&self)->Int;};
        implement<X: A + B<Int>> Granted for X {fn flag(&self)->Int {59}}
        implement<X: A + B<Bool>> Denied for X {fn forbidden(&self)->Int {91}}
        implement B<Int> for Cell {}
    "#;
    let module = compile(declarations, true).unwrap();
    assert!(!module.functions.iter().any(|function| {
        module
            .get_string(function.name)
            .is_some_and(|name| name.ends_with("Cell.forbidden"))
    }));
    run(&module, 59);
    let error = compile(
        &format!("{declarations}\nfn rejected(cell:&Cell)->Int {{cell.forbidden()}}"),
        true,
    )
    .expect_err("another protocol argument cannot inherit a completed proof");
    assert!(error.to_string().contains("forbidden"), "{error}");
}

#[test]
fn a_new_alias_scope_cannot_reuse_an_earlier_prepared_header() {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("obligations"));
    for (argument, action, expected) in [
        ("First", "cell.flag()", 59),
        ("Second", "cell.value", 7),
        ("First", "cell.flag()", 59),
    ] {
        let source = format!(
            r#"
            module obligations;
            type A is protocol {{}};
            type B<T> is protocol {{}};
            type Granted is protocol {{fn flag(&self)->Int;}};
            type Cell is {{value:Int}};
            public module choices {{public type First is (); public type Second is ();}}
            mount choices.{argument} as Argument;
            implement<X:A+B<Argument>> Granted for X {{fn flag(&self)->Int {{59}}}}
            implement A for Cell {{}}
            implement B<choices.First> for Cell {{}}
            fn main()->Int {{let cell=Cell {{value:7}}; {action}}}
        "#
        );
        let module = codegen
            .compile_module(&Parser::new(&source).parse_module().unwrap())
            .unwrap();
        let carries_method = module.functions.iter().any(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name.ends_with("Cell.flag"))
        });
        assert_eq!(carries_method, argument == "First");
        run(&module, expected);
    }
}
