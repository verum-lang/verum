//! T1536: one unresolved sum argument cannot erase another payload's identity.
#![cfg(feature = "codegen")]
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;

fn fields(source: &str, function: &str) -> List<u32> {
    let ast = Parser::new(source).parse_module().expect("source syntax");
    let module = VbcCodegen::with_config(CodegenConfig::new("consumer"))
        .compile_module(&ast)
        .expect("source compiles");
    let body = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some(function))
        .expect("probe function");
    let mut fields = List::new();
    for op in body.instructions.as_ref().expect("decoded body") {
        match op {
            Instruction::GetF { field_idx, .. } => fields.push(*field_idx),
            Instruction::GetFieldNamed { .. } => {
                panic!("payload identity must not rely on runtime field lookup")
            }
            _ => {}
        }
    }
    fields
}

#[test]
fn concrete_payload_survives_a_sibling_unresolved_generic_argument() {
    for pattern in ["Bad", "Choice.Bad"] {
        let source = format!(
            r#"
type Payload is {{ message: Int }};
type Distractor is {{ other: Int, message: Int }};
type Choice<Success, Failure> is Good(Success) | Bad(Failure);
type Noise is Bad(Text);
fn probe<T>(value: Choice<T, Payload>) -> Int {{
    match value {{ Good(_) => 0, {pattern}(info) => info.message }}
}}
"#
        );
        assert_eq!(fields(&source, "consumer.probe").as_slice(), [0]);
    }
}

#[test]
fn declaration_generic_slot_is_independent_of_variant_tag() {
    let source = r#"
type Payload is { other: Int, message: Int };
type Choice<Success, Failure> is Bad(Failure) | Good(Success);
fn probe<T>(value: Choice<T, Payload>) -> Int {
    match value { Bad(info) => info.message, Good(_) => 0 }
}
"#;
    assert_eq!(fields(source, "consumer.probe").as_slice(), [1]);
}

#[test]
fn a_partially_generic_payload_retains_its_nominal_owner() {
    let source = r#"
type Envelope<T> is { value: T, marker: Int };
type Choice<T, E> is Good(T) | Bad(Envelope<E>);
type Noise is Bad(Text);
fn probe<T>(value: Choice<Int, T>) -> Int {
    match value { Good(_) => 0, Bad(info) => info.marker }
}
"#;
    assert_eq!(fields(source, "consumer.probe").as_slice(), [1]);
}

#[test]
fn an_unknown_payload_does_not_inherit_a_colliding_constructor_type() {
    let source = r#"
type Payload is { message: Int };
type Choice<Success, Failure> is Good(Success) | Bad(Failure);
type Noise is Good(Text) | Bad(Text);
fn probe<T>(value: Choice<T, Payload>) -> Int {
    match value { Good(unresolved) => { unresolved.inspect(); 0 }, Bad(info) => info.message }
}
"#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("consumer"))
        .compile_module(&ast)
        .unwrap();
    let function = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("consumer.probe"))
        .unwrap();
    let calls: List<_> = function
        .instructions
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|op| {
            if let Instruction::CallM { method_id, .. } = op {
                module.get_string(verum_vbc::types::StringId(*method_id))
            } else {
                None
            }
        })
        .collect();
    assert!(calls.contains(&"inspect"));
    assert!(!calls.contains(&"Text.inspect"));
    assert_eq!(fields(source, "consumer.probe").as_slice(), [0]);
}

#[test]
fn imported_payload_keeps_its_qualified_owner_in_both_registration_orders() {
    let mut dependencies = List::new();
    for (owner, source) in [
        (
            "alpha",
            "module alpha; type Payload is { message: Int }; type Choice<T, E> is Good(T) | Bad(E);",
        ),
        (
            "beta",
            "module beta; type Payload is { other: Int, message: Int }; type Noise is Good(Text) | Bad(Payload);",
        ),
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let module = VbcCodegen::with_config(CodegenConfig::new(owner))
            .compile_module(&ast)
            .unwrap();
        dependencies.push(
            verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap(),
        );
    }
    for order in [[0, 1], [1, 0]] {
        let ast = Parser::new("module consumer; fn probe<T>(value: alpha.Choice<T, alpha.Payload>) -> Int { match value { alpha.Choice.Good(_) => 0, alpha.Choice.Bad(info) => info.message } }")
            .parse_module().unwrap();
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for i in order {
            codegen.import_archive_module_types(&dependencies[i]);
        }
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        let module = codegen.compile_function_bodies(&ast).unwrap();
        let function = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("consumer.probe"))
            .unwrap();
        let mut indices = List::new();
        for instruction in function.instructions.as_ref().unwrap() {
            match instruction {
                Instruction::GetF { field_idx, .. } => indices.push(*field_idx),
                Instruction::GetFieldNamed { .. } => {
                    panic!("imported descriptor must carry exact payload owner")
                }
                _ => {}
            }
        }
        assert_eq!(indices.as_slice(), [0]);
    }
}

