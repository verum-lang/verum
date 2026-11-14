//! Legacy names cannot stand in for executed CBGR operations.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    interpreter::{Interpreter, InterpreterError, InterpreterResult},
    serialize::serialize_module,
    value::Value,
};

// The old dispatch group accepted these identities without performing their
// operation. Their source declarations below exercise both invocation routes.
const LEGACY: [(&str, &str, &str); 8] = [
    ("verum_cbgr_advance_epoch", "", ""),
    ("cbgr_advance_epoch", "", ""),
    ("verum_cbgr_new_generation", "", ""),
    ("verum_cbgr_invalidate", "reference: &mut Int", "&mut value"),
    ("verum_cbgr_get_generation", "reference: &Int", "&value"),
    (
        "verum_cbgr_advance_generation",
        "reference: &mut Int",
        "&mut value",
    ),
    ("verum_cbgr_get_epoch_caps", "reference: &Int", "&value"),
    ("verum_cbgr_get_stats", "", ""),
];

fn execute(source: &str, wire: bool) -> InterpreterResult<Value> {
    let ast = Parser::new(source).parse_module().expect(source);
    let module = VbcCodegen::new().compile_module(&ast).expect(source);
    let module = if wire {
        deserialize_module(&serialize_module(&module).unwrap()).unwrap()
    } else {
        module
    };
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("main").unwrap();
    Interpreter::new(module.into()).execute_function(entry)
}

fn refusal(identity: &str, parameters: &str, arguments: &str, indirect: bool) {
    let invoke = if indirect {
        format!("let operation = legacy; operation({arguments});")
    } else {
        format!("legacy({arguments});")
    };
    let source = format!(
        "@intrinsic(\"{identity}\") fn legacy({parameters}) -> Unit;
         fn main() -> Int {{ let mut value = 73; {invoke} 73 }}"
    );
    for wire in [false, true] {
        let result = execute(&source, wire);
        assert!(
            matches!(&result, Err(InterpreterError::UnsupportedIntrinsic { intrinsic, .. })
                if intrinsic.as_str() == identity),
            "identity={identity}, indirect={indirect}, wire={wire}: {result:?}"
        );
    }
}

#[test]
fn legacy_function_values_cannot_report_success_without_executing_an_operation() {
    for (identity, parameters, arguments) in LEGACY {
        refusal(identity, parameters, arguments, true);
    }
}

#[test]
fn unregistered_legacy_identities_cannot_report_direct_call_success() {
    for (identity, parameters, arguments) in LEGACY {
        // A registered direct call has a real VBC lowering, checked below.
        if verum_vbc::intrinsics::lookup_intrinsic(identity).is_none() {
            refusal(identity, parameters, arguments, false);
        }
    }
}

#[test]
fn ordinary_same_spelling_bodies_keep_their_source_semantics() {
    for (identity, _, _) in LEGACY {
        for invoke in [
            format!("{identity}(73)"),
            format!("let operation = {identity}; operation(73)"),
        ] {
            let source = format!(
                "fn {identity}(value: Int) -> Int {{ value }}
                 fn main() -> Int {{ {invoke} }}"
            );
            for wire in [false, true] {
                assert_eq!(execute(&source, wire).unwrap().try_as_i64(), Some(73));
            }
        }
    }
}

#[test]
fn registered_operations_still_mutate_real_epoch_and_generation_state() {
    let source = r#"
        @intrinsic("cbgr_advance_epoch") fn advance() -> Unit;
        @intrinsic("cbgr_current_epoch") fn current() -> UInt32;
        fn generation() -> UInt32 { @vbc(CBGR_NEW_GENERATION) }
        fn main() -> Bool {
            let before = current();
            advance();
            let after = current();
            let operation = generation;
            let first = operation();
            let second = operation();
            after > before && first > 0 && second > first
        }
    "#;
    for wire in [false, true] {
        assert_eq!(execute(source, wire).unwrap().try_as_bool(), Some(true));
    }
}
