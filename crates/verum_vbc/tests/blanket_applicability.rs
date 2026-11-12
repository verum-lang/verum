#![cfg(feature = "codegen")]
//! T2177: a declaration's complete, owner-qualified bounds control applicability.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, VbcCodegen},
};

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

fn execute(module: VbcModule, entry: &str) -> i64 {
    let entry = module
        .find_function_by_name(entry)
        .expect("real source entry");
    let value = verum_vbc::interpreter::Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("real bytecode execution");
    value.try_as_i64().expect("integer output")
}

fn carries_method(module: &VbcModule, owner: &str, receiver: &str, method: &str) -> bool {
    module.functions.iter().any(|function| {
        let name = module.get_string(function.name);
        name == Some(format!("{owner}.{receiver}.{method}").as_str())
            || (name == Some(format!("{receiver}.{method}").as_str())
                && function.origin_module.and_then(|id| module.get_string(id)) == Some(owner))
    })
}

fn conjunction(second: bool, reverse: bool, call: bool) -> CodegenResult<VbcModule> {
    let first = "implement A for Cell {}";
    let second = if second {
        "implement B for Cell {}"
    } else {
        ""
    };
    let implementations = if reverse {
        format!("{second}\n{first}")
    } else {
        format!("{first}\n{second}")
    };
    let action = if call { "cell.flag()" } else { "cell.value" };
    let source = format!(
        r#"
        module conjunction;
        type A is protocol {{}};
        type B is protocol {{}};
        type Granted is protocol {{ fn flag(&self)->Int; }};
        implement<X: A + B> Granted for X {{ fn flag(&self)->Int {{ 59 }} }}
        type Cell is {{ value: Int }};
        {implementations}
        fn main()->Int {{ let cell=Cell {{ value: 7 }}; {action} }}
    "#
    );
    let ast = Parser::new(&source)
        .parse_module()
        .expect("declared conjunction grammar");
    VbcCodegen::with_config(CodegenConfig::new("conjunction")).compile_module(&ast)
}

fn owner_case(actual_owner: &str, call: bool) -> CodegenResult<VbcModule> {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core;").unwrap();
    std::fs::write(
        core.join("alpha.vr"),
        r#"
        module core.alpha;
        public type Base is protocol {};
        public type Granted is protocol { fn token(&self)->Int; };
        implement<X: Base> Granted for X { fn token(&self)->Int { 73 } }
    "#,
    )
    .unwrap();
    std::fs::write(
        core.join("beta.vr"),
        r#"
        module core.beta;
        public type Base is protocol {};
    "#,
    )
    .unwrap();
    let action = if call { "cell.token()" } else { "cell.value" };
    let source = format!(
        r#"
        module core.consumer;
        mount core.alpha.Granted;
        mount core.{actual_owner}.Base;
        type Cell is {{ value: Int }};
        implement Base for Cell {{}}
        fn main()->Int {{ let cell=Cell {{ value: 11 }}; {action} }}
    "#
    );
    let path = core.join("consumer.vr");
    std::fs::write(&path, &source).unwrap();
    let ast = Parser::new(&source)
        .parse_module()
        .expect("actual module graph grammar");
    VbcCodegen::with_config(CodegenConfig::new("core.consumer")).compile_module_with_mounts(
        &ast,
        path.to_str().unwrap(),
        core.to_str().unwrap(),
    )
}

#[test]
fn an_unsatisfied_second_bound_cannot_publish_or_dispatch_a_blanket_method() {
    let module =
        conjunction(false, false, false).expect("unrelated receiver remains a valid program");
    for module in [module.clone(), wire(&module)] {
        assert!(
            !carries_method(&module, "conjunction", "Cell", "flag"),
            "A alone must not satisfy the blanket's A+B obligation"
        );
        assert_eq!(execute(module, "conjunction.main"), 7);
    }
    assert!(
        conjunction(false, false, true).is_err(),
        "a missing blanket obligation must refuse dispatch"
    );
}

#[test]
fn same_leaf_protocol_from_another_owner_cannot_supply_a_blanket_bound() {
    let module = owner_case("beta", false).expect("independent Base implementation is valid");
    for module in [module.clone(), wire(&module)] {
        assert!(
            !carries_method(&module, "core.consumer", "Cell", "token"),
            "core.beta.Base cannot satisfy the declaration core.alpha.Base"
        );
        assert_eq!(execute(module, "core.consumer.main"), 11);
    }
    assert!(
        owner_case("beta", true).is_err(),
        "foreign owner must refuse blanket dispatch"
    );
}

#[test]
fn every_satisfied_bound_materializes_one_method_independently_of_source_order() {
    for reverse in [false, true] {
        let module = conjunction(true, reverse, true).expect("both declared obligations hold");
        for module in [module.clone(), wire(&module)] {
            assert!(carries_method(&module, "conjunction", "Cell", "flag"));
            assert_eq!(execute(module, "conjunction.main"), 59);
        }
    }
}

#[test]
fn the_exact_protocol_owner_can_supply_the_blanket_bound() {
    let module = owner_case("alpha", true).expect("actual declared protocol owner");
    for module in [module.clone(), wire(&module)] {
        assert!(carries_method(&module, "core.consumer", "Cell", "token"));
        assert_eq!(execute(module, "core.consumer.main"), 73);
    }
}
