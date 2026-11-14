//! Unsupported runtime predicates must stop strict bytecode production.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;

#[test]
fn unresolved_predicate_never_omits_its_runtime_obligation() {
    for source in [
        "fn inspect(value:Int{absent(it)})->Int {value}",
        "fn inspect(value:Int)->Int{absent(it)} {value}",
        "fn inspect(value:Int)->Int {let checked_value:Int{absent(it)}=value; checked_value}",
        "type Checked is {value:Int{absent(it)}}; fn inspect(value:Int)->Checked {Checked{value:value}}",
        "type Classified is Valid(Int{absent(it)}) | Empty; fn inspect(value:Int)->Classified {Classified.Valid(value)}",
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let result = VbcCodegen::new().compile_module(&ast);
        assert!(
            result.is_err(),
            "unresolved predicate was silently accepted: {source}"
        );
    }
}

#[test]
fn supported_predicates_still_accept_and_refuse_at_each_boundary() {
    for source in [
        "fn inspect(it:Int{it>0})->Int {it}",
        "fn inspect(value:Int{self>0})->Int {value}",
        "fn inspect(value:Int where |selected| selected>0)->Int {value}",
        "fn inspect(value:Int)->Int{it>0} {value}",
        "fn inspect(value:Int)->Int{it>0} {return value;}",
        "fn inspect(value:Int)->Int {let checked_value:Int{it>0}=value; checked_value}",
        "type Checked is {value:Int{it>0}}; fn inspect(value:Int)->Int {let item=Checked{value:value}; item.value}",
        "type Classified is Valid(Int{it>0}) | Empty; fn inspect(value:Int)->Int {let item=Classified.Valid(value); value}",
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("inspect").unwrap();
        for value in [-73, 73] {
            let mut vm = verum_vbc::interpreter::Interpreter::new(module.clone().into());
            vm.run_global_ctors().unwrap();
            let result = vm.execute_function_with_args(entry, &[verum_vbc::Value::from_i64(value)]);
            if value > 0 {
                assert_eq!(result.unwrap().try_as_i64(), Some(value), "{source}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(verum_vbc::interpreter::InterpreterError::AssertionFailed { .. })
                    ),
                    "{source}: {result:?}"
                );
            }
        }
    }
}

#[test]
fn inline_predicate_keeps_earlier_parameter_binding() {
    let ast = Parser::new("fn inspect(limit:Int,value:Int{it>limit})->Int {value}")
        .parse_module()
        .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let entry = module.find_function_by_name("inspect").unwrap();
    for (limit, accepted) in [(100, false), (0, true)] {
        let mut vm = verum_vbc::interpreter::Interpreter::new(module.clone().into());
        let result = vm.execute_function_with_args(
            entry,
            &[
                verum_vbc::Value::from_i64(limit),
                verum_vbc::Value::from_i64(73),
            ],
        );
        if accepted {
            assert_eq!(result.unwrap().try_as_i64(), Some(73));
        } else {
            assert!(
                matches!(
                    result,
                    Err(verum_vbc::interpreter::InterpreterError::AssertionFailed { .. })
                ),
                "{result:?}"
            );
        }
    }
}

#[test]
fn same_named_variant_payloads_keep_the_selected_owner() {
    let ast = Parser::new(
        "module first {
            public type Classified is Valid(Int{it>0}) | Empty;
            public fn inspect(value:Int)->Int {let item=Classified.Valid(value); value}
        }
        module second {
            public type Classified is Valid(Int{it<0}) | Empty;
            public fn inspect(value:Int)->Int {let item=Classified.Valid(value); value}
        }",
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    for (name, valid) in [("first.inspect", 73), ("second.inspect", -73)] {
        let entry = module.find_function_by_name(name).unwrap();
        for value in [valid, -valid] {
            let mut vm = verum_vbc::interpreter::Interpreter::new(module.clone().into());
            vm.run_global_ctors().unwrap();
            let result = vm.execute_function_with_args(entry, &[verum_vbc::Value::from_i64(value)]);
            if value == valid {
                assert_eq!(result.unwrap().try_as_i64(), Some(value), "{name}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(verum_vbc::interpreter::InterpreterError::AssertionFailed { .. })
                    ),
                    "{name}: {result:?}"
                );
            }
        }
    }
}

#[test]
fn seeded_constructor_cannot_replace_its_payload_declaration() {
    let ast = Parser::new("fn inspect()->Result<Int,Int> {Result.Ok(73)}")
        .parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast)
        .expect_err("a name/tag seed is not source payload evidence");
    assert!(error.to_string().contains("no exact declaration"), "{error}");
}

#[test]
fn non_boolean_predicate_cannot_be_encoded_as_truthiness() {
    for predicate in ["73", "wrong(it)", "it && true", "!it"] {
        let source = format!("fn wrong(value:Int)->Int {{value}}
fn inspect(value:Int{{{predicate}}})->Int {{value}}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let error = VbcCodegen::new().compile_module(&ast)
            .expect_err("a non-Bool predicate must not reach a wire Assert");
        assert!(error.to_string().contains("Bool"), "{error}");
    }
}
