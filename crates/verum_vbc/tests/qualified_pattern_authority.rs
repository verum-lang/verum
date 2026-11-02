#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
    module::VbcModule,
};

fn compile(source: &str) -> Result<VbcModule, String> {
    let module = Parser::new(source).parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new("probe"))
        .compile_module(&module)
        .map_err(|error| error.to_string())
}

#[test]
fn qualified_variant_tests_use_the_exact_descriptor() {
    let module = compile(
        r#"
        type State is Cold | Ready;
        type Decoy is Ready | Cold;
        type Alias is State;
        fn value(s: State) -> Int {
            match s { Alias.Ready => 7, State.Cold => 2 }
        }
        fn main() -> Int { value(State.Ready) * 10 + value(State.Cold) }
    "#,
    )
    .unwrap();
    let entry = module.find_function_by_name("probe.main").unwrap();
    let result = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert_eq!(result.try_as_i64(), Some(72));
}

#[test]
fn qualified_pattern_cannot_borrow_a_foreign_member() {
    for owner in ["State", "Missing"] {
        let failure = compile(&format!(
            r#"
            type State is Cold | Warm;
            type Foreign is Ready;
            fn matches(s: State) -> Bool {{ s is {owner}.Ready }}
        "#
        ))
        .expect_err("the only Ready constructor belongs to Foreign");
        assert!(failure.contains("Ready"), "{failure}");
    }
}

#[test]
fn unresolved_qualified_scalar_constant_cannot_match_zero() {
    let failure = compile("fn matches(n: Int) -> Bool { n is Missing.Unknown }")
        .expect_err("missing scalar declaration");
    assert!(failure.contains("Missing.Unknown"), "{failure}");
}

#[test]
fn a_module_qualified_scalar_constant_matches_its_declared_value() {
    let codes = Parser::new("module probe.codes; public const INVALID: Int = 37;")
        .parse_module()
        .unwrap();
    let app = Parser::new(
        r#"
        module probe;
        mount probe.codes as errno;
        fn check(n: Int) -> Int { match n { errno.INVALID => 7, _ => 2 } }
        fn main() -> Int { check(37) * 10 + check(0) }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("probe"));
    cg.collect_unit_declarations(&[&codes, &app]).unwrap();
    cg.resolve_pending_imports();
    cg.compile_unit_items(
        &[&codes, &app],
        verum_vbc::codegen::ItemFailurePolicy::Strict,
    )
    .unwrap();
    let module = cg.finalize_module_from_state().unwrap();
    let entry = module.find_function_by_name("probe.main").unwrap();
    let result = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert_eq!(result.try_as_i64(), Some(72));
}

#[test]
fn relative_mount_of_an_alias_keeps_its_declaring_sum() {
    let origin = Parser::new(
        r#"
        module probe.kinds;
        public type Kind is Medium | Small;
        public type PageKind is Kind;
    "#,
    )
    .parse_module()
    .unwrap();
    let user = Parser::new(
        r#"
        module probe.alloc;
        mount super.kinds.{Kind, PageKind};
        fn small(value: Kind) -> Bool { value is PageKind.Small }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("probe"));
    cg.collect_unit_declarations(&[&origin, &user]).unwrap();
    cg.resolve_pending_imports();
    cg.compile_unit_items(
        &[&origin, &user],
        verum_vbc::codegen::ItemFailurePolicy::Strict,
    )
    .unwrap();
    let module = cg.finalize_module_from_state().unwrap();
    let body = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.ends_with(".small"))
        })
        .unwrap()
        .instructions
        .as_ref()
        .unwrap();
    assert!(body.iter().any(|op| matches!(
        op,
        verum_vbc::instruction::Instruction::IsVar { tag: 1, .. }
    )));
}
