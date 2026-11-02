#![cfg(feature = "codegen")]
//! T1593: unresolved constructor ownership cannot become declaration-order bytecode.
use std::sync::Arc;
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;

fn source(reversed: bool, payload: bool, body: &str) -> Text {
    let (a, b) = if payload {
        (
            "type A is Pending(Int) | Done;",
            "type B is Failed | Pending(Int);",
        )
    } else {
        ("type A is Pending | Done;", "type B is Failed | Pending;")
    };
    if reversed {
        format!("{b} {a} {body}").into()
    } else {
        format!("{a} {b} {body}").into()
    }
}

fn reject_ambiguous(payload: bool) {
    for reversed in [false, true] {
        let body = if payload {
            "fn probe() { let value = Pending(7); }"
        } else {
            "fn probe() { let value = Pending; }"
        };
        let text = source(reversed, payload, body);
        let ast = Parser::new(&text).parse_module().expect("grammar");
        let result = VbcCodegen::new().compile_module(&ast);
        assert!(
            result.is_err(),
            "ambiguous payload={payload} reverse={reversed} must not emit bytecode"
        );
        let error = result.expect_err("checked error");
        assert!(error.to_string().contains("E431"), "{error}");
    }
}
#[test]
fn unresolved_unit_constructor_is_rejected() {
    reject_ambiguous(false);
}
#[test]
fn unresolved_payload_constructor_is_rejected() {
    reject_ambiguous(true);
}

fn run(text: &str) -> i64 {
    let ast = Parser::new(text).parse_module().expect("grammar");
    let module = VbcCodegen::new()
        .compile_module(&ast)
        .expect("known owner compiles");
    for (function, owner) in [("a", "A"), ("b", "B")] {
        if let Some(fd) = module
            .functions
            .iter()
            .find(|fd| module.get_string(fd.name) == Some(function))
        {
            let owner_id = module
                .types
                .iter()
                .find(|ty| module.get_string(ty.name) == Some(owner))
                .expect("owner descriptor")
                .id
                .0;
            let mut found = false;
            for instruction in fd.instructions.as_ref().expect("source instructions") {
                if let Instruction::MakeVariantTyped { type_id, .. } = instruction {
                    assert_eq!(
                        *type_id, owner_id,
                        "{function} must construct its actual declared owner"
                    );
                    found = true;
                }
            }
            assert!(
                found,
                "{function} must carry a typed constructor: {:?}",
                fd.instructions
            );
        }
    }
    let entry = module
        .functions
        .iter()
        .find(|fd| module.get_string(fd.name) == Some("probe"))
        .expect("probe")
        .id;
    let value = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert!(value.is_int(), "{value:?}");
    value.as_i64()
}

#[test]
fn known_expected_and_qualified_owners_execute_in_both_orders() {
    for reversed in [false, true] {
        for ctor_a in ["Pending", "A.Pending"] {
            for ctor_b in ["Pending", "B.Pending"] {
                let body: Text = format!("fn a() -> A {{ {ctor_a} }} fn b() -> B {{ {ctor_b} }} fn probe() -> Int {{ match a() {{ A.Pending => 30, _ => -100 }} + match b() {{ B.Pending => 7, _ => -100 }} }}").into();
                assert_eq!(run(&source(reversed, false, &body)), 37);
            }
        }
    }
}

#[test]
fn callee_parameter_owner_and_lexical_binding_execute() {
    for reversed in [false, true] {
        let body = "fn take_a(value: A) -> Int { match value { A.Pending => 30, _ => -100 } } fn take_b(value: B) -> Int { match value { B.Pending => 7, _ => -100 } } fn probe() -> Int { take_a(Pending) + take_b(Pending) }";
        assert_eq!(run(&source(reversed, false, body)), 37);
        assert_eq!(
            run(&source(
                reversed,
                false,
                "fn probe() -> Int { let Pending = 37; Pending }"
            )),
            37
        );
    }
}

#[test]
fn payload_context_and_function_shadowing_execute() {
    for reversed in [false, true] {
        let body = "fn a() -> A { Pending(30) } fn b() -> B { Pending(7) } fn probe() -> Int { match a() { A.Pending(value) => value, _ => -100 } + match b() { B.Pending(value) => value, _ => -100 } }";
        assert_eq!(run(&source(reversed, true, body)), 37);
        let body = "fn Pending(value: Int) -> Int { value + 30 } fn probe() -> Int { Pending(7) }";
        assert_eq!(run(&source(reversed, true, body)), 37);
        let body = "fn probe() -> Int { let Pending = |value: Int| value + 30; Pending(7) }";
        assert_eq!(run(&source(reversed, true, body)), 37);
    }
}

