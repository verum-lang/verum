//! Type positions and value constructors retain independent declaration authority.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
    types::{TypeId, TypeRef},
};

fn compile(provider: &str, imports: &str, body: &str) -> Result<VbcModule, CodegenError> {
    let scratch = tempfile::tempdir().unwrap();
    let core = scratch.path().join("core");
    std::fs::create_dir_all(core.join("prelude")).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module provider;").unwrap();
    std::fs::write(core.join("prelude/mod.vr"), "module core.prelude;").unwrap();
    std::fs::write(
        core.join("provider.vr"),
        format!("module core.provider; {provider}"),
    )
    .unwrap();
    let source = format!("module core.consumer; {imports} {body}");
    let path = core.join("consumer.vr");
    std::fs::write(&path, &source).unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.consumer").with_validation())
        .compile_module_with_mounts(
            &Parser::new(&source).parse_module().unwrap(),
            path.to_str().unwrap(),
            core.to_str().unwrap(),
        )
}

fn compile_complete_unit(provider: &str, imports: &str, body: &str) -> VbcModule {
    // Mounted source discovery provides signatures. Runtime verification must
    // compile the provider's actual bodies as part of the executable unit.
    compile(provider, imports, body).unwrap();
    compile_owned_unit(provider, imports, body)
}

fn compile_owned_unit(provider: &str, imports: &str, body: &str) -> VbcModule {
    let provider = Parser::new(&format!("module core.provider; {provider}"))
        .parse_module()
        .unwrap();
    let consumer = Parser::new(&format!("module core.consumer; {imports} {body}"))
        .parse_module()
        .unwrap();
    let mut codegen =
        VbcCodegen::with_config(CodegenConfig::new("core.consumer").with_validation());
    let files = [&consumer, &provider];
    codegen.collect_unit_declarations(&files).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_default_methods_with_policy(verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    codegen
        .compile_unit_items(&files, verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    codegen.finalize_module_from_state().unwrap()
}

fn assert_scalar_and_run(module: VbcModule, expected: i64) {
    let replay = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, replay] {
        let main = module.find_function_by_name("core.consumer.main").unwrap();
        let function = module
            .functions
            .iter()
            .find(|function| function.id == main)
            .unwrap();
        assert_eq!(
            function.semantic_return,
            Some(TypeRef::Concrete(TypeId::I64))
        );
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(main)
                .unwrap()
                .try_as_i64(),
            Some(expected)
        );
    }
}

#[test]
fn imported_variant_does_not_replace_a_scalar_in_type_position() {
    let module = compile(
        "public type Opcode is Int64(Int) | Stop;",
        "mount core.provider.{Opcode, Int64};",
        "fn main() -> Int64 { let value: Opcode = Int64(41); match value { Int64(n) => n, Stop => 0 } }",
    ).unwrap();
    assert_scalar_and_run(module, 41);
}

#[test]
fn imported_function_does_not_replace_a_scalar_in_type_position() {
    let module = compile_complete_unit(
        "public fn Int64() -> Int { 41 }",
        "mount core.provider.Int64;",
        "fn main() -> Int64 { Int64() }",
    );
    assert_scalar_and_run(module, 41);
}

#[test]
fn an_unresolved_import_still_refuses_instead_of_assuming_a_value_binding() {
    let error = compile(
        "",
        "mount core.provider.Int64;",
        "fn main() -> Int64 { 41 }",
    )
    .expect_err("unknown export has no established namespace category");
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'Int64'"),
        "{error}"
    );
}

#[test]
fn a_real_imported_type_still_selects_its_nominal_identity() {
    let module = compile(
        "public type Int64 is { public value: Int };",
        "mount core.provider.Int64;",
        "type Holder is { value: Int64 }; fn main() -> Int { let value = Int64 { value: 41 }; value.value }",
    ).unwrap();
    let selected = module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Int64")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some("core.provider")
        })
        .unwrap();
    let holder = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Holder"))
        .unwrap();
    assert_ne!(selected.id, TypeId::I64);
    assert_eq!(
        holder.fields[0].declaration_type,
        Some(TypeRef::Concrete(selected.id))
    );
}

#[test]
fn named_value_and_type_imports_share_a_spelling_without_order_dependence() {
    for imports in [
        "mount core.provider.{produce as Choice, Record as Choice};",
        "mount core.provider.{Record as Choice, produce as Choice};",
    ] {
        let module = compile_complete_unit(
            "public fn produce() -> Int { 1 } public type Record is { public value: Int };",
            imports,
            "fn main() -> Int { let record: Choice = Choice { value: 40 }; Choice() + record.value }",
        );
        assert_scalar_and_run(module, 41);
    }
}

