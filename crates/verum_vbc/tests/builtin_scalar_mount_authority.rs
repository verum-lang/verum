//! Language scalars need no library declaration; an explicit mount still selects its owner.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    types::TypeId,
};

fn compile(mount: &str, body: &str) -> Result<VbcModule, CodegenError> {
    let scratch = tempfile::tempdir().unwrap();
    let core = scratch.path().join("core");
    for directory in ["base", "prelude", "provider"] {
        std::fs::create_dir_all(core.join(directory)).unwrap();
    }
    std::fs::write(
        core.join("mod.vr"),
        "module core; public module base; public module provider;",
    )
    .unwrap();
    std::fs::write(core.join("base/mod.vr"), "module core.base;").unwrap();
    std::fs::write(core.join("prelude/mod.vr"), "module core.prelude;").unwrap();
    std::fs::write(
        core.join("provider/mod.vr"),
        "module core.provider; public type Int64 is { value: Int };",
    )
    .unwrap();
    let source = format!("module core.consumer; {mount} {body}");
    let path = core.join("consumer.vr");
    std::fs::write(&path, &source).unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.consumer").with_validation())
        .compile_module_with_mounts(
            &Parser::new(&source).parse_module().unwrap(),
            path.to_str().unwrap(),
            core.to_str().unwrap(),
        )
}

fn check_wire_and_execution(module: VbcModule, expected: TypeId) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let holder = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Holder"))
            .unwrap();
        assert_eq!(
            holder.fields[0]
                .declaration_type
                .as_ref()
                .and_then(|ty| ty.base_type_id()),
            Some(expected)
        );
        let entry = module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some("core.consumer.main"))
            .unwrap()
            .id;
        let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(41));
    }
}

#[test]
fn fixed_width_scalar_is_available_without_a_library_mount() {
    let module = compile(
        "",
        "type Holder is { value: Int64 }; fn main() -> Int64 { 41 }",
    )
    .unwrap();
    check_wire_and_execution(module, TypeId::I64);
}

#[test]
fn absent_explicit_export_cannot_fall_back_to_a_builtin_or_foreign_nominal() {
    for mount in ["mount base.{Int64};", "mount core.base.Int64;"] {
        let error = compile(
            mount,
            "type Holder is { value: Int64 }; fn main() -> Int { 41 }",
        )
        .expect_err("the explicit owner does not declare Int64");
        assert!(
            error
                .to_string()
                .contains("unresolved nominal type 'Int64'"),
            "{error}"
        );
        assert!(error.to_string().contains("core.consumer"), "{error}");
    }
}

#[test]
fn explicit_same_leaf_nominal_keeps_its_actual_declaration_identity() {
    let module = compile(
        "mount core.provider.Int64;",
        r#"
        type Holder is { value: Int64 };
        fn main() -> Int { let value = Int64 { value: 41 }; value.value }
    "#,
    )
    .unwrap();
    let selected = module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Int64")
                && ty.origin_module.and_then(|owner| module.get_string(owner))
                    == Some("core.provider")
        })
        .unwrap()
        .id;
    assert_ne!(selected, TypeId::I64);
    check_wire_and_execution(module, selected);
}
