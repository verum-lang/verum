#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, TypeDeclBody};
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::VbcCodegen,
    types::{ProtocolId, TypeParamId, TypeRef},
};

fn declaration<'a>(module: &'a VbcModule, name: &str) -> &'a verum_vbc::TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some(name))
        .unwrap()
}

fn check_roster(module: &VbcModule, name: &str, field_count: usize) {
    let owner = declaration(module, name);
    let marker = declaration(module, "Marker").id;
    let value = declaration(module, "Value").id;
    assert_eq!(owner.type_params.len(), 2, "{name}");
    assert_eq!(module.get_string(owner.type_params[0].name), Some("T"));
    assert_eq!(module.get_string(owner.type_params[1].name), Some("N"));
    assert_eq!(owner.type_params[0].id, TypeParamId(0));
    assert_eq!(owner.type_params[1].id, TypeParamId(1));
    assert_eq!(
        owner.type_params[0].bounds.as_slice(),
        &[ProtocolId(marker.0)]
    );
    assert_eq!(owner.type_params[0].default, Some(TypeRef::Concrete(value)));
    assert_eq!(owner.fields.len(), field_count);
    for field in &owner.fields {
        assert_eq!(
            field.declaration_type,
            Some(TypeRef::Generic(TypeParamId(0)))
        );
    }
}

fn check(source: &str, name: &str, fields: usize, legacy_newtype: bool) {
    let mut ast = Parser::new(source).parse_module().unwrap();
    if legacy_newtype {
        // The parser uses Tuple for the surface newtype spelling. The shared
        // compiler also accepts the explicit Newtype AST used by producers.
        let declaration = ast
            .items
            .iter_mut()
            .find_map(|item| match &mut item.kind {
                ItemKind::Type(ty) if ty.name.name.as_str() == name => Some(ty),
                _ => None,
            })
            .unwrap();
        let TypeDeclBody::Tuple(fields) = &declaration.body else {
            panic!("tuple source")
        };
        declaration.body = TypeDeclBody::Newtype(fields[0].clone());
    }
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        check_roster(&module, name, fields);
    }
}

const PREFIX: &str =
    "type Marker is protocol {}; type Value is { value: Int }; implement Marker for Value {}";

#[test]
fn transparent_tuple_keeps_dense_slots_bounds_and_default() {
    check(
        &format!("{PREFIX} type Wrap<'a, T: Marker = Value, N: meta Int> is (T);"),
        "Wrap",
        1,
        false,
    );
}

#[test]
fn tuple_keeps_dense_slots_bounds_and_default() {
    check(
        &format!("{PREFIX} type Pair<'a, T: Marker = Value, const N: Int> is (T, T);"),
        "Pair",
        2,
        false,
    );
}

#[test]
fn phantom_unit_keeps_its_actual_generic_contract() {
    check(
        &format!("{PREFIX} type Token<'a, T: Marker = Value, N: meta Int> is ();"),
        "Token",
        0,
        false,
    );
}

#[test]
fn explicit_newtype_ast_uses_the_same_declared_roster() {
    check(
        &format!("{PREFIX} type Wrap<'a, T: Marker = Value, N: meta Int> is (T);"),
        "Wrap",
        1,
        true,
    );
}

#[test]
fn tuple_and_unit_do_not_silently_erase_unresolved_bounds() {
    for body in ["(T)", "(T, T)", "()"] {
        let ast = Parser::new(&format!("type Invalid<T: Missing> is {body};"))
            .parse_module()
            .unwrap();
        let error = VbcCodegen::new()
            .compile_module(&ast)
            .expect_err("unknown bound is not an empty contract");
        assert!(
            error
                .to_string()
                .contains("unresolved protocol bound 'Missing'"),
            "{error}"
        );
    }
}

#[test]
fn transparent_generic_constructor_preserves_its_value_through_wire() {
    let ast = Parser::new(
        "type Wrap<T> is (T); fn main()->Int { let item: Wrap<Int> = Wrap(37); item.0 }",
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        assert_eq!(declaration(&module, "Wrap").type_params.len(), 1);
        let entry = module
            .functions
            .iter()
            .find(|function| {
                module
                    .get_string(function.name)
                    .is_some_and(|name| name == "main" || name.ends_with(".main"))
            })
            .unwrap()
            .id;
        let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(37));
    }
}
