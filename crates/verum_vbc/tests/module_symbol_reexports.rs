#![cfg(feature = "codegen")]
//! A module namespace reexport cannot establish a declaration-owned type import.
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    types::TypeRef,
};

#[test]
fn namespace_reexport_does_not_supply_a_child_type_in_either_order() {
    let provider = Parser::new("public type PacketFixture is { value: Int };")
        .parse_module()
        .unwrap();
    let facade = Parser::new("public mount fixture.models;")
        .parse_module()
        .unwrap();
    let client = Parser::new(
        "mount fixture.api.PacketFixture; public fn keep(x: PacketFixture) -> PacketFixture { x }",
    )
    .parse_module()
    .unwrap();
    for reverse in [false, true] {
        let mut sources = vec![
            ("fixture.models", &provider),
            ("fixture.api", &facade),
            ("fixture.client", &client),
        ];
        if reverse {
            sources.reverse();
        }
        let error = VbcCodegen::with_config(CodegenConfig::new("bundle"))
            .collect_owned_declaration_interface(&sources)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unresolved nominal type 'PacketFixture'"),
            "{error}"
        );
    }
}

#[test]
fn explicit_owner_and_symbol_reexport_resolve_the_same_wire_identity() {
    for selected in ["fixture.models", "fixture.api"] {
        let provider = Parser::new("public type PacketFixture is { value: Int };")
            .parse_module()
            .unwrap();
        let facade = Parser::new("public mount fixture.models.PacketFixture;")
            .parse_module()
            .unwrap();
        let client = Parser::new(&format!("mount {selected}.PacketFixture; public fn keep(x: PacketFixture) -> PacketFixture {{ x }}")).parse_module().unwrap();
        for reverse in [false, true] {
            let mut sources = vec![
                ("fixture.models", &provider),
                ("fixture.api", &facade),
                ("fixture.client", &client),
            ];
            if reverse {
                sources.reverse();
            }
            let module = VbcCodegen::with_config(CodegenConfig::new("bundle"))
                .collect_owned_declaration_interface(&sources)
                .unwrap();
            let wire = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap();
            for module in [module, wire] {
                let packet = module
                    .types
                    .iter()
                    .find(|ty| module.get_string(ty.name) == Some("PacketFixture"))
                    .unwrap();
                assert_eq!(
                    packet.origin_module.and_then(|id| module.get_string(id)),
                    Some("fixture.models")
                );
                let function = module
                    .functions
                    .iter()
                    .find(|function| {
                        let owner = function.origin_module.and_then(|id| module.get_string(id));
                        owner == Some("fixture.client")
                            && module.get_string(function.name).is_some_and(|name| {
                                verum_vbc::module::qualify_module_name("fixture.client", name)
                                    == "fixture.client.keep"
                            })
                    })
                    .unwrap();
                assert_eq!(function.params[0].type_ref, TypeRef::Concrete(packet.id));
                assert_eq!(function.return_type, TypeRef::Concrete(packet.id));
            }
        }
    }
}
