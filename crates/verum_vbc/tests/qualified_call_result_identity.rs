//! T1536: namespace calls retain the result of the selected declaration.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    instruction::Instruction,
    interpreter::Interpreter,
};

const ALPHA: &str = r#"module alpha {
    type Alpha is { label: Int };
    public fn select() -> Alpha { Alpha { label: 37 } }
}"#;
const BETA: &str = r#"module beta {
    type Beta is { other: Int, label: Int };
    public fn select() -> Beta { Beta { other: 90, label: 41 } }
}"#;

fn registered(source: &str) -> VbcCodegen {
    let ast = Parser::new(source)
        .parse_module()
        .expect("declaration syntax");
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    cg.collect_unit_declarations(&[&ast]).expect("declarations");
    cg
}

#[test]
fn exact_namespace_result_is_independent_of_sibling_registration_order() {
    for modules in [[ALPHA, BETA], [BETA, ALPHA]] {
        let cg = registered(&modules.join("\n"));
        for (call, expected) in [("alpha.select()", "Alpha"), ("beta.select()", "Beta")] {
            let expression = Parser::new(call).parse_expr().expect("call syntax");
            assert_eq!(
                cg.extract_expr_type_name(&expression).as_deref(),
                Some(expected),
                "extract {call}"
            );
            assert_eq!(
                cg.infer_expr_type_name(&expression).as_deref(),
                Some(expected),
                "infer {call}"
            );
        }
    }
}

#[test]
fn missing_namespace_cannot_inherit_a_unique_sibling_result() {
    let cg = registered(ALPHA);
    let expression = Parser::new("missing.select()").parse_expr().unwrap();
    assert_eq!(cg.extract_expr_type_name(&expression), None);
    assert_eq!(cg.infer_expr_type_name(&expression), None);
}

#[test]
fn qualified_calls_select_their_own_field_indices_and_values() {
    for modules in [[ALPHA, BETA], [BETA, ALPHA]] {
        let source = format!(
            "{}\n{}\nfn probe() -> Int {{ let a=alpha.select(); let b=beta.select(); a.label+b.label }}",
            modules[0], modules[1]
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::with_config(CodegenConfig::new("consumer"))
            .compile_module(&ast)
            .unwrap();
        let function = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("consumer.probe"))
            .unwrap();
        let fields: List<_> = function
            .instructions
            .as_ref()
            .unwrap()
            .iter()
            .filter_map(|i| match i {
                Instruction::GetF { field_idx, .. } => Some(*field_idx),
                _ => None,
            })
            .collect();
        assert_eq!(fields.as_slice(), [0, 1], "source-selected field layout");
        let id = function.id;
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(id)
                .unwrap()
                .as_i64(),
            78
        );
    }
}

#[test]
fn aliases_rooted_paths_and_arity_use_the_call_declaration() {
    let mut cg = registered(&format!("{ALPHA}\n{BETA}"));
    cg.ctx_mut()
        .module_aliases
        .insert("chosen".into(), vec!["alpha".into()]);
    cg.ctx_mut().current_source_module = Some("alpha.inner".into());
    for call in ["chosen.select()", "super.select()"] {
        let expression = Parser::new(call).parse_expr().unwrap();
        assert_eq!(
            cg.extract_expr_type_name(&expression).as_deref(),
            Some("Alpha")
        );
        assert_eq!(
            cg.infer_expr_type_name(&expression).as_deref(),
            Some("Alpha")
        );
    }
    let expression = Parser::new("alpha.select(0)").parse_expr().unwrap();
    assert_eq!(cg.extract_expr_type_name(&expression), None);
    assert_eq!(cg.infer_expr_type_name(&expression), None);
}

#[test]
fn namespace_result_pattern_retains_the_error_payload_layout() {
    let source = r#"
module alpha {
    type Alpha is { label: Int };
    public fn select() -> Result<Int, Alpha> { Err(Alpha { label: 37 }) }
}
module beta {
    type Beta is { other: Int, label: Int };
    public fn select() -> Result<Int, Beta> { Err(Beta { other: 90, label: 41 }) }
}
fn probe() -> Int {
    let result=alpha.select();
    match result { Ok(_) => 0, Err(info) => info.label }
}
"#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("consumer"))
        .compile_module(&ast)
        .unwrap();
    let f = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("consumer.probe"))
        .unwrap();
    let fields: List<_> = f
        .instructions
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|op| match op {
            Instruction::GetF { field_idx, .. } => Some(*field_idx),
            _ => None,
        })
        .collect();
    assert_eq!(fields.as_slice(), [0]);
    let id = f.id;
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap()
            .as_i64(),
        37
    );
}

/// Explicit archive gate: the ordinary unit suite has no baked stdlib artifact.
/// This reads the real descriptors; it does not compile/rebake an archive.
#[test]
#[ignore = "requires VERUM_TEST_ARCHIVE pointing to a coherent runtime.vbca"]
fn precompiled_catch_declarations_keep_exact_call_result_identity() {
    use verum_vbc::{
        archive::read_archive_from_file, codegen::context::FunctionInfo, module::FunctionId,
    };
    let archive =
        read_archive_from_file(&std::env::var("VERUM_TEST_ARCHIVE").expect("VERUM_TEST_ARCHIVE"))
            .unwrap();
    let declarations = [
        (
            "core.base",
            "core.base.panic.catch_unwind",
            "Result<T, PanicInfo>",
        ),
        (
            "core.intrinsics",
            "core.intrinsics.control.catch_unwind",
            "Result<T, IntrinsicPanicInfo>",
        ),
    ];
    for order in [[0, 1], [1, 0]] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for index in order {
            let (module_name, name, expected) = declarations[index];
            let module = archive.load_module(module_name).unwrap();
            let function = module
                .functions
                .iter()
                .find(|f| module.get_string(f.name) == Some(name))
                .expect("canonical public catch declaration");
            let carried = function
                .return_type_name
                .and_then(|sid| module.get_string(sid))
                .expect("source return spelling");
            assert_eq!(carried, expected);
            cg.ctx_mut().register_function(
                name.into(),
                FunctionInfo {
                    id: FunctionId(7000 + index as u32),
                    param_count: function.params.len(),
                    return_type: Some(function.return_type.clone()),
                    return_type_name: Some(carried.into()),
                    ..Default::default()
                },
            );
            let missing = Parser::new("foreign.catch_unwind(||0)")
                .parse_expr()
                .unwrap();
            assert_eq!(cg.extract_expr_type_name(&missing), None);
        }
        for (_, name, expected) in declarations {
            let expression = Parser::new(&format!("{name}(||0)")).parse_expr().unwrap();
            assert_eq!(
                cg.extract_expr_type_name(&expression).as_deref(),
                Some(expected)
            );
            assert_eq!(
                cg.infer_expr_type_name(&expression).as_deref(),
                Some(expected)
            );
        }
    }
}
