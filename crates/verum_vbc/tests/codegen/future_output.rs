//! T1509: associated Output projections must retain the concrete layout.

use super::VbcCodegen;
use crate::codegen::CodegenConfig;
use verum_common::{Maybe, Text};
use verum_fast_parser::Parser;

fn codegen_for(declarations: &str) -> VbcCodegen {
    let source = format!("type Future is protocol {{ type Output; }};\n{declarations}");
    let module = Parser::new(&source).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("future_output"));
    codegen
        .compile_module(&module)
        .expect("compile declarations");
    codegen
}

fn output_type(declarations: &str, name: &str) -> Maybe<Text> {
    codegen_for(declarations).future_output_type_name(name)
}

const WRAPPERS: &str = r#"
type Result<T, E> is Ok(T) | Err(E);
type Record<T> is { padding: Int, answer: T };
type Ready<T> is { value: T };
implement<T> Future for Ready<T> { type Output = Record<T>; }
type Wrapper<F, E> is { inner: F, error: E };
implement<F: Future, E> Future for Wrapper<F, E> {
    type Output = Result<F.Output, E>;
}
"#;

#[test]
fn substitutes_projection_base_and_its_own_generic_arguments() {
    assert_eq!(
        output_type(WRAPPERS, "Wrapper<Ready<Int>, Bool>"),
        Maybe::Some(Text::from("Result<Record<Int>, Bool>")),
    );
}

#[test]
fn follows_multiple_output_projections_without_losing_argument_order() {
    assert_eq!(
        output_type(WRAPPERS, "Wrapper<Wrapper<Ready<Int>, Bool>, Text>"),
        Maybe::Some(Text::from("Result<Result<Record<Int>, Bool>, Text>")),
    );
}

#[test]
fn unrelated_associated_name_is_not_replaced_by_output() {
    let declarations = format!(
        "{WRAPPERS}\ntype Other<F> is {{ inner: F }};\nimplement<F: Future> Future for Other<F> {{ type Output = F.Item; }}"
    );
    assert_eq!(output_type(&declarations, "Other<Ready<Int>>"), Maybe::None);
}

#[test]
fn projection_base_without_future_implementation_stays_unknown() {
    let declarations = format!("{WRAPPERS}\ntype Ordinary is {{ value: Int }};");
    assert_eq!(
        output_type(&declarations, "Wrapper<Ordinary, Bool>"),
        Maybe::None
    );
}

#[test]
fn cyclic_output_projection_stays_unknown() {
    assert_eq!(
        output_type(
            "type Cycle is (); implement Future for Cycle { type Output = Cycle.Output; }",
            "Cycle",
        ),
        Maybe::None,
    );
}

#[test]
fn growing_projection_cycle_has_a_recursion_bound() {
    use crate::types::{TypeParamId, TypeRef};
    let mut codegen = codegen_for(
        "type Grow<T> is { value: T }; implement<T> Future for Grow<T> { type Output = T.Output; }",
    );
    let id = codegen.type_name_to_id["Grow"];
    let descriptor = codegen.types.iter_mut().find(|t| t.id == id).expect("Grow");
    // An archive can carry projections whose base is an instantiation,
    // even though the parser does not yet accept Grow<Grow<T>>.Output.
    // Each expansion grows the name, so exact-cycle detection alone is
    // insufficient for this descriptor graph.
    descriptor.protocols[0].associated_types[0].1 = TypeRef::AssociatedProjection {
        base: Box::new(TypeRef::Instantiated {
            base: id,
            args: [TypeRef::Instantiated {
                base: id,
                args: [TypeRef::Generic(TypeParamId(0))].into(),
            }]
            .into(),
        }),
        assoc: "Output".into(),
    };
    assert_eq!(codegen.future_output_type_name("Grow<Int>"), Maybe::None,);
}