#[test]
fn previous_compilation_cannot_authorize_an_originless_import() {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let first = Parser::new("type Payload is { message: Int };")
        .parse_module()
        .unwrap();
    let module = codegen.compile_module(&first).unwrap();
    let mut payload = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Payload"))
        .unwrap()
        .clone();
    assert!(payload.origin_module.is_none());
    let empty = Parser::new("").parse_module().unwrap();
    codegen.compile_module(&empty).unwrap();
    payload.name = verum_vbc::types::StringId(codegen.ctx_mut().intern_string_raw("Payload"));
    for field in &mut payload.fields {
        field.name = verum_vbc::types::StringId(
            codegen
                .ctx_mut()
                .intern_string_raw(module.get_string(field.name).unwrap()),
        );
    }
    codegen.register_archive_type(payload, "Payload".into());
    let ast = Parser::new("type Choice<T,E> is Good(T) | Bad(E); fn probe<T>(value: Choice<T,Payload>)->Int { match value { Good(_)=>0, Bad(info)=>{ info.inspect(); 0 } } }").parse_module().unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let module = codegen.compile_function_bodies(&ast).unwrap();
    let methods = called_methods(&module, "consumer.probe");
    assert!(
        methods.contains(&"inspect"),
        "unexpected imported owner: {methods:?}"
    );
    assert!(!methods.contains(&"Payload.inspect"));
}

#[test]
fn canonical_nested_sum_does_not_roundtrip_through_a_foreign_bare_name() {
    let source = "type Result<T,E> is Other(E); type Poll<T> is Ready(T); fn probe(value: Poll<core.base.result.Result<Int,Bool>>) -> Int { match value { Ready(payload) => { payload.inspect(); 0 } } }";
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let module = codegen.compile_module(&ast).unwrap();
    assert_eq!(
        called_methods(&module, "consumer.probe").as_slice(),
        ["core.base.result.Result.inspect"]
    );
}

fn called_methods<'a>(module: &'a verum_vbc::module::VbcModule, name: &str) -> List<&'a str> {
    let function = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some(name))
        .unwrap();
    function
        .instructions
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|op| {
            if let Instruction::CallM { method_id, .. } = op {
                module.get_string(verum_vbc::types::StringId(*method_id))
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn refutable_patterns_test_the_declared_result_of_the_scrutinee() {
    let source = r#"
        type Event is Empty | Ready(Int);
        type Noise is Ready(Text) | Empty;
        fn event(full: Bool) -> Event {
            if full { Event.Ready(7) } else { Event.Empty }
        }
        fn positive(full: Bool) -> Int {
            if let Ready(n) = event(full) { return n; }
            0
        }
        fn guard(full: Bool) -> Int {
            if event(full) is Ready(n) { return n; }
            0
        }
        fn required(full: Bool) -> Int {
            let Ready(n) = event(full) else { return 0; };
            n
        }
        fn looping(full: Bool) -> Int {
            while let Ready(n) = event(full) { return n; }
            0
        }
        fn main() -> Int {
            positive(true) + guard(true) + required(true) + looping(true)
                + positive(false) + guard(false) + required(false) + looping(false)
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("consumer")).compile_module(&ast).unwrap();
    let entry = module.find_function_by_name("consumer.main").unwrap();
    for name in ["positive", "guard", "required", "looping"] {
        let id = module.find_function_by_name(&format!("consumer.{name}")).unwrap();
        let code = module.get_function(id).unwrap().instructions.as_ref().unwrap();
        assert!(code.iter().any(|op| matches!(op, Instruction::IsVar { tag: 1, .. })), "{name}: {code:?}");
    }
    let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module));
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(28));
}

