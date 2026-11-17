//! Builtin precedence follows the declaration's lexical owner, including
//! retained protocol defaults and multi-source standard-library compilation.
#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{VbcModule, codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen}};

fn compile(sources: &[&str], prefer_existing: bool) -> VbcModule {
    let modules: List<_> = sources.iter()
        .map(|source| Parser::new(source).parse_module().expect("source grammar"))
        .collect();
    let units: List<_> = modules.iter().collect();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("application"));
    compiler.ctx_mut().prefer_existing_functions = prefer_existing;
    compiler.collect_unit_declarations(&units).expect("source declarations");
    compiler.compile_unit_items(&units, ItemFailurePolicy::Strict).expect("source bodies");
    compiler.finalize_module_from_state().expect("complete module")
}

fn check(module: VbcModule, expected: i64) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("application.main").unwrap();
        let result = verum_vbc::interpreter::Interpreter::try_new_validated(module.into())
            .unwrap().execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

#[test]
fn sibling_declarations_cannot_suppress_an_ambient_builtin() {
    let noise = "module unrelated; fn min(a: Int, b: Int) -> Int { 99 }";
    let application = "module application; fn main() -> Int { min(17, 91) }";
    for prefer_existing in [false, true] {
        for sources in [[noise, application], [application, noise]] {
            check(compile(&sources, prefer_existing), 17);
        }
    }
}

#[test]
fn a_default_body_keeps_ambient_builtins_despite_consumer_homonyms() {
    let owner = r#"
        module owner;
        public type Reader is protocol {
            fn sample(&self) -> Int { assert(true); min(17, 91) }
        };
    "#;
    let application = r#"
        module application;
        mount owner.Reader;
        type Subject is { marker: Int };
        implement Reader for Subject {}
        fn assert(value: Bool) { panic("consumer assert must not run"); }
        fn min(a: Int, b: Int) -> Int { 99 }
        fn main() -> Int { let subject = Subject { marker: 0 }; subject.sample() }
    "#;
    for prefer_existing in [false, true] {
        for sources in [[owner, application], [application, owner]] {
            check(compile(&sources, prefer_existing), 17);
        }
    }
}

#[test]
fn the_default_owner_can_shadow_a_builtin_in_both_compilation_modes() {
    let owner = r#"
        module owner;
        fn min(a: Int, b: Int) -> Int { a + b }
        public type Reader is protocol {
            fn sample(&self) -> Int { min(17, 91) }
        };
    "#;
    let application = r#"
        module application;
        mount owner.Reader;
        type Subject is { marker: Int };
        implement Reader for Subject {}
        fn min(a: Int, b: Int) -> Int { 99 }
        fn main() -> Int { let subject = Subject { marker: 0 }; subject.sample() }
    "#;
    for prefer_existing in [false, true] {
        for sources in [[owner, application], [application, owner]] {
            check(compile(&sources, prefer_existing), 108);
        }
    }
}

#[test]
fn source_and_nested_declarations_keep_precedence_over_builtins() {
    for application in [
        "module application; fn min(a: Int, b: Int) -> Int { a + b } fn main() -> Int { min(17, 91) }",
        "module application; fn main() -> Int { fn min(a: Int, b: Int) -> Int { a + b } min(17, 91) }",
    ] {
        for prefer_existing in [false, true] {
            check(compile(&[application], prefer_existing), 108);
        }
    }
}

#[test]
fn an_explicit_mount_keeps_precedence_over_a_builtin() {
    let provider = "module provider; public fn min(a: Int, b: Int) -> Int { a + b }";
    let application = "module application; mount provider.min; fn main() -> Int { min(17, 91) }";
    for prefer_existing in [false, true] {
        for sources in [[provider, application], [application, provider]] {
            check(compile(&sources, prefer_existing), 108);
        }
    }
}

#[test]
fn a_default_uses_its_own_mount_and_retains_the_declared_result() {
    let provider = r#"
        module provider;
        public type Answer is Wrong | Right;
        public fn min(a: Int, b: Int) -> Answer { Answer.Right }
    "#;
    let owner = r#"
        module owner;
        mount provider.{min, Answer};
        public type Reader is protocol {
            fn sample(&self) -> Int {
                match min(17, 91) { Answer.Wrong => 0, Answer.Right => 73 }
            }
        };
    "#;
    let application = r#"
        module application;
        mount owner.Reader;
        type Subject is { marker: Int };
        implement Reader for Subject {}
        fn min(a: Int, b: Int) -> Int { 99 }
        fn main() -> Int { let subject = Subject { marker: 0 }; subject.sample() }
    "#;
    for prefer_existing in [false, true] {
        for sources in [[provider, owner, application], [application, owner, provider]] {
            check(compile(&sources, prefer_existing), 73);
        }
    }
}
