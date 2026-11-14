#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::Instruction;
use verum_vbc::codegen::{CodegenConfig, CodegenError, ItemFailurePolicy, VbcCodegen};
use verum_vbc::module::VbcModule;

#[path = "fixtures/canonical_deref.rs"]
mod canonical_deref;

fn compile(bounds: &str, arm: &str) -> Result<VbcModule, CodegenError> {
    let api = Parser::new(
        "module api;
        public type Decision is Hold | Pick;
        public type Policy is protocol { fn resolve(&mut self, x: Int) -> Decision; };",
    )
    .parse_module()
    .unwrap();
    let source = format!(
        "module consumer; mount api.Policy; mount core.base.memory.Heap;
        type Decoy is Pick | Foreign;
        type Marker is protocol {{ fn describe(&self) -> Text; }};
        type Other is protocol {{ fn resolve(&mut self, x: Int) -> Decoy; }};
        fn borrowed(p: &mut Heap<dyn {bounds}>) -> Int {{
            let r = (&mut **p).resolve(7);
            match r {{ {arm} => 1, _ => 0 }}
        }}
        fn implicit(p: &mut Heap<dyn {bounds}>) -> Int {{
            match p.resolve(7) {{ {arm} => 1, _ => 0 }}
        }}"
    );
    let consumer = Parser::new(&source).parse_module().unwrap();
    let protocols = canonical_deref::declarations();
    // These tests inspect dispatch without constructing a Heap. Import its
    // actual declaration and projection signatures, never a seeded name.
    let mut memory = Parser::new(include_str!("../../../core/base/memory.vr"))
        .parse_module().unwrap();
    memory.items.retain_mut(|item| match &mut item.kind {
        verum_ast::ItemKind::Module(decl) => decl.items.is_none(),
        verum_ast::ItemKind::Type(decl) => decl.name.name.as_str() == "Heap",
        verum_ast::ItemKind::Impl(decl) => {
            let target = match &decl.kind {
                verum_ast::decl::ImplKind::Inherent(target) => target,
                verum_ast::decl::ImplKind::Protocol { protocol, for_type, .. }
                    if protocol.segments.last().is_some_and(|part|
                        matches!(part, verum_ast::ty::PathSegment::Name(name)
                            if matches!(name.name.as_str(), "Deref" | "DerefMut"))) => for_type,
                _ => return false,
            };
            let is_heap = matches!(&target.kind, verum_ast::ty::TypeKind::Generic { base, .. }
                if matches!(&base.kind, verum_ast::ty::TypeKind::Path(path)
                    if path.segments.last().is_some_and(|part|
                        matches!(part, verum_ast::ty::PathSegment::Name(name) if name.name.as_str() == "Heap"))));
            if matches!(decl.kind, verum_ast::decl::ImplKind::Inherent(_)) {
                decl.items.retain(|member| matches!(&member.kind,
                    verum_ast::decl::ImplItemKind::Function(function)
                        if matches!(function.name.name.as_str(), "as_ref" | "as_mut")));
            }
            is_heap && !decl.items.is_empty()
        }
        _ => false,
    });
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let api = VbcCodegen::with_config(CodegenConfig::new("api")).compile_module(&api)?;
    let bytes = verum_vbc::serialize::serialize_module(&api).unwrap();
    let archive = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    codegen.import_bootstrap_nominal_dependencies(&[&consumer], &[&archive])?;
    codegen.collect_unit_declarations(&[&protocols, &memory, &consumer])?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&[&consumer], ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn qualified_protocol_owns_call_and_match_tags() {
    let module = compile("Policy", "Pick").unwrap();
    for name in ["consumer.borrowed", "consumer.implicit"] {
        let function = module
            .get_function(module.find_function_by_name(name).unwrap())
            .unwrap();
        let start = function.bytecode_offset as usize;
        let ops = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        assert!(ops.iter().any(|op| matches!(op, Instruction::CallM { method_id, .. }
            if module.get_string(verum_vbc::types::StringId(*method_id)) == Some("dyn:api.Policy.resolve"))),
            "{name}: {ops:?}");
        let tags: Vec<_> = ops
            .iter()
            .filter_map(|op| {
                if let Instruction::IsVar { tag, .. } = op {
                    Some(*tag)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(tags, [1], "{name}: {ops:?}");
    }
}

#[test]
fn wrong_protocol_variant_is_refused() {
    let error = compile("Policy", "Foreign").unwrap_err().to_string();
    assert!(error.contains("no declared tag"), "{error}");
}

#[test]
fn second_dynamic_bound_can_select_the_unique_member() {
    let module = compile("Marker + Policy", "Pick").unwrap();
    let function = module
        .get_function(module.find_function_by_name("consumer.implicit").unwrap())
        .unwrap();
    let start = function.bytecode_offset as usize;
    let ops = verum_vbc::bytecode::decode_instructions(
        &module.bytecode[start..start + function.bytecode_length as usize],
    )
    .unwrap();
    assert!(ops.iter().any(|op| matches!(op, Instruction::CallM { method_id, .. }
        if module.get_string(verum_vbc::types::StringId(*method_id)) == Some("dyn:api.Policy.resolve"))));
}

#[test]
fn conflicting_protocol_declarations_are_refused() {
    let error = compile("Policy + Other", "Pick").unwrap_err().to_string();
    assert!(error.contains("ambiguous protocol method"), "{error}");
}

#[test]
fn declared_protocol_decisions_execute_through_the_selected_implementation() {
    let ast = Parser::new(
        "module service;
        type Decision is Hold | Pick;
        type Policy is protocol { fn resolve(&mut self, x: Int) -> Decision; };
        type Choice is { marker: Int };
        implement Policy for Choice {
            fn resolve(&mut self, x: Int) -> Decision {
                if x == self.marker { Pick } else { Hold }
            }
        }
        fn route(p: &mut dyn Policy, x: Int) -> Int {
            match p.resolve(x) { Pick => 13, Hold => 7 }
        }
        fn run() -> Int {
            let mut choice = Choice { marker: 11 };
            route(&mut choice, 11) * 100 + route(&mut choice, 12)
        }",
    )
    .parse_module()
    .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("service"));
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen
        .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for artifact in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let entry = artifact.find_function_by_name("service.run").unwrap();
        let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(artifact))
            .execute_function(entry)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(1307));
    }
}