#[test]
fn explicit_variant_test_reads_the_serialized_owner_tag() {
    let module = |name, source| {
        VbcCodegen::with_config(CodegenConfig::new(name))
            .compile_module(&Parser::new(source).parse_module().unwrap()).unwrap()
    };
    let sum = module("alpha", "module alpha; type Event is Empty | Ready(Int);");
    let sum = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&sum).unwrap(),
    ).unwrap();
    let ast = Parser::new(r#"
        module consumer;
        mount alpha.Event;
        fn select(value: Event) -> Int {
            if let Event.Ready(n) = value { return n; }
            0
        }
    "#).parse_module().unwrap();
    let mut gen_ = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    gen_.import_archive_module_types(&sum);
    gen_.collect_unit_declarations(&[&ast]).unwrap();
    gen_.resolve_pending_imports();
    gen_.compile_unit_items(&[&ast], verum_vbc::codegen::ItemFailurePolicy::Strict).unwrap();
    let module = gen_.finalize_module_from_state().unwrap();
    let id = module.find_function_by_name("consumer.select").unwrap();
    let body = module.get_function(id).unwrap().instructions.as_ref().unwrap();
    assert!(body.iter().any(|op| matches!(op, Instruction::IsVar { tag: 1, .. })), "{body:?}");
}

#[test]
fn imported_named_variant_field_owns_its_conversion_target() {
    let dependency = Parser::new(r#"
        module owner;
        type Amount is { n: Int };
        type Event is Present { amount: Amount } | Absent;
    "#).parse_module().unwrap();
    let dependency = VbcCodegen::with_config(CodegenConfig::new("owner")).compile_module(&dependency).unwrap();
    let ast = Parser::new(r#"
        module consumer;
        mount owner.{Amount, Event};
        type Envelope is Wrap(Event);
        implement Envelope { fn from(n: Int) -> Envelope { panic("wrong target") } }
        implement Amount { fn from(n: Int) -> Amount { Amount { n: n + 2 } } }
        fn build() -> Envelope { Envelope.Wrap(Event.Present { amount: (40).into() }) }
        fn main() -> Int {
            match build() { Envelope.Wrap(Event.Present { amount }) => amount.n, _ => 0 }
        }
    "#).parse_module().unwrap();
    let mut gen_ = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    gen_.import_archive_module_types(&dependency);
    gen_.collect_unit_declarations(&[&ast]).unwrap();
    gen_.resolve_pending_imports();
    gen_.compile_unit_items(&[&ast], verum_vbc::codegen::ItemFailurePolicy::Strict).unwrap();
    let module = gen_.finalize_module_from_state().unwrap();
    let id = module.find_function_by_name("consumer.main").unwrap();
    assert_eq!(verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(id).unwrap().try_as_i64(), Some(42));
}

#[test]
fn archived_collection_payload_selects_native_iteration_by_owner() {
    for (owner, native) in [("core.collections.list", true), ("foreign", false)] {
        let source = format!("module {owner}; type List<T> is {{ storage: T }};");
        let ast = Parser::new(&source).parse_module().unwrap();
        let dependency = VbcCodegen::with_config(CodegenConfig::new(owner))
            .compile_module(&ast).unwrap();
        let mut source = format!(r#"
            module consumer;
            mount {owner}.List;
            type Packet<T> is Empty | Data(T);
            fn sum(packet: Packet<List<Int>>) -> Int {{
                let mut total = 0;
                if let Packet.Data(items) = packet {{
                    for value in items {{ total = total + value; }}
                }}
                total
            }}
        "#);
        if native {
            source.push_str("fn main() -> Int { sum(Packet.Data([17, 25])) }");
        }
        let ast = Parser::new(&source).parse_module().unwrap();
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        codegen.import_archive_module_types(&dependency);
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen.resolve_pending_imports();
        codegen.compile_unit_items(&[&ast], verum_vbc::codegen::ItemFailurePolicy::Strict).unwrap();
        let module = codegen.finalize_module_from_state().unwrap();
        let id = module.find_function_by_name("consumer.sum").unwrap();
        let body = module.get_function(id).unwrap().instructions.as_ref().unwrap();
        assert_eq!(body.iter().any(|op| matches!(op, Instruction::IterNew { .. })), native, "{owner}");
        if native {
            let main = module.find_function_by_name("consumer.main").unwrap();
            let mut vm = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module));
            assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(42));
        }
    }
}
