//! Materialized negative integers use the same literal/sign boundary as source.
use std::{fs, sync::Arc};
use verum_ast::{ExprKind, LiteralKind, Span, UnOp};
use verum_compiler::{
    api::compile_to_vbc,
    meta::{ConstValue, const_lowering::materialize_runtime_value},
    precompile::{PrecompileConfig, precompile_stdlib},
};
use verum_vbc::{VbcModule, archive::read_archive_from_file, interpreter::Interpreter};

fn check(module: VbcModule, entry: &str, expected: i64) {
    verum_vbc::validate::validate_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&decoded).unwrap();
    for module in [module, decoded] {
        let id = module.find_function_by_name(entry).unwrap();
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().unwrap();
        assert_eq!(
            vm.execute_function(id).unwrap().try_as_i64(),
            Some(expected)
        );
    }
}
fn source(expression: &str) -> String {
    format!(
        "fn selected(value:Int=@const {{ {expression} }})->Int{{value}} public fn main()->Int{{selected()}}"
    )
}
#[test]
fn ordinary_negative_const_defaults_execute() {
    for (expression, expected) in [("-42", -42), ("-9223372036854775807 - 1", i64::MIN)] {
        check(
            compile_to_vbc(&source(expression)).unwrap(),
            "main",
            expected,
        );
    }
}
#[test]
fn producer_negative_const_defaults_execute() {
    for (expression, expected) in [("-42", -42), ("-9223372036854775807 - 1", i64::MIN)] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("core");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("mod.vr"), source(expression)).unwrap();
        let config = PrecompileConfig {
            stdlib_path: root,
            output_path: temp.path().join("runtime.vbca"),
            target_triple: None,
            verbose: false,
        };
        let result = precompile_stdlib(&config).unwrap();
        assert!(result.errors.is_empty());
        check(
            read_archive_from_file(&config.output_path)
                .unwrap()
                .load_module("core")
                .unwrap(),
            "core.main",
            expected,
        );
    }
}
#[test]
fn materialized_negative_boundaries_keep_explicit_sign() {
    for value in [-1i128, i64::MIN as i128, i128::MIN] {
        let expression =
            materialize_runtime_value(ConstValue::Int(value), Span::default()).unwrap();
        let ExprKind::Unary {
            op: UnOp::Neg,
            expr,
        } = expression.kind
        else {
            panic!("negative value lost its explicit sign")
        };
        let ExprKind::Literal(literal) = expr.kind else {
            panic!()
        };
        let LiteralKind::Int(integer) = literal.kind else {
            panic!()
        };
        assert_eq!(integer.value as u128, value.unsigned_abs());
    }
}
#[test]
fn ordinary_negative_const_body_keeps_value() {
    check(
        compile_to_vbc("public fn main()->Int{@const{-42}}").unwrap(),
        "main",
        -42,
    );
}
