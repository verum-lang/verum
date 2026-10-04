//! T1519 source boundary: parameter annotations preserve nominal owners before
//! field metadata and method lookup. Ordinary/checked references keep the
//! existing field-carrier policy; this does not change borrowing or autoderef.
//! No test injects variable_type_names or supplies a pre-resolved call target.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;

fn body_targets(source: &str) -> Vec<Vec<String>> {
    let mut sources = Vec::new();
    let mut registries = Vec::new();
    let mut field_names = Vec::new();
    let mut layouts = Vec::new();
    for (owner, offset, result) in [("alpha", 1000, 7), ("beta", 2000, 13)] {
        let ast = Parser::new(&format!("module {owner}; type Leaf is {{ n: Int }}; type Envelope is {{ value: Leaf }}; implement Leaf {{ fn identity(&self) -> Int {{ {result} }} }}")).parse_module().unwrap();
        let mut cg = VbcCodegen::with_config(CodegenConfig::new(owner));
        sources.push(cg.compile_module(&ast).unwrap());
        let mut registry = cg.export_functions();
        for info in registry.values_mut().filter(|info| info.id.0 < 100_000) {
            info.id.0 += offset;
        }
        registries.push(registry);
        field_names.push(cg.export_type_field_names());
        layouts.push(cg.export_type_layouts());
    }
    let mut observed = Vec::new();
    for reverse in [false, true] {
        let ast = Parser::new(source).parse_module().unwrap();
        let order = if reverse { [1, 0] } else { [0, 1] };
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for i in order {
            cg.import_functions(&registries[i]);
            cg.import_type_field_names(&field_names[i]);
            cg.import_type_layouts(&layouts[i]);
        }
        cg.import_bootstrap_nominal_dependencies(
            &[&ast],
            &[&sources[order[0]], &sources[order[1]]],
        )
        .unwrap();
        cg.collect_unit_declarations(&[&ast]).unwrap();
        let module = cg.compile_function_bodies(&ast).unwrap();
        let probe = module
            .functions
            .iter()
            .find(|f| {
                module
                    .strings
                    .get(f.name)
                    .is_some_and(|n| n.ends_with("probe"))
            })
            .unwrap();
        let mut pc = probe.bytecode_offset as usize;
        let end = pc + probe.bytecode_length as usize;
        let mut methods = Vec::new();
        while pc < end {
            let instruction =
                verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).unwrap();
            match instruction {
                Instruction::Call { func_id, .. } | Instruction::CallG { func_id, .. } => methods
                    .push(
                        module
                            .band_reference_name(func_id)
                            .or_else(|| {
                                module
                                    .functions
                                    .iter()
                                    .find(|f| f.id.0 == func_id)
                                    .and_then(|f| module.strings.get(f.name))
                            })
                            .unwrap_or("<unknown>")
                            .to_owned(),
                    ),
                Instruction::CallM { method_id, .. } => methods.push(
                    module
                        .strings
                        .get(verum_vbc::types::StringId(method_id))
                        .unwrap_or("<unknown>")
                        .to_owned(),
                ),
                _ => {}
            }
        }
        observed.push(methods);
    }
    observed
}

#[test]
fn qualified_source_field_receivers_keep_method_owner_in_both_orders() {
    assert_eq!(
        body_targets(
            "module consumer; fn probe(a: &alpha.Envelope, b: &beta.Envelope) -> Int { a.value.identity() + b.value.identity() }"
        ),
        vec![vec!["alpha.Leaf.identity", "beta.Leaf.identity"]; 2]
    );
}

#[test]
fn checked_and_mutable_reference_parameters_preserve_the_same_owners() {
    assert_eq!(
        body_targets(
            "module consumer; fn probe(a: &checked alpha.Envelope, b: &mut beta.Envelope) -> Int { a.value.identity() + b.value.identity() }"
        ),
        vec![vec!["alpha.Leaf.identity", "beta.Leaf.identity"]; 2]
    );
}

#[test]
fn missing_qualified_parameter_never_borrows_an_ancestor_or_sibling() {
    assert_eq!(
        body_targets(
            "module consumer; fn probe(a: &alpha.child.Envelope) -> Int { a.value.identity() }"
        ),
        vec![vec!["identity"]; 2]
    );
}

#[test]
fn local_record_parameter_keeps_its_own_field_identity() {
    let targets = body_targets(
        "module consumer; type LocalLeaf is { n: Int }; type Envelope is { value: LocalLeaf }; implement LocalLeaf { fn identity(&self) -> Int { 99 } } fn probe(a: &Envelope) -> Int { a.value.identity() }",
    );
    assert!(
        targets
            .iter()
            .all(|names| names.len() == 1 && names[0].ends_with("LocalLeaf.identity")),
        "{targets:?}"
    );
}

#[test]
fn declared_generic_field_is_not_captured_by_an_imported_nominal() {
    assert_eq!(
        body_targets(
            "module consumer; type Envelope<T> is { value: T }; fn probe<T>(a: &Envelope<T>) -> Int { a.value.identity() }"
        ),
        vec![vec!["identity"]; 2]
    );
}