fn execute_declared_result(body: &str) {
    let ast = Parser::new(&format!(r#"
        module typed_object;
        mount core.base.protocols.Deref;
        type Key is Int {{ it >= 0 }};
        type Source<T> is protocol {{ fn key(&self) -> T; }};
        type Value is {{ key: Int }};
        implement Source<Key> for Value {{ fn key(&self) -> Key {{ self.key }} }}
        type Carrier<T> is {{ value: T }};
        implement<T> Deref for Carrier<T> {{
            type Target = T;
            fn deref(&self) -> &T {{ &self.value }}
        }}
        fn keep<T>(mut value: T) -> T {{ value }}
        fn read<T>(value: &dyn Source<T>) -> T {{ keep(value.key()) }}
        fn run() -> Int {{
            let value = Value {{ key: 73 }};
            {body}
        }}
    "#)).parse_module().expect("typed dynamic source");
    let module = canonical_deref::compile(CodegenConfig::new("typed_object"), &ast)
        .expect("exact dynamic receiver and result");
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("typed_object.run").unwrap();
    let result = verum_vbc::interpreter::Interpreter::new(module.into())
        .execute_function(entry).expect("dynamic declared result execution");
    assert_eq!(result.try_as_i64(), Some(73));
}

#[test]
fn applied_dynamic_bound_supplies_a_generic_call_result_witness() {
    execute_declared_result("let object: &dyn Source<Key> = &value; keep(object.key())");
}

#[test]
fn qualified_user_carrier_forwards_the_dynamic_result_declaration() {
    execute_declared_result(r#"
        let object: &dyn Source<Key> = &value;
        let carrier: typed_object.Carrier<&dyn Source<Key>> = Carrier { value: object };
        keep(carrier.key())
    "#);
}

#[test]
fn dynamic_bound_lowers_caller_parameters_out_of_its_existential_scope() {
    execute_declared_result("read<Key>(&value)");
}

#[test]
fn hidden_subject_cannot_become_the_callers_same_numbered_parameter() {
    let source = Parser::new(r#"
        type Source<T = Self> is protocol { fn key(&self) -> T; };
        fn read<T>(value: &dyn Source) -> T { value.key() }
    "#).parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&source)
        .expect_err("a hidden implementor is not the caller's T");
    assert!(matches!(error.kind, verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)),
        "{error}");
    assert!(error.to_string().contains("unresolved existential subject"), "{error}");
}
