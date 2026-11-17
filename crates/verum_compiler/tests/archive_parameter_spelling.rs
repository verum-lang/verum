//! T1543: absent parameter spellings must not read the module-name string slot.

use verum_common::Text;
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::core_metadata::ParamDescriptor;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

// T1543: optional spelling IDs must survive the archive boundary as optional.
fn parameter_metadata(
    source_type: &str,
    declared: Option<&str>,
    keep_semantic: bool,
) -> ParamDescriptor {
    use verum_vbc::types::StringId;

    let source = format!(
        "module fixture.producer; public type NominalCallback is {{value:Int}}; public fn accept(value:{source_type})->() {{}}"
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut checker = verum_types::TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("fixture.producer");
    assert!(checker.prepare_module_declarations(&ast).is_empty());
    for item in &ast.items {
        checker.check_item(item).expect("actual source checks");
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostic_sources().is_empty());
    let mut module = VbcCodegen::with_config(CodegenConfig::new("fixture.producer"))
        .compile_module(&ast)
        .unwrap();
    assert_eq!(module.strings.get(StringId::EMPTY), Some(""));
    let spelling = declared
        .map(|name| module.intern_string(name))
        .unwrap_or(StringId::EMPTY);
    let function = module
        .functions
        .iter_mut()
        .find(|function| {
            function
                .source_signature
                .as_ref()
                .is_some_and(|source| source.name.name == "accept")
        })
        .unwrap();
    assert!(function.declaration_identity.is_some());
    let source_signature = function.source_signature.clone();
    // The legacy display cases omit the optional semantic slot as well as
    // changing the optional spelling. The complete actual source header and
    // identity remain authoritative, and the emitted ABI is never fabricated.
    // Full source emission gives raw pointers the actual PTR ABI used here.
    function.params[0].type_name = spelling;
    if !keep_semantic {
        function.semantic_params = None;
    }
    let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
    archive
        .add_module("fixture.producer", &module, &[])
        .expect("archive fixture");
    let metadata = archive_to_core_metadata(&archive.finish()).expect("valid archive metadata");
    let declaration = metadata
        .functions
        .get(&Text::from("fixture.producer.accept"))
        .expect("converted function");
    assert_eq!(declaration.source_signature, source_signature);
    declaration.params[0].clone()
}

#[test]
fn absent_parameter_spelling_preserves_opaque_callable_structure() {
    let param = parameter_metadata("fn()->*const Int", None, false);
    assert_eq!(param.ty.as_str(), "fn() -> __opaque_type_14");
    assert!(
        param.declared_ty.is_empty(),
        "slot zero is an absent spelling, not a type"
    );
}

#[test]
fn absent_parameter_spelling_does_not_invent_a_nominal_declaration() {
    let param = parameter_metadata("Int", None, false);
    assert_eq!(param.ty.as_str(), "Int");
    assert!(param.declared_ty.is_empty());
}

#[test]
fn explicit_parameter_spelling_still_repairs_opaque_types() {
    let nominal = parameter_metadata("*const Int", Some("other.NominalCallback"), false);
    assert_eq!(nominal.ty.as_str(), "other.NominalCallback");
    assert_eq!(nominal.declared_ty.as_str(), "other.NominalCallback");
    let callable = parameter_metadata("fn()->*const Int", Some("fn() -> other.Token"), false);
    assert_eq!(callable.ty.as_str(), "fn() -> other.Token");
    assert_eq!(callable.declared_ty.as_str(), "fn() -> other.Token");
}

#[test]
fn real_nominal_parameter_remains_nominal_without_a_carried_spelling() {
    let param = parameter_metadata("NominalCallback", None, true);
    assert_eq!(param.ty.as_str(), "fixture.producer.NominalCallback");
    assert!(param.declared_ty.is_empty());
}

#[test]
fn retained_semantic_parameters_take_precedence_over_display_spellings() {
    let parameter = parameter_metadata("*const Int", Some("other.NominalCallback"), true);
    assert_eq!(parameter.ty.as_str(), "&unsafe Int");
    assert_eq!(parameter.declared_ty.as_str(), "other.NominalCallback");
    let callback = parameter_metadata("fn()->*const Int", Some("fn() -> other.Token"), true);
    assert_eq!(callback.ty.as_str(), "fn() -> &unsafe Int");
    assert_eq!(callback.declared_ty.as_str(), "fn() -> other.Token");
}
