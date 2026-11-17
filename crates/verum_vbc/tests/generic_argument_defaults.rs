//! Default and inactive-variant arguments retain actual producer declarations.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenError, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::VbcModule,
};

fn compiler() -> VbcCodegen {
    VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation().with_strict_codegen())
}

fn compile(source: &str) -> Result<VbcModule, CodegenError> {
    compiler().compile_module(&Parser::new(source).parse_module().unwrap())
}

fn source_and_wire(module: VbcModule, expected: i64) {
    verum_vbc::validate::validate_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, decoded] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("fixture.probe")
            .or_else(|| module.find_function_by_name("fixture.consumer.probe")).unwrap();
        let actual = Interpreter::new(Arc::new(module)).execute_function(entry).unwrap();
        assert_eq!(actual.try_as_i64(), Some(expected));
    }
}

#[test]
fn omitted_concrete_default_preserves_inferred_and_explicit_witnesses() {
    source_and_wire(compile(r#"
        fn selected<T>(value: T, extra: Int = 41) -> Int { extra + T.size }
        fn probe() -> Int { selected(true) + selected<Int>(7) }
    "#).unwrap(), 91);
}

#[test]
fn emitted_default_binds_an_otherwise_unconstrained_generic_slot() {
    // The result is always Int, so no expected-result equation can supply T.
    source_and_wire(compile(r#"
        fn selected<T>(value: T = 41) -> Int { T.size }
        fn probe() -> Int { selected() }
    "#).unwrap(), 8);
    source_and_wire(compile(r#"
        type Payload is { first: Int, second: Int };
        fn selected<T>(value: T = Payload { first: 3, second: 7 }) -> Int { T.size }
        fn probe() -> Int { selected() - Payload.size }
    "#).unwrap(), 0);
}

#[test]
fn generic_defaults_remain_with_the_selected_declaration_owner() {
    let provider = Parser::new("module fixture.provider; public fn selected<T>(value:T=41)->Int {T.size}")
        .parse_module().unwrap();
    let sibling = Parser::new("module fixture.sibling; public fn selected<T>(value:T=false)->Int {T.size}")
        .parse_module().unwrap();
    let consumer = Parser::new("module fixture.consumer; mount fixture.provider.selected; public fn probe()->Int {selected()}")
        .parse_module().unwrap();
    for reverse in [false, true] {
        let mut files = [("fixture", &provider), ("fixture", &sibling), ("fixture", &consumer)];
        if reverse { files.reverse(); }
        let mut generator = compiler();
        generator.initialize();
        generator.collect_owned_unit_declarations(&files).unwrap();
        generator.resolve_pending_imports();
        generator.compile_owned_unit_items(&files, ItemFailurePolicy::Strict).unwrap();
        source_and_wire(generator.finalize_module().unwrap(), 8);
    }
}

#[test]
fn omitted_required_extra_and_conflicting_default_arguments_are_refused() {
    for source in [
        "fn selected<T>(value:T, extra:Int=41)->Int {T.size} fn probe()->Int {selected()}",
        "fn selected<T>(value:T=41)->Int {T.size} fn probe()->Int {selected(1,2)}",
        "fn selected<T>(value:T, other:T=41)->Int {T.size} fn probe()->Int {selected(true)}",
        "fn selected<T>(value:T=41)->Int {T.size} fn probe()->Int {selected<Bool>()}",
        "fn selected<T>(value:T)->Int {T.size} fn probe()->Int {selected<Bool>(41)}",
        "fn selected<T>(value:T)->Int {T.size} fn probe()->Int {let value=41; selected<Bool>(value)}",
        "fn selected<T>(value:T)->Int {T.size} fn probe()->Int {selected<Int>(false)}",
    ] {
        assert!(compile(source).is_err(), "unproved default argument accepted: {source}");
    }
}

#[test]
fn contextual_numeric_arguments_and_defaults_obey_the_selected_range() {
    source_and_wire(compile(r#"
        fn selected<T>(value:T=41)->Int {T.size}
        fn probe()->Int {selected<UInt8>()+selected<UInt8>(255)+selected<Int8>(-128)}
    "#).unwrap(), 3);
    for call in ["selected<UInt8>(256)", "selected<Int8>(128)", "selected<UInt8>(-1)"] {
        assert!(compile(&format!("fn selected<T>(value:T)->Int {{T.size}} fn probe()->Int {{{call}}}")).is_err(),
            "out-of-range argument accepted: {call}");
    }
    assert!(compile("fn selected<T>(value:T=256)->Int {T.size} fn probe()->Int {selected<UInt8>()}").is_err());
}

#[test]
fn method_and_constructor_arguments_cannot_overwrite_explicit_witnesses() {
    for source in [
        "type Holder<T> is (T); fn probe()->Int {let value=Holder<Bool>(41); 0}",
        "type Receiver is (); implement Receiver {fn selected<T>(&self,value:T)->Int {T.size}} fn probe()->Int {Receiver().selected<Bool>(41)}",
    ] {
        assert!(compile(source).is_err(), "contradictory explicit witness accepted: {source}");
    }
    source_and_wire(compile(r#"
        type Receiver is ();
        implement Receiver {fn selected<T>(&self,value:T)->Int {T.size}}
        fn probe()->Int {Receiver().selected<UInt8>(255)}
    "#).unwrap(), 1);
}

#[test]
fn explicit_holes_are_checked_against_actual_operand_declarations() {
    source_and_wire(compile(r#"
        type Holder<T> is { value:T };
        fn selected<T>(value:T)->Int {T.size}
        fn probe()->Int {selected<_>(41)+selected<Holder<_>>(Holder{value:7})}
    "#).unwrap(), 16);
    assert!(compile(r#"
        type Holder<T> is { value:T };
        fn selected<T>(value:T)->Int {T.size}
        fn probe()->Int {selected<Holder<_>>(true)}
    "#).is_err());
}

#[test]
fn inactive_variants_use_the_actual_owner_and_contextual_arguments() {
    source_and_wire(compile(r#"
        type Packet<T> is Empty | Number(Int) | Held(T);
        fn selected<T>(value:T)->Int {1}
        fn probe()->Int {
            selected<Packet<Bool>>(Packet.Empty) + selected<Packet<Int>>(Packet.Number(27))
        }
    "#).unwrap(), 2);
}

#[test]
fn inactive_variants_cannot_invent_arguments_or_change_nominal_owner() {
    for argument in ["Packet.Empty", "Other.Empty", "Other.Number(27)"] {
        let call = if argument == "Packet.Empty" {
            "selected(Packet.Empty)".to_owned()
        } else {
            format!("selected<Packet<Int>>({argument})")
        };
        let source = format!(r#"
            type Packet<T> is Empty | Number(Int) | Held(T);
            type Other<T> is Empty | Number(Int) | Held(T);
            fn selected<T>(value:T)->Int {{1}}
            fn probe()->Int {{{call}}}
        "#);
        assert!(compile(&source).is_err(), "unproved constructor accepted: {call}");
    }
}
