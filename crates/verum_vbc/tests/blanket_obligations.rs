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
