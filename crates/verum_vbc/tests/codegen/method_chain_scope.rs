#[test]
fn chained_shadow_parameter_does_not_capture_the_receivers_argument() {
    use super::VbcCodegen;
    use crate::codegen::CodegenConfig;
    let ast = verum_fast_parser::Parser::new(
        r#"
type Pair<A, B> is { left: A, right: B };
type Owner<T> is { value: T };
implement<T> Owner<T> {
    fn pair<T>(self, other: T) -> Pair<Self, T> { Pair { left: self, right: other } }
}
"#,
    )
    .parse_module()
    .expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("chain_scope"));
    codegen.compile_module(&ast).expect("compile");
    let name = codegen
        .resolved_method_return_type_name("Owner<Int>", "pair", &verum_common::List::new())
        .expect("result");
    assert!(name.contains("Owner<Int>"), "{name}");
    assert!(
        name.ends_with(", _>"),
        "method T must remain unbound: {name}"
    );
}
