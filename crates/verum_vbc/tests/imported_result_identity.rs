//! Imported results must come from their selected source declaration.
#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    module::VbcModule,
};

fn boundary(provider: &str, consumer: &str) -> (VbcCodegen, VbcModule) {
    let ast = Parser::new(provider).parse_module().unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("alpha"));
    let module = producer.compile_module(&ast).unwrap();
    let ast = Parser::new(consumer).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let mut unrelated = VbcCodegen::with_config(CodegenConfig::new("unrelated"));
    unrelated
        .compile_module(
            &Parser::new("module unrelated; fn registry() -> Int { 99 }")
                .parse_module()
                .unwrap(),
        )
        .unwrap();
    let mut functions = unrelated.export_functions();
    functions.retain(|name, _| name == "registry" || name == "unrelated.registry");
    assert!(functions.contains_key("registry"));
    for info in functions.values_mut() {
        info.id.0 += 20_000;
    }
    codegen.import_functions(&functions);
    codegen.import_functions(&producer.export_functions());
    codegen.import_type_layouts(&producer.export_type_layouts());
    codegen.import_type_field_names(&producer.export_type_field_names());
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&module])
        .unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let result = codegen.compile_function_bodies(&ast).unwrap();
    codegen.ctx_mut().current_source_module = Some("consumer".into());
    (codegen, result)
}

fn field_indices(module: &VbcModule) -> List<u32> {
    use verum_vbc::{bytecode::decode_instruction, instruction::Instruction};
    let function = module
        .functions
        .iter()
        .find(|function| module.strings.get(function.name) == Some("consumer.probe"))
        .unwrap();
    let mut pc = function.bytecode_offset as usize;
    let end = pc + function.bytecode_length as usize;
    let mut indices = List::new();
    while pc < end {
        let instruction = decode_instruction(&module.bytecode, &mut pc).unwrap();
        assert!(!matches!(instruction, Instruction::GetFieldNamed { .. }));
        if let Instruction::GetF { field_idx, .. } = instruction {
            indices.push(field_idx);
        }
    }
    indices
}

#[test]
fn renamed_import_keeps_the_method_result_owner() {
    let (mut codegen, module) = boundary(
        r#"
module alpha.clock;
public type Calendar is { year: Int, month: Int, day: Int };
public type DatetimeInstant is { serial: Int };
implement DatetimeInstant {
    public fn ymd_view(&self) -> Calendar { Calendar { year: 2026, month: 11, day: 1 } }
}
"#,
        r#"
module consumer;
mount alpha.clock.{DatetimeInstant as Instant};
type Unrelated is { a: Int, b: Int, c: Int, day: Int, other: Int };
fn probe(inst: &Instant) -> Int { let ymd = inst.ymd_view(); ymd.day }
"#,
    );
    assert_eq!(field_indices(&module).as_slice(), &[2]);
    codegen
        .ctx_mut()
        .variable_type_names
        .insert("inst".into(), "Instant".into());
    let expression = Parser::new("inst.ymd_view()").parse_expr().unwrap();
    let result = codegen
        .extract_expr_type_name(&expression)
        .expect("mounted method result");
    assert!(
        result == "Calendar" || result == "alpha.clock.Calendar",
        "{result}"
    );
}

#[test]
fn module_import_keeps_the_free_function_result_owner() {
    let (codegen, module) = boundary(
        r#"
module alpha.registry;
public type Arity is { min: Int, max: Int };
public fn arity_of(value: Int) -> Arity { Arity { min: value, max: value + 1 } }
"#,
        r#"
module consumer;
mount alpha.registry;
type Unrelated is { a: Int, min: Int, b: Int, c: Int, max: Int };
fn probe() -> Int { let arity = registry.arity_of(2); arity.max }
"#,
    );
    assert_eq!(field_indices(&module).as_slice(), &[1]);
    for path in ["registry", "alpha.registry"] {
        let expression = Parser::new(&format!("{path}.arity_of(2)"))
            .parse_expr()
            .unwrap();
        let result = codegen
            .extract_expr_type_name(&expression)
            .expect("mounted free result");
        assert!(
            result == "Arity" || result == "alpha.registry.Arity",
            "{result}"
        );
        assert_eq!(
            codegen.infer_expr_type_name(&expression),
            Some(result),
            "{path}"
        );
    }
    for source in ["absent.arity_of(2)", "registry.absent(2)"] {
        let expression = Parser::new(source).parse_expr().unwrap();
        assert_eq!(
            codegen.extract_expr_type_name(&expression),
            None,
            "{source}"
        );
        assert_eq!(codegen.infer_expr_type_name(&expression), None, "{source}");
    }
}