#[test]
fn value_reexports_keep_their_namespace_before_callable_registration() {
    let module = compile(
        "public module inner { public type Opcode is Int64(Int) | Stop; } public mount inner.{Opcode, Int64, Stop};",
        "mount core.provider.{Opcode, Int64, Stop};",
        "fn main() -> Int64 { let value: Opcode = Int64(41); match value { Int64(n) => n, Stop => 0 } }",
    ).unwrap();
    assert_scalar_and_run(module, 41);
}

#[test]
fn value_globs_do_not_hide_types_from_another_glob() {
    for imports in [
        "mount core.provider.values.*; mount core.provider.types.*;",
        "mount core.provider.types.*; mount core.provider.values.*;",
    ] {
        let module = compile_complete_unit(
            "public module values { public fn Choice() -> Int { 1 } } public module types { public type Choice is { public value: Int }; }",
            imports,
            "fn main() -> Int { let record: Choice = Choice { value: 40 }; Choice() + record.value }",
        );
        assert_scalar_and_run(module, 41);
    }
}

#[test]
fn unresolved_import_beside_a_value_is_not_discarded() {
    let error = compile(
        "public fn produce() -> Int { 41 }",
        "mount core.provider.{produce as Int64, absent as Int64};",
        "fn main() -> Int64 { 41 }",
    )
    .expect_err("unknown category cannot be assumed to belong to values");
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'Int64'"),
        "{error}"
    );
}

#[test]
fn two_imported_nominals_remain_ambiguous_even_with_a_value() {
    let error = compile(
        "public type First is { public value: Int }; public type Second is { public value: Int }; public fn produce() -> Int { 41 }",
        "mount core.provider.{First as Choice, produce as Choice, Second as Choice};",
        "fn main(value: Choice) -> Int { value.value }",
    ).expect_err("different type declarations cannot collapse through a value import");
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'Choice'"),
        "{error}"
    );
}

#[test]
fn a_public_value_does_not_publish_a_same_named_private_type() {
    let error = compile(
        "type Int64 is { public value: Int }; public fn Int64() -> Int { 41 }",
        "mount core.provider.Int64;",
        "fn main() -> Int64 { 41 }",
    )
    .expect_err("a public function is not visibility evidence for the nominal declaration");
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'Int64'"),
        "{error}"
    );
}

#[test]
fn wire_scope_keeps_type_imports_and_does_not_reclassify_values_as_types() {
    let module = compile_complete_unit(
        "public fn produce() -> Int { 1 } public type Record is { public value: Int };",
        "mount core.provider.{produce as Int64, produce as Choice, Record as Choice};",
        "public type Carrier<T> is { public witness: Type, public payload: T }; fn main() -> Int64 { let record: Choice = Choice { value: 40 }; Int64() + record.value }",
    );
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let (_, scope) = module
            .declaration_type_bindings
            .iter()
            .find(|(owner, _)| owner.as_str() == "core.consumer")
            .unwrap();
        assert!(scope.iter().all(|(name, _)| name.as_str() != "Int64"));
        assert_eq!(
            scope
                .iter()
                .find(|(name, _)| name.as_str() == "Choice")
                .and_then(|(_, target)| target.as_ref())
                .map(|target| target.as_str()),
            Some("core.provider.Record")
        );
        let main = module.find_function_by_name("core.consumer.main").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(main)
                .unwrap()
                .try_as_i64(),
            Some(41)
        );
    }
}

#[test]
fn namespace_audit_local_value_does_not_hide_a_reexported_type() {
    let module = compile_owned_unit(
        "public module definitions { public type Choice is { public value: Int }; } public module facade { public fn Choice() -> Int { 1 } public mount super.definitions.Choice; }",
        "mount core.provider.facade.Choice;",
        "fn main() -> Int { let value: Choice = Choice { value: 40 }; Choice() + value.value }",
    );
    assert_scalar_and_run(module, 41);
}

#[test]
fn namespace_audit_local_value_does_not_hide_a_glob_reexported_type() {
    let module = compile_owned_unit(
        "public module definitions { public type Choice is { public value: Int }; } public module facade { public fn Choice() -> Int { 1 } public mount super.definitions.*; }",
        "mount core.provider.facade.Choice;",
        "fn main() -> Int { let value: Choice = Choice { value: 40 }; Choice() + value.value }",
    );
    assert_scalar_and_run(module, 41);
}

#[test]
fn namespace_audit_public_type_does_not_publish_a_private_callable() {
    let error = compile(
        "public type Choice is { public value: Int }; fn Choice() -> Int { 41 }",
        "mount core.provider.Choice;",
        "fn main() -> Int { Choice() }",
    )
    .expect_err("a public type must not grant visibility to a same-named function");
    assert!(
        error.to_string().contains("inaccessible") || error.to_string().contains("private"),
        "{error}"
    );
}
