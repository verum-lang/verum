#![cfg(feature = "codegen")]
//! Invalid static initializers cannot become successful nil constructors.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, CodegenErrorKind, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str, bootstrap: bool) -> Result<VbcModule, CodegenError> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("static_errors").with_validation());
    if bootstrap {
        cg.collect_unit_declarations(&[&ast])?;
        cg.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        cg.finalize_module_from_state()
    } else {
        cg.compile_module(&ast)
    }
}

fn invalid_initializer(declaration: &str) {
    let source = format!("type Cell is {{ value: Int }}; {declaration} fn probe() -> Int {{ 31 }}");
    for bootstrap in [false, true] {
        let error = match compile(&source, bootstrap) {
            Err(error) => error,
            Ok(_) => panic!("static initialization must fail"),
        };
        assert!(
            matches!(&error.kind, CodegenErrorKind::BreakOutsideLoop),
            "original declaration error must survive: {error}"
        );
        assert!(
            error.span.is_some(),
            "source position must survive: {error}"
        );
        assert!(
            error
                .context
                .as_deref()
                .is_some_and(|name| name.contains("__tls_init_") && name.contains("BROKEN")),
            "initializer identity must be present: {error}"
        );
    }
}

#[test]
fn immutable_record_static_rejects_invalid_initializer() {
    invalid_initializer("static BROKEN: Cell = { break; Cell { value: 7 } };");
}

#[test]
fn mutable_static_rejects_invalid_initializer() {
    invalid_initializer("static mut BROKEN: Cell = { break; Cell { value: 7 } };");
}

#[test]
fn intentional_thread_local_static_rejects_invalid_initializer() {
    invalid_initializer("@thread_local static BROKEN: Cell = { break; Cell { value: 7 } };");
}

#[test]
fn repeated_finalization_cannot_forget_a_failed_initializer() {
    let source = r#"
        type Cell is { value: Int };
        static BEFORE: Cell = Cell { value: 11 };
        static BROKEN: Cell = { break; Cell { value: 7 } };
        static AFTER: Cell = Cell { value: 19 };
        fn probe() -> Int { BEFORE.value + AFTER.value }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("static_errors").with_validation());
    cg.collect_unit_declarations(&[&ast]).unwrap();
    let first = cg
        .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
        .unwrap_err();
    assert!(matches!(first.kind, CodegenErrorKind::BreakOutsideLoop));
    for _ in 0..2 {
        let error = match cg.finalize_module_from_state() {
            Err(error) => error,
            Ok(_) => panic!("finalization forgot the failed initializer"),
        };
        assert!(matches!(error.kind, CodegenErrorKind::BreakOutsideLoop));
        assert_eq!(error.span, first.span);
        assert_eq!(error.context, first.context);
    }
}

#[test]
fn valid_static_initializers_keep_their_declared_values_through_wire() {
    for declaration in ["static", "static mut", "@thread_local static"] {
        let source = format!(
            "type Cell is {{ value: Int }}; {declaration} GOOD: Cell = Cell {{ value: 31 }}; fn probe() -> Int {{ GOOD.value }}"
        );
        for bootstrap in [false, true] {
            let module = compile(&source, bootstrap).unwrap();
            let wire = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap();
            for module in [module, wire] {
                let probe = module.find_function_by_name("static_errors.probe").unwrap();
                let mut vm = Interpreter::new(Arc::new(module));
                vm.run_global_ctors().unwrap();
                assert_eq!(vm.execute_function(probe).unwrap().try_as_i64(), Some(31));
            }
        }
    }
}
