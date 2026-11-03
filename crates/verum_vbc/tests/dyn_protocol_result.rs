#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::Instruction;
use verum_vbc::codegen::{CodegenConfig, CodegenError, ItemFailurePolicy, VbcCodegen};
use verum_vbc::module::VbcModule;

fn compile(bounds: &str, arm: &str) -> Result<VbcModule, CodegenError> {
    let api = Parser::new(
        "module api;
        public type Decision is Hold | Pick;
        public type Policy is protocol { fn resolve(&mut self, x: Int) -> Decision; };",
    )
    .parse_module()
    .unwrap();
    let source = format!(
        "module consumer; mount api.Policy;
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
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let api = VbcCodegen::with_config(CodegenConfig::new("api")).compile_module(&api)?;
    let bytes = verum_vbc::serialize::serialize_module(&api).unwrap();
    let archive = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    codegen.import_bootstrap_nominal_dependencies(&[&consumer], &[&archive])?;
    codegen.collect_unit_declarations(&[&consumer])?;
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
