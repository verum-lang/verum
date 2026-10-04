//! T1536: intrinsic declarations must retain canonical sums before core.base is baked.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{FunctionDescriptor, VbcModule};
use verum_vbc::types::{TypeId, TypeParamId, TypeRef};

const SOURCE: &str = r#"
module core.intrinsics.control;
mount core.base.result.{Result};
mount core.base.maybe.{Maybe};
type Location is { file: Text, line: Int, column: Int };
type PanicInfo is { message: Text, location: Maybe<Location> };
@intrinsic("catch_unwind")
fn fence<T>(f: fn() -> T) -> Result<T, PanicInfo> { @intrinsic("catch_unwind", f) }
"#;

// The actual compile_core_module_from_ast caller sequence. In particular,
// core.base is not a previously compiled module and source mounts are not loaded.
fn bootstrap(source: &str) -> VbcModule {
    bootstrap_with_prior(source, &[])
}

fn bootstrap_with_prior(source: &str, available: &[&VbcModule]) -> VbcModule {
    let ast = Parser::new(source)
        .parse_module()
        .expect("parse bootstrap source");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.intrinsics"));
    // Bootstrap's global variant prepass supplies constructor facts even
    // before the producing module body exists. Build those facts from AST.
    let result = Parser::new("module core.base.result; type Result<T, E> is Ok(T) | Err(E);")
        .parse_module()
        .unwrap();
    let maybe = Parser::new("module core.base.maybe; type Maybe<T> is None | Some(T);")
        .parse_module()
        .unwrap();
    let mut declarations = VbcCodegen::with_config(CodegenConfig::new("core.base"));
    declarations
        .collect_unit_declarations(&[&result, &maybe])
        .unwrap();
    codegen.import_functions(&declarations.export_functions());
    codegen.register_stdlib_intrinsics();
    codegen.register_runtime_io_functions();
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], available)
        .unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_pending_default_methods().unwrap();
    codegen.compile_items_into_state(&ast).unwrap();
    codegen.finalize_module_from_state().unwrap()
}

fn function<'a>(module: &'a VbcModule, name: &str) -> &'a FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n.rsplit('.').next() == Some(name))
        })
        .expect("intrinsic function")
}

fn assert_live_contract(module: &VbcModule, name: &str) {
    let function = function(module, name);
    assert!(
        matches!(&function.return_type, TypeRef::Instantiated { base, args }
        if *base == TypeId::RESULT && args[0] == TypeRef::Generic(TypeParamId(0))),
        "bootstrap lost Result identity: {:?}",
        function.return_type
    );
    assert_eq!(
        function.type_params.len(),
        1,
        "panic stub loses its declaration roster"
    );
    assert_eq!(function.explicit_type_param_ids, [Some(TypeParamId(0))]);
    assert!(
        matches!(&function.params[0].type_ref, TypeRef::Function { params, return_type, .. }
        if params.is_empty() && **return_type == TypeRef::Generic(TypeParamId(0)))
    );
    let mut pc = function.bytecode_offset as usize;
    let end = pc + function.bytecode_length as usize;
    let mut has_try = false;
    let mut result_tags = Vec::new();
    while pc < end {
        let op = verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).unwrap();
        has_try |= matches!(op, verum_vbc::instruction::Instruction::TryBegin { .. });
        if let verum_vbc::instruction::Instruction::MakeVariantTyped { type_id, tag, .. } = op {
            if type_id == TypeId::RESULT.0 {
                result_tags.push(tag);
            }
        }
        assert!(
            !matches!(op, verum_vbc::instruction::Instruction::Panic { .. }),
            "panic stub: {op:?}"
        );
    }
    assert!(
        has_try,
        "the raw intrinsic body must compile into a panic boundary"
    );
    assert_eq!(
        result_tags,
        [0, 1],
        "canonical Result constructors must not use a collided bare variant"
    );
}