#[test]
fn enclosing_return_owner_does_not_type_an_unannotated_initializer() {
    for reversed in [false, true] {
        for initializer in [
            "Pending(7)",
            "{ Pending(7) }",
            "if true { Pending(7) } else { A.Done }",
        ] {
            let text = source(
                reversed,
                true,
                &format!("fn probe() -> A {{ let value = {initializer}; A.Done }}"),
            );
            let ast = Parser::new(&text).parse_module().expect("grammar");
            let error = VbcCodegen::new()
                .compile_module(&ast)
                .expect_err("initializer has no expected owner");
            assert!(error.to_string().contains("E431"), "{error}");
        }
        let body = "fn probe() -> Int { let value: B = { Pending(37) }; match value { B.Pending(value) => value, _ => -1 } }";
        assert_eq!(run(&source(reversed, true, body)), 37);
    }
}

#[test]
fn codegen_reuse_does_not_retain_old_constructor_collisions() {
    let first = Parser::new(&source(false, false, "fn ignored() {} "))
        .parse_module()
        .expect("grammar");
    let second = Parser::new("type A is Pending | Done; fn probe() -> Int { let value = Pending; match value { A.Pending => 37, _ => 0 } }").parse_module().expect("grammar");
    let mut codegen = VbcCodegen::new();
    codegen.compile_module(&first).expect("first unit");
    let module = codegen
        .compile_module(&second)
        .expect("old collisions must not block own bare slot registration");
    let entry = module
        .functions
        .iter()
        .find(|fd| module.get_string(fd.name) == Some("probe"))
        .expect("probe")
        .id;
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .expect("execute")
            .as_i64(),
        37
    );
}

#[test]
fn ambiguity_is_fatal_even_in_lenient_body_compilation() {
    let text = source(false, true, "fn probe() { let value = Pending(7); }");
    let ast = Parser::new(&text).parse_module().expect("grammar");
    let mut codegen = VbcCodegen::new();
    codegen
        .collect_unit_declarations(&[&ast])
        .expect("declarations");
    let error = codegen
        .compile_module_items_lenient(&ast)
        .expect_err("unresolved source ownership must not become a panic stub");
    assert!(error.to_string().contains("E431"), "{error}");
}

#[test]
fn file_local_and_mounted_constructors_execute_with_their_declared_tags() {
    use verum_vbc::codegen::ItemFailurePolicy;
    let sources = [
        "module first; public type A is Other | Clash(Int); fn first_probe() -> Int { let x = Clash(7); match x { A.Clash(v) => v, _ => -1 } }",
        "module second; public type B is Clash(Int) | Other; fn second_probe() -> Int { let x = Clash(9); match x { B.Clash(v) => v, _ => -2 } }",
        "module holder; public type Register is Blank | Num(Int);",
        "module elsewhere; public type Pair is Num(Int, Int) | Empty;",
        "module machine; mount holder.Register; fn make_num() -> Int { let v = Num(23); match v { Register.Num(x) => x, _ => -4 } }",
        "module client; mount first.Clash; fn mounted_probe() -> Int { let x = Clash(17); match x { A.Clash(v) => v, _ => -3 } }",
    ];
    for reverse in [false, true] {
        let mut modules: Vec<_> = sources
            .iter()
            .map(|s| Parser::new(s).parse_module().unwrap())
            .collect();
        if reverse {
            modules.reverse();
        }
        let units: Vec<_> = modules.iter().collect();
        let mut cg = VbcCodegen::new();
        cg.collect_unit_declarations(&units).unwrap();
        cg.compile_unit_items(&units, ItemFailurePolicy::Strict)
            .unwrap();
        let module = cg.finalize_module().unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        let decoded = Arc::new(verum_vbc::deserialize::deserialize_module(&wire).unwrap());
        for (entry, expected, tag) in [
            ("first.first_probe", 7, 1),
            ("second.second_probe", 9, 0),
            ("client.mounted_probe", 17, 1),
            ("machine.make_num", 23, 1),
        ] {
            let id = decoded.find_function_by_name(entry).expect(entry);
            let body = module.functions.iter().find(|f| f.id == id).unwrap();
            let typed = body.instructions.as_ref().unwrap().iter().any(|op| {
                matches!(op,
                Instruction::MakeVariantTyped { tag: actual, .. } if *actual == tag)
            });
            assert!(typed, "{entry}: {:?}", body.instructions);
            let result = Interpreter::new(decoded.clone())
                .execute_function(id)
                .unwrap();
            assert_eq!(result.as_i64(), expected, "{entry}, reverse={reverse}");
        }
    }
}
