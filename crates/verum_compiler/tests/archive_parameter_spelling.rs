//! T1543: absent parameter spellings must not read the module-name string slot.

use verum_common::Text;
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_types::core_metadata::ParamDescriptor;
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeKind, TypeRef};

// T1543: optional spelling IDs must survive the archive boundary as optional.
fn parameter_metadata(type_ref: TypeRef, declared: Option<&str>) -> ParamDescriptor {
    use verum_vbc::module::{FunctionDescriptor, ParamDescriptor as VbcParam};
    use verum_vbc::types::{StringId, TypeDescriptor, TypeId};

    let mut module = VbcModule::new("fixture.producer".to_string());
    assert_eq!(module.strings.get(StringId::EMPTY), Some("fixture.producer"));
    let nominal_name = module.intern_string("NominalCallback");
    module.add_type(TypeDescriptor {
        id: TypeId(2000),
        name: nominal_name,
        kind: TypeKind::Record,
        ..Default::default()
    });
    let mut function = FunctionDescriptor::new(module.intern_string("accept"));
    function.params.push(VbcParam {
        name: module.intern_string("value"),
        type_ref,
        type_name: declared
            .map(|name| module.intern_string(name))
            .unwrap_or(StringId::EMPTY),
        ..Default::default()
    });
    module.add_function(function);
    let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
    archive
        .add_module("fixture.producer", &module, &[])
        .expect("archive fixture");
    let metadata = archive_to_core_metadata(&archive.finish());
    metadata
        .functions
        .get(&Text::from("fixture.producer.accept"))
        .expect("converted function")
        .params[0]
        .clone()
}

fn opaque_callback() -> TypeRef {
    TypeRef::Function {
        params: Vec::new(),
        return_type: Box::new(TypeRef::Concrete(verum_vbc::types::TypeId::PTR)),
        contexts: Default::default(),
    }
}

#[test]
fn absent_parameter_spelling_preserves_opaque_callable_structure() {
    let param = parameter_metadata(opaque_callback(), None);
    assert_eq!(param.ty.as_str(), "fn() -> __opaque_type_14");
    assert!(
        param.declared_ty.is_empty(),
        "slot zero is an absent spelling, not a type"
    );
}

#[test]
fn absent_parameter_spelling_does_not_invent_a_nominal_declaration() {
    let param = parameter_metadata(TypeRef::Concrete(verum_vbc::types::TypeId::INT), None);
    assert_eq!(param.ty.as_str(), "Int");
    assert!(param.declared_ty.is_empty());
}

#[test]
fn explicit_parameter_spelling_still_repairs_opaque_types() {
    let nominal = parameter_metadata(
        TypeRef::Concrete(verum_vbc::types::TypeId::PTR),
        Some("other.NominalCallback"),
    );
    assert_eq!(nominal.ty.as_str(), "other.NominalCallback");
    assert_eq!(nominal.declared_ty.as_str(), "other.NominalCallback");
    let callable = parameter_metadata(opaque_callback(), Some("fn() -> other.Token"));
    assert_eq!(callable.ty.as_str(), "fn() -> other.Token");
    assert_eq!(callable.declared_ty.as_str(), "fn() -> other.Token");
}

#[test]
fn real_nominal_parameter_remains_nominal_without_a_carried_spelling() {
    let param = parameter_metadata(TypeRef::Concrete(verum_vbc::types::TypeId(2000)), None);
    assert_eq!(param.ty.as_str(), "NominalCallback");
    assert!(param.declared_ty.is_empty());
}