#[test]
fn bootstrap_mounts_resolve_reserved_sums_before_their_source_module() {
    assert_live_contract(&bootstrap(SOURCE), "fence");
}

#[test]
fn bootstrap_qualified_and_aliased_canonical_sum_names_resolve() {
    let qualified = SOURCE
        .replace(
            "Result<T, PanicInfo>",
            "core.base.result.Result<T, PanicInfo>",
        )
        .replace("Maybe<Location>", "core.base.maybe.Maybe<Location>");
    assert_live_contract(&bootstrap(&qualified), "fence");
    let aliased = SOURCE
        .replace(
            "mount core.base.result.{Result};",
            "mount core.base.result.Result as Outcome;",
        )
        .replace("Result<T, PanicInfo>", "Outcome<T, PanicInfo>");
    assert_live_contract(&bootstrap(&aliased), "fence");
}

#[test]
fn actual_control_source_uses_the_bootstrap_contract() {
    assert_live_contract(
        &bootstrap(include_str!("../../../core/intrinsics/control.vr")),
        "catch_unwind",
    );
}

#[test]
fn foreign_sum_mounts_do_not_fall_back_to_builtin_leaf_names() {
    for (owner, diagnostic) in [
        ("result", "declaration must return"),
        ("maybe", "error descriptor must declare"),
    ] {
        let source = SOURCE.replace(
            &format!("mount core.base.{owner}"),
            &format!("mount foreign.base.{owner}"),
        );
        let module = bootstrap(&source);
        let function = function(&module, "fence");
        let mut pc = function.bytecode_offset as usize;
        let op = verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).unwrap();
        let verum_vbc::instruction::Instruction::Panic { message_id } = op else {
            panic!("foreign {owner} must fail the intrinsic contract, got {op:?}");
        };
        let message = module
            .get_string(verum_vbc::types::StringId(message_id))
            .unwrap();
        assert!(message.contains(diagnostic), "{owner}: {message}");
    }
}

#[test]
fn a_local_sum_declaration_keeps_its_own_identity() {
    let source = SOURCE.replace(
        "type Location",
        "type Result<T, E> is Success(T) | Failure(E); type Location",
    );
    let module = bootstrap(&source);
    assert!(matches!(&function(&module, "fence").return_type,
        TypeRef::Instantiated { base, .. } if *base != TypeId::RESULT));
}

fn sum_module(owner: &str, declaration: &str) -> VbcModule {
    let ast = Parser::new(&format!("module {owner}; {declaration}"))
        .parse_module()
        .unwrap();
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .unwrap()
}

#[test]
fn bootstrap_intrinsic_uses_qualified_constructors_after_bare_variant_collision() {
    let source = SOURCE.replace(
        "type Location",
        "type Other is Err(Int) | Ok(Int); type Location",
    );
    assert_live_contract(&bootstrap(&source), "fence");
}

#[test]
fn imported_canonical_sum_descriptor_owns_intrinsic_variants() {
    let result = sum_module("core.base.result", "type Result<T, E> is Ok(T) | Err(E);");
    let maybe = sum_module("core.base.maybe", "type Maybe<T> is None | Some(T);");
    let source = SOURCE.replace(
        "type Location",
        "type Other is Err(Int) | Ok(Int); type Location",
    );
    assert_live_contract(&bootstrap_with_prior(&source, &[&result, &maybe]), "fence");
}

#[test]
fn imported_foreign_result_cannot_claim_the_reserved_sum_id() {
    let foreign = sum_module(
        "foreign.base.result",
        "type Result<T, E> is Ok(T) | Err(E);",
    );
    let source = SOURCE.replace("mount core.base.result", "mount foreign.base.result");
    let module = bootstrap_with_prior(&source, &[&foreign]);
    assert!(matches!(&function(&module, "fence").return_type,
        TypeRef::Instantiated { base, .. } if *base != TypeId::RESULT));
}
