//! Scope restoration for declaration passes, including nested Result errors.
use super::*;

#[test]
fn owned_source_scope_restores_both_fields_after_nested_error() {
    let outer = verum_fast_parser::Parser::new("module fixture.outer;")
        .parse_module()
        .unwrap();
    let inner = verum_fast_parser::Parser::new("module fixture.inner;")
        .parse_module()
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("original.parent"));
    codegen.ctx.current_source_module = Some("original.source".into());
    let result: CodegenResult<()> =
        codegen.with_owned_source_scope("outer.parent", &outer, |codegen| {
            assert_eq!(codegen.config.module_name, "outer.parent");
            assert_eq!(
                codegen.ctx.current_source_module.as_deref(),
                Some("fixture.outer")
            );
            let result: CodegenResult<()> =
                codegen.with_owned_source_scope("inner.parent", &inner, |codegen| {
                    assert_eq!(codegen.config.module_name, "inner.parent");
                    assert_eq!(
                        codegen.ctx.current_source_module.as_deref(),
                        Some("fixture.inner")
                    );
                    Err(CodegenError::internal("pass failed"))
                });
            assert_eq!(codegen.config.module_name, "outer.parent");
            assert_eq!(
                codegen.ctx.current_source_module.as_deref(),
                Some("fixture.outer")
            );
            result
        });
    assert!(result.is_err());
    assert_eq!(codegen.config.module_name, "original.parent");
    assert_eq!(
        codegen.ctx.current_source_module.as_deref(),
        Some("original.source")
    );
}
